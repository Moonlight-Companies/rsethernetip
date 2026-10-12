//! High-level EtherNet/IP client: read/write tags by name, enumerate tags,
//! device identity, with optional automatic reconnection and retries.
//!
//! Port of Go `client.go`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::cip::{self, Marshaler, SizedCip, Unmarshaler};
use crate::conn::{Error, Result};
use crate::connector::{SessionCloser, SessionConnector};
use crate::datapoint::Tag;
use crate::eip;
use crate::logging::Logger;
use crate::options::{split_opts, ClientOption, ClientSettings, Opt};
use crate::plc::{self, ByteOrder, Context, DataPoint, Value};
use crate::session::Session;
use crate::transport::{DirectTransport, ReconnectingTransport, Transport};

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Read Tag (0x4C) / Write Tag (0x4D) services
// addressed by symbolic tag path; CIP Vol. 2 explicit messaging over SendRRData.
// Implementation detail: sessions come from a pluggable transport; the client implements
// `plc::Plc`. Use [`connect`] for a direct connection, [`new_reconnecting_client`] for automatic
// reconnection, or [`new_client`] with a custom transport.
/// A high-level EtherNet/IP client that reads and writes PLC tags over TCP using explicit CIP
/// messaging.
pub struct Client {
    transport: Arc<dyn Transport<Arc<Session>>>,
    pub(crate) logger: Arc<dyn Logger>,
    pub(crate) retries: i32,
    pub(crate) retry_delay: Duration,

    // Ref: PCCC requestor-ID fields (Allen-Bradley PCCC / DF1 protocol manual 1770-6.5.16;
    // CIP Execute_PCCC 0x4B requestor ID layout, section unverified). Used as the originator
    // stamp on Execute_PCCC. Default to the Go `DefaultCIP*` values unless overridden.
    #[allow(dead_code)] // read by pccc_glue's Execute_PCCC
    pub(crate) cip_vendor_id: u16,
    #[allow(dead_code)]
    pub(crate) cip_serial_number: u32,
}

// Implementation detail (not spec): Go `NewClient`.
/// Creates a new [`Client`] backed by the given transport. Apply [`ClientOption`] values to
/// configure retries, logging and other behavior.
pub fn new_client(t: Arc<dyn Transport<Arc<Session>>>, opts: &[ClientOption]) -> Client {
    let mut st = ClientSettings::default();
    for o in opts {
        st.apply(o);
    }
    Client::from_parts(t, st)
}

// Implementation detail (not spec): Go package-level `Connect`.
/// Dials `address` over TCP, registers an EtherNet/IP session, and returns a ready-to-use
/// [`Client`]. The connection is direct (non-reconnecting); if it drops, operations fail until a
/// new Client is created. Conn, client and transport options are routed to their layers.
pub fn connect(ctx: &Context, address: &str, opts: Vec<Opt>) -> Result<Client> {
    let (conn_opts, client_opts, transport_opts) = split_opts(opts);

    let mut st = ClientSettings::default();
    for o in &client_opts {
        st.apply(o);
    }

    let connector = SessionConnector::new(address, Some(st.logger.clone()), conn_opts);
    let dt = DirectTransport::new(ctx, Arc::new(connector), Arc::new(SessionCloser), &transport_opts)?;

    Ok(Client::from_parts(Arc::new(dt), st))
}

// Implementation detail (not spec): Go `NewReconnectingClient`.
/// Creates a [`Client`] that connects lazily on the first operation and automatically
/// reconnects after a transport failure. Never dials in the constructor.
pub fn new_reconnecting_client(address: &str, opts: Vec<Opt>) -> Client {
    let (conn_opts, client_opts, transport_opts) = split_opts(opts);

    let mut st = ClientSettings::default();
    for o in &client_opts {
        st.apply(o);
    }

    let connector = SessionConnector::new(address, Some(st.logger.clone()), conn_opts);
    let rt = ReconnectingTransport::new(Arc::new(connector), Arc::new(SessionCloser), &transport_opts);

    Client::from_parts(Arc::new(rt), st)
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Read Tag response layout: 2-byte CIP data
// type code (4 bytes for structure types: 0x02A0 + 2-byte structure handle) followed by the data.
// Implementation detail: strips that header from `data`; mirrors the Go length checks.
fn strip_type_header(data: &[u8]) -> Result<&[u8]> {
    if data.len() < 2 {
        return Err(Error::msg("response too short to contain type code"));
    }
    let type_code = cip::DataType(u16::from_le_bytes([data[0], data[1]]));
    let hdr_len = if type_code >= cip::TYPE_STRUCT { 4 } else { 2 };
    if data.len() < hdr_len {
        return Err(Error::msg("response too short for type header"));
    }
    Ok(&data[hdr_len..])
}

// Ref: CIP Vol. 1, Appendix C data type codes (section unverified) mapped to the protocol-agnostic
// plc::DataType. Implementation detail: SINT/USINT are 8-bit but mapped to the closest 16-bit hint
// (as in Go).
/// Maps CIP data type codes to protocol-agnostic [`plc::DataType`].
fn cip_type_to_plc_type(dt: cip::DataType) -> plc::DataType {
    match dt {
        cip::TYPE_BOOL => plc::DataType::Bool,
        cip::TYPE_SINT => plc::DataType::Int16, // SINT is 8-bit but closest hint
        cip::TYPE_INT => plc::DataType::Int16,
        cip::TYPE_DINT => plc::DataType::Int32,
        cip::TYPE_LINT => plc::DataType::Int64,
        cip::TYPE_USINT => plc::DataType::Uint16, // USINT is 8-bit but closest hint
        cip::TYPE_UINT => plc::DataType::Uint16,
        cip::TYPE_UDINT => plc::DataType::Uint32,
        cip::TYPE_ULINT => plc::DataType::Uint64,
        cip::TYPE_REAL => plc::DataType::Float32,
        cip::TYPE_LREAL => plc::DataType::Float64,
        cip::TYPE_STRING | cip::TYPE_STRING2 | cip::TYPE_SHORT_STRING => plc::DataType::String,
        _ => plc::DataType::Bytes,
    }
}

impl Client {
    fn from_parts(transport: Arc<dyn Transport<Arc<Session>>>, st: ClientSettings) -> Client {
        Client {
            transport,
            logger: st.logger,
            retries: st.retries,
            retry_delay: st.retry_delay,
            cip_vendor_id: st.cip_vendor_id,
            cip_serial_number: st.cip_serial_number,
        }
    }

    /// Closes the underlying transport and releases all resources.
    pub fn close(&self) -> Result<()> {
        self.transport.close()
    }

    // ---------- Tag-level API ----------

    // Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Read Tag service (0x4C).
    /// Reads a single element of a named tag using the CIP Read Tag service (0x4C). The returned
    /// bytes include the 2-byte CIP type code prefix followed by the element data.
    pub fn read_tag(&self, ctx: &Context, tag_name: &str) -> Result<Vec<u8>> {
        self.read_tag_elements(ctx, tag_name, 1)
    }

    // Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Read Tag service (0x4C), Number of
    // Elements field (UINT).
    /// Reads `count` elements of a named tag (CIP Read Tag, 0x4C). The returned bytes include the
    /// 2-byte CIP type code prefix followed by the element data. `count` must be at least 1.
    pub fn read_tag_elements(&self, ctx: &Context, tag_name: &str, count: u16) -> Result<Vec<u8>> {
        if count == 0 {
            return Err(Error::msg("element count must be at least 1"));
        }

        let p = cip::parse_tag_path(tag_name)?;
        let req = cip::new_read_tag_request(p, count);

        self.do_op(ctx, |sess| {
            let resp = sess.send_cip_request(ctx, &req)?;
            if let Some(e) = resp.error() {
                return Err(Error::Cip(e));
            }
            Ok(resp.response_data)
        })
    }

    // Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Write Tag service (0x4D): data type code
    // (UINT), element count (UINT), data. The CIP type code is inferred from the Rust type.
    /// Writes a value to a named tag using the CIP Write Tag service (0x4D). The value must be a
    /// basic numeric type (bool, i32, f64, ...) or a registered `TypeCodec`.
    pub fn write_tag<T: Marshaler + cip::CipTyped + ?Sized>(&self, ctx: &Context, tag_name: &str, value: &T) -> Result<()> {
        let p = cip::parse_tag_path(tag_name)?;

        let data_type = cip::go_type_to_cip_type(value)?;

        let data = cip::marshal(value)?;

        let req = cip::new_write_tag_request(p, data_type, 1, &data);

        self.do_op(ctx, |sess| {
            let resp = sess.send_cip_request(ctx, &req)?;
            if let Some(e) = resp.error() {
                return Err(Error::Cip(e));
            }
            Ok(())
        })
    }

    /// Reads a single element of a named tag and unmarshals it into `dst`.
    pub fn read_tag_into<T: Unmarshaler + ?Sized>(&self, ctx: &Context, tag_name: &str, dst: &mut T) -> Result<()> {
        self.read_tag_elements_into(ctx, tag_name, 1, dst)
    }

    /// Reads `count` elements of a named tag and unmarshals them into `dst` (e.g. a slice
    /// pre-sized to `count`).
    pub fn read_tag_elements_into<T: Unmarshaler + ?Sized>(
        &self,
        ctx: &Context,
        tag_name: &str,
        count: u16,
        dst: &mut T,
    ) -> Result<()> {
        let data = self.read_tag_elements(ctx, tag_name, count)?;
        let payload = strip_type_header(&data)?;
        Ok(cip::unmarshal(payload, dst)?)
    }

    // Ref: Rockwell Logix 5000 General Instructions Reference Manual (1756-RM003) TIMER structure;
    // Rockwell 1756-PM020 Logix 5000 Data Access for structure reads.
    /// Reads a Rockwell Logix Timer tag (TON, TOF, RTO) and decodes it. The tag may use
    /// program-scope and member syntax accepted by [`cip::parse_tag_path`].
    pub fn read_timer(&self, ctx: &Context, tag_name: &str) -> Result<cip::Timer> {
        let payload = self.read_struct_payload(ctx, tag_name)?;
        Ok(cip::decode_timer(&payload)?)
    }

    // Ref: Rockwell 1756-RM003 COUNTER structure; 1756-PM020 Logix 5000 Data Access.
    /// Reads a Rockwell Logix Counter tag (CTU, CTD, CTUD) and decodes it.
    pub fn read_counter(&self, ctx: &Context, tag_name: &str) -> Result<cip::Counter> {
        let payload = self.read_struct_payload(ctx, tag_name)?;
        Ok(cip::decode_counter(&payload)?)
    }

    // Implementation detail: reads a tag and strips the CIP type-code header, returning just the
    // structure payload bytes.
    fn read_struct_payload(&self, ctx: &Context, tag_name: &str) -> Result<Vec<u8>> {
        let data = self.read_tag(ctx, tag_name)?;
        Ok(strip_type_header(&data)?.to_vec())
    }

    // ---------- Discovery / Enumeration ----------

    // Ref: CIP Vol. 2, Section 2-4 ListIdentity (0x63) (section number unverified).
    /// Sends the EIP ListIdentity command via the current session and returns the device
    /// identity items reported by the target.
    pub fn list_identity(&self, ctx: &Context) -> Result<Vec<eip::ListIdentityItem>> {
        self.do_op(ctx, |sess| sess.list_identity(ctx))
    }

    // Ref: CIP Vol. 2, Section 2-4 ListServices (0x04) (section number unverified).
    /// Sends the EIP ListServices command and returns the communication services supported.
    pub fn list_services(&self, ctx: &Context) -> Result<Vec<eip::ListServicesItem>> {
        self.do_op(ctx, |sess| sess.list_services(ctx))
    }

    // Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Symbol Object (class 0x6B): class
    // attribute query for the max instance, then Get Attributes per instance.
    /// Enumerates all tags on the PLC by querying every instance of the CIP Symbol Object (class
    /// 0x6B). The entire enumeration runs inside a single retry scope so a mid-enumeration
    /// connection drop retries from scratch.
    pub fn list_tags(&self, ctx: &Context) -> Result<Vec<cip::SymbolInstance>> {
        self.do_op(ctx, |sess| {
            let req_class = cip::new_get_symbol_class_attributes_request();
            let resp_class = sess.send_cip_request(ctx, &req_class)?;
            if !resp_class.is_success() {
                // is_success() is false so error() is always Some.
                return Err(match resp_class.error() {
                    Some(e) => Error::Cip(e),
                    None => Error::msg("symbol class attributes request failed"),
                });
            }

            let (_, max_instance) = cip::decode_symbol_class_attributes_response(&resp_class.response_data)
                .map_err(|e| Error::wrap("failed to decode symbol class attributes", e.into()))?;

            self.logger.info(format_args!("Max Symbol Instance: {}", max_instance));

            let mut all_symbols = Vec::new();

            for id in 1..=(max_instance as u32) {
                if let Some(e) = ctx.err() {
                    return Err(Error::Plc(e));
                }
                let req = cip::new_get_symbol_attributes_request(id);
                let resp = match sess.send_cip_request(ctx, &req) {
                    Ok(r) => r,
                    // Deviation from Go: transport failures abort so do_op resets the session.
                    Err(e) if !matches!(e, Error::CipLayer(_) | Error::Cip(_)) => return Err(e),
                    Err(e) => {
                        self.logger.warn(format_args!("Failed to fetch attributes for instance {}: {}", id, e));
                        continue;
                    }
                };

                if !resp.is_success() {
                    // Go distinguishes ObjectDoesNotExist / PathDestinationUnknown here but
                    // skips the instance in every failure case.
                    continue;
                }

                let (name, type_code) = match cip::decode_symbol_attributes_response(&resp.response_data) {
                    Ok(v) => v,
                    Err(e) => {
                        self.logger.warn(format_args!("Failed to decode attributes for instance {}: {}", id, e));
                        continue;
                    }
                };

                if !name.is_empty() {
                    all_symbols.push(cip::SymbolInstance { instance_id: id, name, type_: type_code });
                }
            }

            Ok(all_symbols)
        })
    }

    // ---------- Internal retry loop ----------

    // Implementation detail (not spec): Go `Client.do`. Executes `op` with retry logic. On
    // transport errors the session is reset and the operation retried. CIP-level errors
    // ([`Error::Cip`], Go `cipError`) are not retried and are returned as-is.
    //
    // Go quirk ported as-is: with `retries == 0` an operation (not connection) failure falls out
    // of the loop and reports "operation failed after 1 retries" instead of the cause.
    // Deviation: unlike Go, a cancelled/expired `ctx` aborts the retry loop with the context error
    // (Go loops forever with infinite retries) and the retry delay sleeps in short slices that
    // observe `ctx`.
    pub(crate) fn do_op<T>(&self, ctx: &Context, mut op: impl FnMut(&Arc<Session>) -> Result<T>) -> Result<T> {
        let mut i: i32 = 0;
        while self.retries < 0 || i <= self.retries {
            let sess = match self.transport.conn(ctx) {
                Ok(s) => s,
                Err(err) => {
                    if self.retries == 0 {
                        return Err(err);
                    }
                    self.logger.warn(format_args!("Session unavailable (attempt {}): {}", i.wrapping_add(1), err));
                    if self.retries < 0 || i < self.retries {
                        self.sleep_retry(ctx)?;
                    }
                    i = i.wrapping_add(1);
                    continue;
                }
            };

            let err = match op(&sess) {
                Ok(v) => return Ok(v),
                Err(e) => e,
            };

            // CIP errors are not retryable.
            if matches!(err, Error::Cip(_)) {
                return Err(err);
            }

            // Transport error: reset and retry.
            let _ = self.transport.reset(&sess);
            self.logger.warn(format_args!("Operation failed (attempt {}): {}", i.wrapping_add(1), err));

            if self.retries < 0 || i < self.retries {
                self.sleep_retry(ctx)?;
            }

            // Prevent integer overflow for infinite retries.
            if i == i32::MAX {
                i = 0;
            }
            i = i.wrapping_add(1);
        }

        Err(Error::msg(format!("operation failed after {} retries", self.retries as i64 + 1)))
    }

    // Implementation detail: sleeps `retry_delay` in slices, returning the context error if `ctx`
    // is cancelled or expires meanwhile.
    fn sleep_retry(&self, ctx: &Context) -> Result<()> {
        let end = Instant::now() + self.retry_delay;
        loop {
            if let Some(e) = ctx.err() {
                return Err(Error::Plc(e));
            }
            let now = Instant::now();
            if now >= end {
                return Ok(());
            }
            std::thread::sleep((end - now).min(Duration::from_millis(20)));
        }
    }
}

// ---------- plc::Plc interface ----------

// Implementation detail (not spec): Go `Client.Read`/`Write`/`Connect`/`Disconnect`/`IsConnected`.
impl plc::Reader for Client {
    // Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Read Tag response: 2-byte type code
    // (4 bytes for STRUCT) precedes the data; the prefix is stripped so `raw` holds only the
    // payload and the type hint carries the CIP type.
    /// Reads one or more data points. Each must be a [`Tag`]. Returned values contain the raw data
    /// bytes (type-code prefix stripped) and a protocol-agnostic type hint.
    fn read(&self, ctx: &Context, points: &[Arc<dyn DataPoint>]) -> plc::Result<Vec<Value>> {
        let mut values = Vec::with_capacity(points.len());
        for dp in points {
            let tag = match dp.as_any().and_then(|a| a.downcast_ref::<Tag>()) {
                Some(t) => t,
                None => {
                    return Err(Error::msg(format!("ethernetip: expected Tag DataPoint, got {:?}", dp)).into());
                }
            };
            let elements = if tag.elements == 0 { 1 } else { tag.elements };
            let mut raw = self.read_tag_elements(ctx, &tag.name, elements)?;

            let mut val = Value {
                data_point: Some(dp.clone()),
                byte_order: ByteOrder::LittleEndian,
                ..Value::default()
            };

            // The first 2 bytes of a CIP ReadTag response are the type code (4 bytes for
            // struct types). Extract the type hint and strip the prefix.
            if raw.len() >= 2 {
                let cip_type = cip::DataType(u16::from_le_bytes([raw[0], raw[1]]));
                val.data_type = cip_type_to_plc_type(cip_type);
                let hdr_len = if cip_type >= cip::TYPE_STRUCT { 4 } else { 2 };
                if raw.len() >= hdr_len {
                    raw.drain(..hdr_len);
                }
            }
            val.raw = raw;

            values.push(val);
        }
        Ok(values)
    }
}

impl plc::Writer for Client {
    // Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Write Tag service (0x4D).
    /// Writes raw bytes to a tag. `data` must begin with a 2-byte CIP type code followed by the
    /// payload bytes.
    fn write(&self, ctx: &Context, point: &dyn DataPoint, data: &[u8]) -> plc::Result<()> {
        let tag = match point.as_any().and_then(|a| a.downcast_ref::<Tag>()) {
            Some(t) => t,
            None => {
                return Err(Error::msg(format!("ethernetip: expected Tag DataPoint, got {:?}", point)).into());
            }
        };

        // We need at least 2 bytes for the type code in the raw data.
        if data.len() < 2 {
            return Err(Error::msg("ethernetip: write data must include 2-byte type code prefix").into());
        }

        let data_type = cip::DataType(u16::from_le_bytes([data[0], data[1]]));
        let elements = if tag.elements == 0 { 1 } else { tag.elements };

        let p = cip::parse_tag_path(&tag.name).map_err(Error::from)?;
        let req = cip::new_write_tag_request(p, data_type, elements, &data[2..]);

        Ok(self.do_op(ctx, |sess| {
            let resp = sess.send_cip_request(ctx, &req)?;
            if let Some(e) = resp.error() {
                return Err(Error::Cip(e));
            }
            Ok(())
        })?)
    }
}

impl plc::Plc for Client {
    /// Establishes the underlying TCP connection and EtherNet/IP session. For a direct
    /// transport this is a no-op (already connected); a reconnecting transport dials here.
    fn connect(&self, ctx: &Context) -> plc::Result<()> {
        self.transport.conn(ctx)?;
        Ok(())
    }

    /// Unregisters the EtherNet/IP session and closes the TCP connection.
    fn disconnect(&self, _ctx: &Context) -> plc::Result<()> {
        Ok(self.transport.close()?)
    }

    /// True if a session is currently available. Never dials a new connection when the
    /// transport supports peeking.
    fn is_connected(&self) -> bool {
        match self.transport.peek() {
            Some(b) => b,
            // Fallback for custom transports that don't implement peek.
            None => self.transport.conn(&Context::background()).is_ok(),
        }
    }
}

// ---------- Generic helpers ----------

// Implementation detail (not spec): Go generic `Read[T]`.
/// Reads a single tag value as `T` (a fixed-size type or a [`Unmarshaler`]).
pub fn read_typed<T: Unmarshaler + Default>(c: &Client, ctx: &Context, tag_name: &str) -> Result<T> {
    let data = c.read_tag(ctx, tag_name)?;
    let payload = strip_type_header(&data)?;
    let mut result = T::default();
    cip::unmarshal(payload, &mut result)?;
    Ok(result)
}

// Implementation detail (not spec): Go generic `ReadSlice[T]`.
/// Reads `count` elements of a tag and returns them as `Vec<T>`.
pub fn read_slice<T: Unmarshaler + SizedCip + Default + Clone>(c: &Client, ctx: &Context, tag_name: &str, count: u16) -> Result<Vec<T>> {
    let data = c.read_tag_elements(ctx, tag_name, count)?;
    let payload = strip_type_header(&data)?;
    let mut result = vec![T::default(); count as usize];
    cip::unmarshal(payload, &mut result)?;
    Ok(result)
}

// Implementation detail: ported tests from Go ethernetip_test.go (client side) against the
// in-test fake target, plus retry/reconnect behavior tests.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cip::{MessageRouter, TYPE_BOOL, TYPE_DINT, TYPE_REAL};
    use crate::options::{with_conn, with_retries, with_retry_delay};
    use crate::plc::{Plc, Reader, Writer};
    use crate::session::fixture::{FakeTarget, MockObject};
    use std::net::TcpStream;
    use std::sync::atomic::Ordering;

    fn ctx() -> Context {
        Context::background().with_timeout(Duration::from_secs(5))
    }

    fn symbolic_router(response: Vec<u8>) -> Arc<MessageRouter> {
        let router = Arc::new(MessageRouter::new());
        router.register_symbolic_handler(MockObject::new(move |_, _, _| Ok(response.clone())));
        router
    }

    // Go `setupPipePair` + `Connect(ctx, "", WithConn(clientConn))`.
    fn connect_pair(router: Arc<MessageRouter>) -> (FakeTarget, Client) {
        let t = FakeTarget::start(router);
        let s = TcpStream::connect(&t.addr).unwrap();
        let c = connect(&ctx(), "", vec![with_conn(s).into()]).unwrap();
        (t, c)
    }

    fn tag_points(name: &str) -> Vec<Arc<dyn DataPoint>> {
        vec![Arc::new(Tag { name: name.into(), elements: 0 })]
    }

    // Go TestClientReadStripsTypePrefix - table cases
    #[test]
    fn client_read_strips_type_prefix() {
        let real = 3.14f32.to_le_bytes().to_vec();
        let cases: Vec<(&str, cip::DataType, Vec<u8>, plc::DataType)> = vec![
            ("BOOL false", TYPE_BOOL, vec![0x00], plc::DataType::Bool),
            ("BOOL true", TYPE_BOOL, vec![0x01], plc::DataType::Bool),
            ("DINT 42", TYPE_DINT, vec![0x2A, 0, 0, 0], plc::DataType::Int32),
            ("REAL 3.14", TYPE_REAL, real, plc::DataType::Float32),
        ];
        for (name, cip_type, payload, want_type) in cases {
            let mut response = cip_type.0.to_le_bytes().to_vec();
            response.extend_from_slice(&payload);

            let (_t, client) = connect_pair(symbolic_router(response));
            let values = client.read(&ctx(), &tag_points("TestTag")).unwrap();
            assert_eq!(values.len(), 1, "{name}");
            assert_eq!(values[0].data_type, want_type, "{name}: Type");
            assert_eq!(values[0].raw, payload, "{name}: type prefix should be stripped");
        }
    }

    // Go TestClientReadStripsTypePrefix/"Bool false helper"
    #[test]
    fn client_read_bool_false_helper() {
        let (_t, client) = connect_pair(symbolic_router(vec![0xC1, 0x00, 0x00]));
        let values = client.read(&ctx(), &tag_points("BoolTag")).unwrap();
        assert!(!values[0].bool(), "Bool() = true for BOOL false; type prefix was not stripped");
    }

    // Go TestClientReadStripsTypePrefix/"DINT Int helper"
    #[test]
    fn client_read_dint_int_helper() {
        let (_t, client) = connect_pair(symbolic_router(vec![0xC4, 0x00, 0x2A, 0x00, 0x00, 0x00]));
        let values = client.read(&ctx(), &tag_points("IntTag")).unwrap();
        assert_eq!(values[0].int().unwrap(), 42);
    }

    // Go TestClientReadStripsTypePrefix/"REAL Float32 helper"
    #[test]
    fn client_read_real_float32_helper() {
        let mut response = vec![0xCA, 0x00];
        response.extend_from_slice(&3.14f32.to_le_bytes());
        let (_t, client) = connect_pair(symbolic_router(response));
        let values = client.read(&ctx(), &tag_points("RealTag")).unwrap();
        assert_eq!(values[0].float32().unwrap(), 3.14);
    }

    #[test]
    fn read_struct_type_prefix_is_four_bytes() {
        // STRUCT (0x02A0) + 2-byte structure handle + payload.
        let (_t, client) = connect_pair(symbolic_router(vec![0xA0, 0x02, 0x34, 0x12, 0xAA, 0xBB]));
        let values = client.read(&ctx(), &tag_points("S")).unwrap();
        assert_eq!(values[0].raw, vec![0xAA, 0xBB]);
        assert_eq!(values[0].data_type, plc::DataType::Bytes);
    }

    #[test]
    fn read_rejects_non_tag_data_point() {
        #[derive(Debug)]
        struct Other;
        impl DataPoint for Other {
            fn describe(&self) -> String {
                "other".into()
            }
        }
        let (_t, client) = connect_pair(symbolic_router(vec![0xC1, 0, 0]));
        let pts: Vec<Arc<dyn DataPoint>> = vec![Arc::new(Other)];
        let e = client.read(&ctx(), &pts).unwrap_err();
        assert!(e.to_string().starts_with("ethernetip: expected Tag DataPoint, got "), "{e}");
        let e = client.write(&ctx(), &Other, &[0, 0, 0]).unwrap_err();
        assert!(e.to_string().starts_with("ethernetip: expected Tag DataPoint, got "), "{e}");
    }

    #[test]
    fn write_validates_prefix_and_sends_tag() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let s2 = seen.clone();
        let router = Arc::new(MessageRouter::new());
        router.register_symbolic_handler(MockObject::new(move |svc, _, data| {
            assert_eq!(svc, cip::SERVICE_WRITE_TAG);
            *s2.lock().unwrap() = data.to_vec();
            Ok(vec![])
        }));
        let (_t, client) = connect_pair(router);

        let e = client.write(&ctx(), &Tag::new("T"), &[0xC4]).unwrap_err();
        assert_eq!(e.to_string(), "ethernetip: write data must include 2-byte type code prefix");

        // type DINT + value 7
        client.write(&ctx(), &Tag::new("T"), &[0xC4, 0x00, 7, 0, 0, 0]).unwrap();
        // Write Tag data: type(2) elements(2) value(4)
        assert_eq!(*seen.lock().unwrap(), vec![0xC4, 0x00, 0x01, 0x00, 7, 0, 0, 0]);

        client.write_tag(&ctx(), "T", &9i32).unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![0xC4, 0x00, 0x01, 0x00, 9, 0, 0, 0]);
    }

    #[test]
    fn read_tag_zero_count_and_bad_path() {
        let (_t, client) = connect_pair(symbolic_router(vec![0xC1, 0, 0]));
        let e = client.read_tag_elements(&ctx(), "T", 0).unwrap_err();
        assert_eq!(e.to_string(), "element count must be at least 1");
        assert!(client.read_tag(&ctx(), "").is_err());
    }

    #[test]
    fn typed_helpers_and_short_responses() {
        let (_t, client) = connect_pair(symbolic_router(vec![0xC4, 0x00, 0x2A, 0, 0, 0, 0x2B, 0, 0, 0]));
        let v: i32 = read_typed(&client, &ctx(), "T").unwrap();
        assert_eq!(v, 42);
        let vs: Vec<i32> = read_slice(&client, &ctx(), "T", 2).unwrap();
        assert_eq!(vs, vec![42, 43]);
        let mut dst = 0i32;
        client.read_tag_into(&ctx(), "T", &mut dst).unwrap();
        assert_eq!(dst, 42);

        let (_t2, short) = connect_pair(symbolic_router(vec![0xC4]));
        let e = read_typed::<i32>(&short, &ctx(), "T").unwrap_err();
        assert_eq!(e.to_string(), "response too short to contain type code");
        let (_t3, short) = connect_pair(symbolic_router(vec![0xA0, 0x02, 0x00]));
        let e = read_typed::<i32>(&short, &ctx(), "T").unwrap_err();
        assert_eq!(e.to_string(), "response too short for type header");
    }

    #[test]
    fn read_timer_and_counter() {
        // STRUCT header (4 bytes) + 14-byte timer: reserved, status(EN|DN), PRE=1000, ACC=250.
        let mut r = vec![0xA0, 0x02, 0x00, 0x00];
        r.extend_from_slice(&[0, 0]);
        r.extend_from_slice(&((1u32 << 31) | (1 << 29)).to_le_bytes());
        r.extend_from_slice(&1000i32.to_le_bytes());
        r.extend_from_slice(&250i32.to_le_bytes());
        let (_t, client) = connect_pair(symbolic_router(r));
        let t = client.read_timer(&ctx(), "Program:Main.T1").unwrap();
        assert_eq!((t.pre, t.acc, t.en, t.dn, t.tt), (1000, 250, true, true, false));
        assert!(client.read_counter(&ctx(), "C1").is_ok());
    }

    #[test]
    fn cip_error_is_returned_unretried() {
        let router = Arc::new(MessageRouter::new());
        router.register_symbolic_handler(MockObject::new(|_, _, _| Err(cip::Error::new(cip::STATUS_PATH_DESTINATION_UNKNOWN).into())));
        let t = FakeTarget::start(router);
        let s = TcpStream::connect(&t.addr).unwrap();
        let c = connect(&ctx(), "", vec![with_conn(s).into(), with_retries(3).into(), with_retry_delay(Duration::from_millis(1)).into()]).unwrap();
        let before = t.requests.load(Ordering::SeqCst);
        let e = c.read_tag(&ctx(), "Missing").unwrap_err();
        match e {
            Error::Cip(ce) => assert_eq!(ce.status, cip::STATUS_PATH_DESTINATION_UNKNOWN),
            other => panic!("expected CIP error, got {other}"),
        }
        // Exactly one request: CIP errors are not retried.
        assert_eq!(t.requests.load(Ordering::SeqCst) - before, 1);
    }

    #[test]
    fn plc_trait_lifecycle() {
        let (_t, client) = connect_pair(symbolic_router(vec![0xC1, 0, 1]));
        assert!(client.is_connected());
        Plc::connect(&client, &ctx()).unwrap();
        client.disconnect(&ctx()).unwrap();
        assert!(!client.is_connected());
        assert!(client.read_tag(&ctx(), "T").is_err());
    }

    #[test]
    fn identity_and_services() {
        let (_t, client) = connect_pair(Arc::new(MessageRouter::new()));
        let id = client.list_identity(&ctx()).unwrap();
        assert_eq!(id[0].product_name, "FakeTarget");
        let sv = client.list_services(&ctx()).unwrap();
        assert_eq!(sv[0].name, "Communications");
    }

    #[test]
    fn list_tags_enumerates_symbols() {
        // Symbol class (0x6B) object: class query (instance 0) returns revision 3 / max
        // instance 2; instance 1 is tag "Abc" (DINT), instance 2 does not exist and is skipped.
        let router = Arc::new(MessageRouter::new());
        router.register_object(
            cip::CLASS_SYMBOL,
            MockObject::new(|_svc, path, _data| {
                let b = path.bytes();
                if b == [0x24, 0x00] {
                    return Ok(vec![2, 0, 1, 0, 0, 0, 3, 0, 2, 0, 0, 0, 2, 0]);
                }
                if b == [0x24, 0x01] {
                    return Ok(vec![2, 0, 1, 0, 0, 0, 3, 0, b'A', b'b', b'c', 2, 0, 0, 0, 0xC4, 0x00]);
                }
                Err(cip::Error::new(cip::STATUS_OBJECT_DOES_NOT_EXIST).into())
            }),
        );
        let (_t, client) = connect_pair(router);
        let tags = client.list_tags(&ctx()).unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!((tags[0].instance_id, tags[0].name.as_str(), tags[0].type_), (1, "Abc", TYPE_DINT));
    }

    // Retry behavior: a target that drops the first connection mid-operation; a reconnecting
    // client with retries re-registers and succeeds.
    #[test]
    fn reconnecting_client_retries_after_drop() {
        let router = symbolic_router(vec![0xC4, 0x00, 0x2A, 0, 0, 0]);
        let t = FakeTarget::start(router);
        // Drop each connection after serving RegisterSession only: the RR request is not answered.
        t.drop_after.store(1, Ordering::SeqCst);
        let c = new_reconnecting_client(
            &t.addr,
            vec![with_retries(2).into(), with_retry_delay(Duration::from_millis(10)).into()],
        );
        assert!(!c.is_connected(), "constructor must not dial");
        let e = c.read_tag(&ctx(), "T").unwrap_err();
        assert_eq!(e.to_string(), "operation failed after 3 retries");
        assert!(t.accepted.load(Ordering::SeqCst) >= 3, "should have reconnected per attempt");

        // Heal the target: next call reconnects and succeeds.
        t.drop_after.store(usize::MAX, Ordering::SeqCst);
        let data = c.read_tag(&ctx(), "T").unwrap();
        assert_eq!(data, vec![0xC4, 0x00, 0x2A, 0, 0, 0]);
        assert!(c.is_connected());
        c.close().unwrap();
    }

    // Go quirk: retries == 0 and an operation failure reports "operation failed after 1 retries".
    #[test]
    fn no_retries_operation_failure_message() {
        let t = FakeTarget::start(symbolic_router(vec![0xC1, 0, 0]));
        t.drop_after.store(1, Ordering::SeqCst);
        let c = new_reconnecting_client(&t.addr, vec![]);
        let e = c.read_tag(&ctx(), "T").unwrap_err();
        assert_eq!(e.to_string(), "operation failed after 1 retries");
    }

    #[test]
    fn connection_refused_with_no_retries_returns_cause() {
        let port = crate::testutil::find_free_port().unwrap();
        let c = new_reconnecting_client(&format!("127.0.0.1:{port}"), vec![]);
        assert!(c.read_tag(&ctx(), "T").is_err());
        assert!(!c.is_connected());
    }

    #[test]
    fn cancelled_context_stops_infinite_retries() {
        let port = crate::testutil::find_free_port().unwrap();
        let c = new_reconnecting_client(
            &format!("127.0.0.1:{port}"),
            vec![with_retries(-1).into(), with_retry_delay(Duration::from_millis(5)).into()],
        );
        let cx = Context::background().with_timeout(Duration::from_millis(100));
        let e = c.read_tag(&cx, "T").unwrap_err();
        assert!(matches!(e, Error::Plc(plc::Error::DeadlineExceeded)), "{e}");
    }

    #[test]
    fn address_without_port_gets_default_and_fails_cleanly() {
        // Nothing listens on 44818 locally in CI; either outcome must be an error or a connection,
        // never a panic. Use a reserved-invalid host to guarantee resolution failure.
        let c = new_reconnecting_client("invalid.invalid", vec![]);
        assert!(c.connect_for_test().is_err());
    }

    impl Client {
        fn connect_for_test(&self) -> plc::Result<()> {
            Plc::connect(self, &Context::background().with_timeout(Duration::from_secs(2)))
        }
    }
}
