//! Glue between the top-level EtherNet/IP [`Client`] and the `pccc` package:
//! the Execute_PCCC service wrapper and PCCC requestor-ID defaults.
//!
//! Port of Go `pccc.go` / `pccc_test.go`. The `WithCIPVendorID` /
//! `WithCIPSerialNumber` options live in `options.rs` (re-exported here).
//!
//! Ref: Allen-Bradley PCCC / DF1 Protocol and Command Set Reference Manual
//! 1770-6.5.16; the CIP PCCC Object (class 0x67) and Execute_PCCC service
//! (0x4B) with its 7-byte requestor ID (length, vendor ID, serial number) per
//! Rockwell/ODVA PCCC Object documentation (section unverified).

use crate::cip;
use crate::client::Client;
use crate::conn::{Error, Result};
use crate::pccc;
use crate::plc::{self, Context};

pub use crate::options::{with_cip_serial_number, with_cip_vendor_id};

// Ref: CIP Vol. 1, Identity Object Vendor ID (UINT) used as the originator
// stamp in the Execute_PCCC requestor ID; an SLC/MicroLogix does not validate
// it (section unverified).
/// Default originator CIP vendor ID written into the Execute_PCCC requestor header.
pub const DEFAULT_CIP_VENDOR_ID: u16 = 0x1234;

// Ref: CIP Vol. 1, Identity Object Serial Number (UDINT) used as the
// originator stamp in the Execute_PCCC requestor ID (section unverified).
/// Default originator CIP serial number written into the Execute_PCCC requestor header.
pub const DEFAULT_CIP_SERIAL_NUMBER: u32 = 0x1234_5678;

// Ref: Execute_PCCC reply echoes the 7-byte requestor ID (1 length byte +
// UINT vendor + UDINT serial) before the PCCC reply (PCCC Object, section unverified).
const REQUESTOR_ID_LEN: usize = 7;

impl Client {
    // Ref: CIP PCCC Object class 0x67 instance 1, Execute_PCCC service 0x4B
    // (section unverified). Errors: transport errors trigger reconnect+retry
    // per client config; CIP-level errors (non-zero general status) are
    // returned as `Error::Cip` and not retried; a PCCC STS error is NOT
    // reported here, but by `pccc::decode_reply`.
    /// Sends a raw PCCC command via the CIP Execute_PCCC service (0x4B) and
    /// returns the raw PCCC reply payload (requestor ID stripped). Low-level
    /// escape hatch; most callers should use [`pccc::Client`].
    pub fn execute_pccc(&self, ctx: &Context, pccc_cmd: &[u8]) -> Result<Vec<u8>> {
        if pccc_cmd.is_empty() {
            return Err(Error::msg("ethernetip: ExecutePCCC: empty PCCC command"));
        }

        let req = cip::new_execute_pccc_request(self.cip_vendor_id, self.cip_serial_number, pccc_cmd);

        let mut reply: Vec<u8> = Vec::new();
        self.do_op(ctx, |sess| {
            let resp = sess.send_cip_request(ctx, &req)?;
            if let Some(e) = resp.error() {
                return Err(Error::Cip(e));
            }
            // Strip the echoed 7-byte requestor ID. What remains is the raw
            // PCCC reply that the device produced.
            match resp.response_data.get(REQUESTOR_ID_LEN..) {
                Some(rest) => {
                    reply = rest.to_vec();
                    Ok(())
                }
                None => Err(Error::msg(format!(
                    "ethernetip: Execute_PCCC reply too short ({} bytes, need >=7 for requestor ID)",
                    resp.response_data.len()
                ))),
            }
        })?;
        Ok(reply)
    }
}

// Implementation detail (not spec): the Go *ethernetip.Client satisfies
// pccc.Executor structurally; in Rust the impl is explicit. Wrap the client in
// an `Arc` to hand to `pccc::Client::new`.
impl pccc::Executor for Client {
    fn execute_pccc(
        &self,
        ctx: &Context,
        pccc_cmd: &[u8],
    ) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        Client::execute_pccc(self, ctx, pccc_cmd).map_err(|e| Box::new(e) as _)
    }

    fn as_connector(&self) -> Option<&dyn pccc::Connector> {
        Some(self)
    }
}

// Implementation detail (not spec): the Go Client also provides Connect /
// Disconnect / IsConnected, so it satisfies pccc.Connector.
impl pccc::Connector for Client {
    fn connect(&self, ctx: &Context) -> plc::Result<()> {
        plc::Plc::connect(self, ctx)
    }
    fn disconnect(&self, ctx: &Context) -> plc::Result<()> {
        plc::Plc::disconnect(self, ctx)
    }
    fn is_connected(&self) -> bool {
        plc::Plc::is_connected(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cip::{CLASS_PCCC, MessageRouter, Object, Path, SERVICE_EXECUTE_PCCC};
    use crate::client::new_client;
    use crate::conn::TcpConn;
    use crate::options::{ClientOption, with_conn};
    use crate::pccc::{REPLY_BIT, decode_reply, encode_typed_read};
    use crate::server::Server;
    use crate::session::Session;
    use crate::transport::Transport;
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    // Transport that always returns a pre-built session (Go `staticTransport`).
    struct StaticTransport {
        sess: Arc<Session>,
    }

    impl Transport<Arc<Session>> for StaticTransport {
        fn conn(&self, _ctx: &Context) -> Result<Arc<Session>> {
            Ok(self.sess.clone())
        }
        fn reset(&self, _stale: &Arc<Session>) -> Result<()> {
            Ok(())
        }
        fn close(&self) -> Result<()> {
            self.sess.close()
        }
        fn peek(&self) -> Option<bool> {
            Some(true)
        }
    }

    // Minimal CIP object handling Execute_PCCC (0x4B): verifies the
    // requestor-ID header layout, records the inner PCCC bytes and replies
    // `requestor ID echo || reply_pccc`.
    struct PcccStub {
        want_vendor: u16,
        want_serial: u32,
        last_pccc: Mutex<Vec<u8>>,
        reply_pccc: Vec<u8>,
        return_cip_error: bool,
    }

    impl Object for PcccStub {
        fn handle_request(&self, service: u8, _path: &Path, data: &[u8]) -> cip::Result<Vec<u8>> {
            assert_eq!(service, SERVICE_EXECUTE_PCCC, "unexpected service");
            assert!(data.len() >= 7, "Execute_PCCC payload too short: {} bytes", data.len());
            assert_eq!(data[0], 0x07, "requestor-ID length");
            let vendor = u16::from_le_bytes([data[1], data[2]]);
            let serial = u32::from_le_bytes([data[3], data[4], data[5], data[6]]);
            assert_eq!(vendor, self.want_vendor, "vendor");
            assert_eq!(serial, self.want_serial, "serial");
            *self.last_pccc.lock().unwrap() = data[7..].to_vec();

            if self.return_cip_error {
                return Err(cip::Error::new(cip::STATUS_SERVICE_NOT_SUPPORTED).into());
            }
            let mut out = data[..7].to_vec();
            out.extend_from_slice(&self.reply_pccc);
            Ok(out)
        }
    }

    struct ShortReply;
    impl Object for ShortReply {
        fn handle_request(&self, _: u8, _: &Path, _: &[u8]) -> cip::Result<Vec<u8>> {
            Ok(vec![0x01, 0x02, 0x03])
        }
    }

    // Loopback replacement for net.Pipe(): (server side, client side).
    fn pipe() -> (TcpStream, TcpStream) {
        let ln = TcpListener::bind("127.0.0.1:0").unwrap();
        let c = TcpStream::connect(ln.local_addr().unwrap()).unwrap();
        let (s, _) = ln.accept().unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        (s, c)
    }

    // Client whose transport is backed by an in-process EtherNet/IP server
    // with the given PCCC object registered (Go `newPCCCStubClient`).
    fn new_pccc_stub_client(obj: Arc<dyn Object>, opts: &[ClientOption]) -> (Client, Server) {
        let router = Arc::new(MessageRouter::new());
        router.register_object(CLASS_PCCC, obj);
        let (s, c) = pipe();
        let srv = Server::builder(router).conn(s).build();
        srv.start(&Context::background(), "").unwrap();

        let tc = TcpConn::new("", &[with_conn(c)]).unwrap();
        let sess = Arc::new(Session::new(tc, None));
        sess.register(&Context::background()).unwrap();
        let client = new_client(Arc::new(StaticTransport { sess }), opts);
        (client, srv)
    }

    fn stub(vendor: u16, serial: u32, reply: Vec<u8>, err: bool) -> Arc<PcccStub> {
        Arc::new(PcccStub {
            want_vendor: vendor,
            want_serial: serial,
            last_pccc: Mutex::new(vec![]),
            reply_pccc: reply,
            return_cip_error: err,
        })
    }

    // Encode a typed-logical read, send via execute_pccc, confirm the PCCC
    // bytes are propagated back unchanged (requestor ID stripped) and the
    // inner request bytes match what was sent.
    #[test]
    fn execute_pccc_read_round_trip() {
        let pccc_req = encode_typed_read(0x1234, 2, 7, pccc::FileType::INTEGER, 0, 0).unwrap();
        let st = stub(
            0x1234,
            0xCAFE_F00D,
            vec![pccc_req[0] | REPLY_BIT, 0x00, 0x34, 0x12, 0x2A, 0x00],
            false,
        );
        let (c, _srv) = new_pccc_stub_client(
            st.clone(),
            &[with_cip_vendor_id(0x1234), with_cip_serial_number(0xCAFE_F00D)],
        );

        let got = c.execute_pccc(&Context::background(), &pccc_req).unwrap();
        assert_eq!(got, st.reply_pccc, "returned PCCC bytes mismatch");
        assert_eq!(*st.last_pccc.lock().unwrap(), pccc_req, "PCCC bytes received by stub mismatch");
        let _ = c.close();
    }

    // decode_reply on a STS-error reply surfaces as a PCCC status error, and
    // execute_pccc itself does NOT report a transport-layer failure.
    #[test]
    fn execute_pccc_sts_error_surfaces() {
        let pccc_req = encode_typed_read(0x0001, 2, 7, pccc::FileType::INTEGER, 0, 0).unwrap();
        let st = stub(
            DEFAULT_CIP_VENDOR_ID,
            DEFAULT_CIP_SERIAL_NUMBER,
            vec![pccc_req[0] | REPLY_BIT, 0x10, 0x01, 0x00],
            false,
        );
        let (c, _srv) = new_pccc_stub_client(st, &[]);

        let raw = c.execute_pccc(&Context::background(), &pccc_req).expect("ExecutePCCC at CIP layer");
        let err = decode_reply(&raw).expect_err("expected PCCC STS error");
        let pccc::PcccError::Status(pe) = err else { panic!("expected pccc status error") };
        assert_eq!(pe.sts, 0x10);
        let _ = c.close();
    }

    // A non-zero CIP general status propagates as a cip error from
    // execute_pccc and is not retried.
    #[test]
    fn execute_pccc_cip_error_surfaces() {
        let pccc_req = encode_typed_read(0x0001, 2, 7, pccc::FileType::INTEGER, 0, 0).unwrap();
        let st = stub(DEFAULT_CIP_VENDOR_ID, DEFAULT_CIP_SERIAL_NUMBER, vec![], true);
        let (c, _srv) = new_pccc_stub_client(st, &[]);

        let err = c.execute_pccc(&Context::background(), &pccc_req).expect_err("expected CIP error");
        let Error::Cip(ce) = err else { panic!("expected cip error, got {err}") };
        assert_eq!(ce.status, cip::STATUS_SERVICE_NOT_SUPPORTED);
        let _ = c.close();
    }

    // Without with_cip_vendor_id / with_cip_serial_number the client uses the documented defaults.
    #[test]
    fn execute_pccc_default_requestor_id() {
        let pccc_req = encode_typed_read(0x0001, 2, 7, pccc::FileType::INTEGER, 0, 0).unwrap();
        let st = stub(
            DEFAULT_CIP_VENDOR_ID,
            DEFAULT_CIP_SERIAL_NUMBER,
            vec![pccc_req[0] | REPLY_BIT, 0x00, 0x01, 0x00, 0x00, 0x00],
            false,
        );
        let (c, _srv) = new_pccc_stub_client(st, &[]);
        c.execute_pccc(&Context::background(), &pccc_req).unwrap();
        let _ = c.close();
    }

    // A reply too short to contain the 7-byte requestor-ID echo errors cleanly.
    #[test]
    fn execute_pccc_rejects_truncated_requestor_id() {
        let pccc_req = encode_typed_read(0x0001, 2, 7, pccc::FileType::INTEGER, 0, 0).unwrap();
        let (c, _srv) = new_pccc_stub_client(Arc::new(ShortReply), &[]);
        assert!(c.execute_pccc(&Context::background(), &pccc_req).is_err());
        let _ = c.close();
    }

    #[test]
    fn execute_pccc_rejects_empty_command() {
        let st = stub(0, 0, vec![], false);
        let (c, _srv) = new_pccc_stub_client(st, &[]);
        let e = c.execute_pccc(&Context::background(), &[]).unwrap_err();
        assert_eq!(e.to_string(), "ethernetip: ExecutePCCC: empty PCCC command");
    }

    // End to end: pccc::Client over the EtherNet/IP Client executor.
    #[test]
    fn pccc_client_over_ethernetip_client() {
        // Reply depends on the request TNS, so use a custom object.
        struct Echo;
        impl Object for Echo {
            fn handle_request(&self, _: u8, _: &Path, d: &[u8]) -> cip::Result<Vec<u8>> {
                let p = &d[7..];
                let mut out = d[..7].to_vec();
                out.extend_from_slice(&[p[0] | REPLY_BIT, 0x00, p[2], p[3], 0x2A, 0x00]);
                Ok(out)
            }
        }
        let (c, _srv) = new_pccc_stub_client(Arc::new(Echo), &[]);
        let pc = pccc::Client::new(Arc::new(c));
        let v = pc.read_address(&Context::background(), "N7:0").unwrap();
        assert_eq!(v.int().unwrap(), 42);
    }
}
