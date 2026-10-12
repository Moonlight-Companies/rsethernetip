//! Ported from Go `ethernetip_test.go` (server-side tests). Implementation detail, not spec.
//! `net.Pipe` pairs are replaced by loopback TCP pairs.

use super::*;
use crate::cip::{
    decode_message_router_response, MessageRouterRequest, MessageRouterResponse, Object, Path,
    SERVICE_GET_ATTRIBUTE_SINGLE, SERVICE_READ_TAG, SERVICE_WRITE_TAG, STATUS_SERVICE_NOT_SUPPORTED,
};
use crate::eip::{
    Command, CommonPacketFormat, CpfItem, ITEM_ID_CONNECTED_DATA, ITEM_ID_CONNECTION_BASED, ITEM_ID_LIST_IDENTITY,
    ITEM_ID_NULL_ADDRESS, ITEM_ID_UNCONNECTED_MESSAGE, STATUS_INVALID_SESSION_HANDLE,
};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Instant;

type HandleFn = Box<dyn Fn(u8, &Path, &[u8]) -> cip::Result<Vec<u8>> + Send + Sync>;

struct MockObject {
    handle: Option<HandleFn>,
}

impl Object for MockObject {
    fn handle_request(&self, service: u8, path: &Path, data: &[u8]) -> cip::Result<Vec<u8>> {
        match &self.handle {
            Some(f) => f(service, path, data),
            None => Ok(vec![0x01, 0x02, 0x03, 0x04]),
        }
    }
}

fn mock() -> Arc<MockObject> {
    Arc::new(MockObject { handle: None })
}

fn mock_fn(f: impl Fn(u8, &Path, &[u8]) -> cip::Result<Vec<u8>> + Send + Sync + 'static) -> Arc<MockObject> {
    Arc::new(MockObject { handle: Some(Box::new(f)) })
}

// Loopback replacement for net.Pipe(): returns (server side, client side).
fn pipe() -> (TcpStream, TcpStream) {
    let ln = TcpListener::bind("127.0.0.1:0").unwrap();
    let c = TcpStream::connect(ln.local_addr().unwrap()).unwrap();
    let (s, _) = ln.accept().unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    (s, c)
}

// Spawns handle_conn for `srv` on a thread; returns a receiver that fires when it returns.
fn spawn_handle(srv: &Server, conn: TcpStream) -> mpsc::Receiver<()> {
    let (tx, rx) = mpsc::channel();
    let srv = srv.clone();
    thread::spawn(move || {
        srv.handle_conn(conn);
        let _ = tx.send(());
    });
    rx
}

// Go `setupPipePair`: server handling a single injected connection.
fn setup_pipe_pair(router: Arc<MessageRouter>) -> (Server, TcpStream) {
    let (s, c) = pipe();
    let srv = Server::builder(router).conn(s).build();
    srv.start(&Context::background(), "").unwrap();
    (srv, c)
}

struct Stop(Server);
impl Drop for Stop {
    fn drop(&mut self) {
        let _ = self.0.stop();
    }
}

fn send_eip_packet(conn: &mut TcpStream, cmd: Command, session: u32, ctx: [u8; 8], data: &[u8]) {
    let h = eip::EncapsulationHeader {
        command: cmd,
        length: data.len() as u16,
        session_handle: session,
        status: 0,
        sender_context: ctx,
        options: 0,
    };
    conn.write_all(&h.bytes()).unwrap();
    if !data.is_empty() {
        conn.write_all(data).unwrap();
    }
}

// Returns (command, session, status, sender context, data).
fn recv_eip_packet(conn: &mut TcpStream) -> (Command, u32, u32, [u8; 8], Vec<u8>) {
    let mut hb = [0u8; HEADER_SIZE];
    conn.read_exact(&mut hb).expect("read header");
    let h = eip::EncapsulationHeader::from_bytes(&hb).unwrap();
    let mut data = vec![0u8; h.length as usize];
    if h.length > 0 {
        conn.read_exact(&mut data).expect("read data");
    }
    (h.command, h.session_handle, h.status, h.sender_context, data)
}

fn register_session(conn: &mut TcpStream) -> u32 {
    let mut reg = [0u8; 4];
    reg[0..2].copy_from_slice(&1u16.to_le_bytes());
    send_eip_packet(conn, Command::REGISTER_SESSION, 0, [0; 8], &reg);
    let (cmd, session, status, _, _) = recv_eip_packet(conn);
    assert_eq!(cmd, Command::REGISTER_SESSION, "expected RegisterSession response, got {cmd}");
    assert_eq!(status, 0, "register session failed with status 0x{status:08X}");
    assert_ne!(session, 0, "session handle should not be 0");
    session
}

fn rr_payload(mr_req: &MessageRouterRequest) -> Vec<u8> {
    let mr = mr_req.encode().unwrap();
    let cpf = CommonPacketFormat::new(vec![
        CpfItem::new(ITEM_ID_NULL_ADDRESS, Vec::new()),
        CpfItem::new(ITEM_ID_UNCONNECTED_MESSAGE, mr),
    ]);
    let mut rr = vec![0u8; 6];
    rr.extend_from_slice(&cpf.encode().unwrap());
    rr
}

fn send_rr_data_request(conn: &mut TcpStream, session: u32, mr_req: &MessageRouterRequest) -> MessageRouterResponse {
    send_eip_packet(conn, Command::SEND_RR_DATA, session, [0; 8], &rr_payload(mr_req));
    let (cmd, _, status, _, resp) = recv_eip_packet(conn);
    assert_eq!(cmd, Command::SEND_RR_DATA, "expected SendRRData response, got {cmd}");
    assert_eq!(status, 0, "SendRRData failed at encapsulation level: 0x{status:08X}");
    assert!(resp.len() >= 6, "response data too short");
    let cpf = CommonPacketFormat::decode(&resp[6..]).unwrap();
    let item = cpf.find_item_by_type(ITEM_ID_UNCONNECTED_MESSAGE).expect("no unconnected message item in response");
    decode_message_router_response(&item.data).unwrap()
}

fn class4_inst1_req(service: u8, data: Vec<u8>) -> MessageRouterRequest {
    MessageRouterRequest { service, request_path: Path(vec![0x20, 0x04, 0x24, 0x01]), request_data: data }
}

fn wait_for(cond: impl Fn() -> bool, max: Duration) {
    let end = Instant::now() + max;
    while Instant::now() < end {
        if cond() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn eip_session_register_unregister() {
    let srv = Server::new(Arc::new(MessageRouter::new()));
    let (s, mut c) = pipe();
    let done = spawn_handle(&srv, s);

    let session = register_session(&mut c);
    assert_ne!(session, 0);

    send_eip_packet(&mut c, Command::UNREGISTER_SESSION, session, [0; 8], &[]);
    done.recv_timeout(Duration::from_secs(2)).expect("server should have closed connection after UnregisterSession");
}

#[test]
fn cip_read_tag() {
    let router = Arc::new(MessageRouter::new());
    let expected = vec![0xC4, 0x00, 0x2A, 0x00, 0x00, 0x00];
    let exp = expected.clone();
    router.register_object(
        0x04,
        mock_fn(move |service, _, _| {
            if service != SERVICE_READ_TAG {
                return Err(cip::Error::new(STATUS_SERVICE_NOT_SUPPORTED).into());
            }
            Ok(exp.clone())
        }),
    );
    let (srv, mut c) = setup_pipe_pair(router);
    let _g = Stop(srv);
    let session = register_session(&mut c);
    let resp = send_rr_data_request(&mut c, session, &class4_inst1_req(SERVICE_READ_TAG, vec![0x01, 0x00]));
    assert!(resp.is_success(), "ReadTag failed with status 0x{:02X}", resp.general_status);
    assert_eq!(resp.response_data, expected);
}

#[test]
fn cip_write_tag() {
    let router = Arc::new(MessageRouter::new());
    let written = Arc::new(Mutex::new(Vec::new()));
    let w = written.clone();
    router.register_object(
        0x04,
        mock_fn(move |service, _, data| {
            if service != SERVICE_WRITE_TAG {
                return Err(cip::Error::new(STATUS_SERVICE_NOT_SUPPORTED).into());
            }
            *w.lock().unwrap() = data.to_vec();
            Ok(Vec::new())
        }),
    );
    let (srv, mut c) = setup_pipe_pair(router);
    let _g = Stop(srv);
    let session = register_session(&mut c);
    let payload = vec![0xC4, 0x00, 0x01, 0x00, 0x64, 0x00, 0x00, 0x00];
    let resp = send_rr_data_request(&mut c, session, &class4_inst1_req(SERVICE_WRITE_TAG, payload.clone()));
    assert!(resp.is_success(), "WriteTag failed with status 0x{:02X}", resp.general_status);
    assert_eq!(*written.lock().unwrap(), payload);
}

#[test]
fn sender_context_preserved() {
    let srv = Server::new(Arc::new(MessageRouter::new()));
    let (s, mut c) = pipe();
    spawn_handle(&srv, s);
    let ctx = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    send_eip_packet(&mut c, Command::REGISTER_SESSION, 0, ctx, &[1, 0, 0, 0]);
    let (_, _, _, got, _) = recv_eip_packet(&mut c);
    assert_eq!(got, ctx);
}

#[test]
fn unsupported_command() {
    let srv = Server::new(Arc::new(MessageRouter::new()));
    let (s, mut c) = pipe();
    spawn_handle(&srv, s);
    send_eip_packet(&mut c, Command(0xFFFF), 0, [0; 8], &[]);
    let (_, _, status, _, _) = recv_eip_packet(&mut c);
    assert_eq!(status, 0x0001, "status = 0x{status:08X}, want 0x00000001");
}

#[test]
fn max_packet_size_exceeded() {
    let srv = Server::new(Arc::new(MessageRouter::new()));
    let (s, mut c) = pipe();
    let done = spawn_handle(&srv, s);
    let h = eip::EncapsulationHeader { command: Command::SEND_RR_DATA, length: 5000, ..Default::default() };
    c.write_all(&h.bytes()).unwrap();
    done.recv_timeout(Duration::from_secs(2)).expect("server should close connection for oversized packet");
}

#[test]
fn double_register_session() {
    let srv = Server::new(Arc::new(MessageRouter::new()));
    let (s, mut c) = pipe();
    spawn_handle(&srv, s);
    assert_ne!(register_session(&mut c), 0, "first session should not be 0");
    assert_ne!(register_session(&mut c), 0, "second session should not be 0");
}

#[test]
fn send_rr_data_short_payload() {
    let srv = Server::new(Arc::new(MessageRouter::new()));
    let (s, mut c) = pipe();
    spawn_handle(&srv, s);
    let session = register_session(&mut c);
    send_eip_packet(&mut c, Command::SEND_RR_DATA, session, [0; 8], &[0x00, 0x00]);
    let (cmd, _, status, _, _) = recv_eip_packet(&mut c);
    assert_eq!(cmd, Command::SEND_RR_DATA);
    assert_ne!(status, 0, "expected non-zero status for short SendRRData payload");
}

#[test]
fn nop_command() {
    let srv = Server::new(Arc::new(MessageRouter::new()));
    let (s, mut c) = pipe();
    spawn_handle(&srv, s);
    let session = register_session(&mut c);
    send_eip_packet(&mut c, Command::NOP, session, [0; 8], &[]);
    let (cmd, _, status, _, _) = recv_eip_packet(&mut c);
    assert_eq!(cmd, Command::NOP, "expected Nop response");
    assert_ne!(status, 0, "expected non-zero status for NOP command");
}

#[test]
fn send_rr_data_invalid_cpf() {
    let srv = Server::new(Arc::new(MessageRouter::new()));
    let (s, mut c) = pipe();
    spawn_handle(&srv, s);
    let session = register_session(&mut c);
    let payload = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF];
    send_eip_packet(&mut c, Command::SEND_RR_DATA, session, [0; 8], &payload);
    let (cmd, _, status, _, _) = recv_eip_packet(&mut c);
    assert_eq!(cmd, Command::SEND_RR_DATA);
    assert_ne!(status, 0, "expected non-zero status for invalid CPF");
}

#[test]
fn multiple_sender_contexts() {
    let router = Arc::new(MessageRouter::new());
    router.register_object(0x04, mock());
    let (srv, mut c) = setup_pipe_pair(router);
    let _g = Stop(srv);
    let session = register_session(&mut c);
    let contexts: [[u8; 8]; 4] = [
        [0x01, 0, 0, 0, 0, 0, 0, 0],
        [0x00, 0x02, 0, 0, 0, 0, 0, 0],
        [0xFF; 8],
        [0; 8],
    ];
    for (i, ctx) in contexts.iter().enumerate() {
        let rr = rr_payload(&class4_inst1_req(SERVICE_GET_ATTRIBUTE_SINGLE, Vec::new()));
        send_eip_packet(&mut c, Command::SEND_RR_DATA, session, *ctx, &rr);
        let (_, _, _, got, _) = recv_eip_packet(&mut c);
        assert_eq!(got, *ctx, "request {i}: sender context mismatch");
    }
}

#[test]
fn list_services_command() {
    let srv = Server::new(Arc::new(MessageRouter::new()));
    let (s, mut c) = pipe();
    spawn_handle(&srv, s);
    let _ = register_session(&mut c);
    send_eip_packet(&mut c, Command::LIST_SERVICES, 0, [0; 8], &[]);
    let (cmd, _, status, _, data) = recv_eip_packet(&mut c);
    assert_eq!(cmd, Command::LIST_SERVICES);
    assert_eq!(status, 0, "ListServices failed: status 0x{status:08X}");
    let items = eip::decode_list_services_response(&data).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "Communications");
    assert_ne!(items[0].capability_flags & 0x0020, 0, "expected CIP encapsulation capability flag (0x0020)");
}

#[test]
fn list_identity_command() {
    let srv = Server::builder(Arc::new(MessageRouter::new()))
        .identity(eip::ListIdentityItem {
            type_id: ITEM_ID_LIST_IDENTITY,
            vendor_id: 42,
            product_name: "TestDevice".into(),
            ..Default::default()
        })
        .build();
    let (s, mut c) = pipe();
    spawn_handle(&srv, s);
    let _ = register_session(&mut c);
    send_eip_packet(&mut c, Command::LIST_IDENTITY, 0, [0; 8], &[]);
    let (cmd, _, status, _, data) = recv_eip_packet(&mut c);
    assert_eq!(cmd, Command::LIST_IDENTITY);
    assert_eq!(status, 0, "ListIdentity failed: status 0x{status:08X}");
    let items = eip::decode_list_identity_response(&data).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].vendor_id, 42);
    assert_eq!(items[0].product_name, "TestDevice");
}

#[test]
fn send_unit_data_round_trip() {
    let router = Arc::new(MessageRouter::new());
    router.register_object(0x04, mock_fn(|_, _, _| Ok(vec![0xDE, 0xAD, 0xBE, 0xEF])));
    let (srv, mut c) = setup_pipe_pair(router);
    let _g = Stop(srv);
    let session = register_session(&mut c);

    let conn_id = 0x1234_5678u32.to_le_bytes().to_vec();
    let mr = class4_inst1_req(SERVICE_GET_ATTRIBUTE_SINGLE, Vec::new()).encode().unwrap();
    let mut conn_data = 1u16.to_le_bytes().to_vec();
    conn_data.extend_from_slice(&mr);
    let cpf = CommonPacketFormat::new(vec![
        CpfItem::new(ITEM_ID_CONNECTION_BASED, conn_id),
        CpfItem::new(eip::ITEM_ID_CONNECTED_TRANSPORT, conn_data),
    ]);
    let mut payload = vec![0u8; 6];
    payload.extend_from_slice(&cpf.encode().unwrap());
    send_eip_packet(&mut c, Command::SEND_UNIT_DATA, session, [0; 8], &payload);

    let (cmd, _, status, _, resp) = recv_eip_packet(&mut c);
    assert_eq!(cmd, Command::SEND_UNIT_DATA);
    assert_eq!(status, 0, "SendUnitData failed: status 0x{status:08X}");
    assert!(resp.len() >= 6, "response too short");
    let rcpf = CommonPacketFormat::decode(&resp[6..]).unwrap();
    let item = rcpf.find_item_by_type(ITEM_ID_CONNECTED_DATA).expect("missing connected data item in response");
    assert!(item.data.len() >= 6, "connected data too short: {} bytes", item.data.len());
    assert_eq!(u16::from_le_bytes([item.data[0], item.data[1]]), 1, "response sequence");
}

#[test]
fn session_handle_unique() {
    let srv = Server::new(Arc::new(MessageRouter::new()));
    let (s1, mut c1) = pipe();
    spawn_handle(&srv, s1);
    let (s2, mut c2) = pipe();
    spawn_handle(&srv, s2);
    let h1 = register_session(&mut c1);
    let h2 = register_session(&mut c2);
    assert_ne!(h1, h2, "session handles should be unique: both got 0x{h1:08X}");
    assert!(h1 != 0 && h2 != 0, "session handles must not be 0");
}

#[test]
fn session_handle_validation() {
    let router = Arc::new(MessageRouter::new());
    router.register_object(0x04, mock());
    let (srv, mut c) = setup_pipe_pair(router);
    let _g = Stop(srv);
    let session = register_session(&mut c);
    let rr = rr_payload(&class4_inst1_req(SERVICE_GET_ATTRIBUTE_SINGLE, Vec::new()));
    send_eip_packet(&mut c, Command::SEND_RR_DATA, session + 999, [0; 8], &rr);
    let (_, _, status, _, _) = recv_eip_packet(&mut c);
    assert_eq!(status, STATUS_INVALID_SESSION_HANDLE);
}

#[test]
fn session_handle_zero_rejected() {
    let router = Arc::new(MessageRouter::new());
    router.register_object(0x04, mock());
    let (srv, mut c) = setup_pipe_pair(router);
    let _g = Stop(srv);
    let _ = register_session(&mut c);
    let rr = rr_payload(&class4_inst1_req(SERVICE_GET_ATTRIBUTE_SINGLE, Vec::new()));
    send_eip_packet(&mut c, Command::SEND_RR_DATA, 0, [0; 8], &rr);
    let (_, _, status, _, _) = recv_eip_packet(&mut c);
    assert_eq!(status, STATUS_INVALID_SESSION_HANDLE);
}

#[test]
fn connected_clients() {
    let (s, mut c) = pipe();
    let srv = Server::builder(Arc::new(MessageRouter::new())).conn(s).build();
    srv.start(&Context::background(), "").unwrap();
    let _g = Stop(srv.clone());

    wait_for(|| srv.connected_clients().len() == 1, Duration::from_secs(2));
    assert_eq!(srv.connected_clients().len(), 1, "expected 1 client");

    let session = register_session(&mut c);
    thread::sleep(Duration::from_millis(20));
    let clients = srv.connected_clients();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].session_handle, session);

    drop(c);
    wait_for(|| srv.connected_clients().is_empty(), Duration::from_secs(2));
    assert_eq!(srv.connected_clients().len(), 0, "expected 0 clients after disconnect");
}

#[test]
fn client_callbacks() {
    let (ctx, crx) = mpsc::sync_channel::<ConnectedClient>(1);
    let (dtx, drx) = mpsc::sync_channel::<ConnectedClient>(1);
    let (s, c) = pipe();
    let srv = Server::builder(Arc::new(MessageRouter::new()))
        .conn(s)
        .on_client_connect(move |cl| {
            let _ = ctx.send(cl);
        })
        .on_client_disconnect(move |cl| {
            let _ = dtx.send(cl);
        })
        .build();
    srv.start(&Context::background(), "").unwrap();
    let _g = Stop(srv);

    crx.recv_timeout(Duration::from_secs(2)).expect("timeout waiting for connect callback");
    drop(c);
    drx.recv_timeout(Duration::from_secs(2)).expect("timeout waiting for disconnect callback");
}

// Replaces Go TestServerWithPipeListener: a server using an injected listener.
#[test]
fn server_with_injected_listener() {
    let router = Arc::new(MessageRouter::new());
    router.register_object(0x04, mock());
    let ln = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = ln.local_addr().unwrap();
    let srv = Server::builder(router).listener(ln).build();
    srv.start(&Context::background(), "").unwrap();
    let _g = Stop(srv);

    let mut c = TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    assert_ne!(register_session(&mut c), 0, "session handle should not be 0");
}

#[test]
fn server_shutdown_drain() {
    const N: usize = 3;
    let ln = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = ln.local_addr().unwrap();
    let srv = Server::builder(Arc::new(MessageRouter::new())).listener(ln).build();
    srv.start(&Context::background(), "").unwrap();

    let mut clients: Vec<TcpStream> = Vec::new();
    for _ in 0..N {
        let c = TcpStream::connect(addr).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        clients.push(c);
        thread::sleep(Duration::from_millis(20));
    }
    wait_for(|| srv.connected_clients().len() == N, Duration::from_secs(2));
    assert_eq!(srv.connected_clients().len(), N, "expected {N} connected clients");

    for c in clients.iter_mut() {
        assert_ne!(register_session(c), 0);
    }

    let readers: Vec<_> = clients
        .iter()
        .map(|c| {
            let mut c = c.try_clone().unwrap();
            thread::spawn(move || {
                let mut b = [0u8; 1];
                // Err or Ok(0) (EOF) both mean the server closed the connection.
                matches!(c.read(&mut b), Ok(0) | Err(_))
            })
        })
        .collect();

    srv.stop().unwrap();

    for (i, r) in readers.into_iter().enumerate() {
        assert!(r.join().unwrap(), "client {i}: expected read error/EOF after stop()");
    }
    wait_for(|| srv.connected_clients().is_empty(), Duration::from_secs(2));
    assert_eq!(srv.connected_clients().len(), 0, "expected 0 connected clients after stop()");
}

#[test]
fn server_max_clients() {
    const MAX: usize = 2;
    let ln = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = ln.local_addr().unwrap();
    let srv = Server::builder(Arc::new(MessageRouter::new())).listener(ln).max_clients(MAX).build();
    srv.start(&Context::background(), "").unwrap();
    let _g = Stop(srv.clone());

    let dial = || TcpStream::connect_timeout(&addr, Duration::from_secs(1)).unwrap();
    let conn1 = dial();
    let _conn2 = dial();
    wait_for(|| srv.connected_clients().len() >= MAX, Duration::from_secs(2));
    assert_eq!(srv.connected_clients().len(), MAX);

    // Third client should be rejected (closed immediately).
    let mut conn3 = dial();
    conn3.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut b = [0u8; 1];
    assert!(matches!(conn3.read(&mut b), Ok(0) | Err(_)), "expected 3rd client to be rejected");

    drop(conn1);
    wait_for(|| srv.connected_clients().len() < MAX, Duration::from_secs(2));

    let _conn4 = dial();
    wait_for(|| srv.connected_clients().len() >= MAX, Duration::from_secs(2));
    assert_eq!(srv.connected_clients().len(), MAX, "expected {MAX} connected clients after reconnect");
}

// Extra: stop() is idempotent and works before start.
#[test]
fn stop_is_idempotent() {
    let srv = Server::new(Arc::new(MessageRouter::new()));
    srv.stop().unwrap();
    srv.stop().unwrap();
}
