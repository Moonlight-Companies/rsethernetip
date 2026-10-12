//! CIP Connection Manager Object (Class 0x06) for EtherNet/IP.
//!
//! Port of Go `objects/connmgr/types.go` and `connmgr.go`.
//!
//! The Connection Manager establishes, maintains and tears down logical connections between
//! devices via the Forward_Open and Forward_Close services - how EtherNet/IP scanners set up
//! implicit (cyclic UDP) I/O connections with adapters. This module provides request/response
//! types for Forward_Open, Large_Forward_Open and Forward_Close plus a [`ConnectionManager`]
//! that tracks active connections and dispatches CIP requests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::cip::{
    CipError, Error, Object, Path, Result, BYTE, DWORD, STATUS_CONNECTION_FAILURE,
    STATUS_SERVICE_NOT_SUPPORTED, UDINT, UINT, USINT, WORD,
};

// ---------------------------------------------------------------------------
// Service codes
// ---------------------------------------------------------------------------

// Ref: ODVA CIP Vol. 1, Connection Manager Object (class 0x06) services - Forward_Close is
// service code 0x4E (section unverified).
// Forward_Close tears down a CIP connection.
pub const SERVICE_FORWARD_CLOSE: USINT = 0x4E;
// Ref: ODVA CIP Vol. 1, Connection Manager Object services - Unconnected_Send 0x52 (section
// unverified). Used to route unconnected messages.
pub const SERVICE_UNCONNECTED_SEND: USINT = 0x52;
// Ref: ODVA CIP Vol. 1, Connection Manager Object services - Forward_Open 0x54 (section
// unverified). Establishes a CIP connection.
pub const SERVICE_FORWARD_OPEN: USINT = 0x54;
// Ref: ODVA CIP Vol. 1, Connection Manager Object services - Large_Forward_Open 0x5B (section
// unverified). For connections needing 32-bit network connection parameters.
pub const SERVICE_LARGE_FORWARD_OPEN: USINT = 0x5B;
// Ref: ODVA CIP Vol. 1, Connection Manager Object services - Get_Connection_Data 0x56 (section
// unverified).
pub const SERVICE_GET_CONNECTION_DATA: USINT = 0x56;
// Ref: ODVA CIP Vol. 1, Connection Manager Object services - Search_Connection_Data 0x57
// (section unverified; Go names it SearchConnection).
pub const SERVICE_SEARCH_CONNECTION: USINT = 0x57;
// Ref: ODVA CIP Vol. 1, Connection Manager Object services - Get_Connection_Owner is 0x5A
// (section unverified). The Go-only "CloseConnection = 0x58" constant is not a defined
// Connection Manager service and has been removed.
pub const SERVICE_GET_CONNECTION_OWNER: USINT = 0x5A;

// ---------------------------------------------------------------------------
// Extended status codes
// ---------------------------------------------------------------------------

// Ref: ODVA CIP Vol. 1, Connection Manager Object - Connection Manager Service Request Error
// Codes (extended status for general status 0x01 Connection failure), Table number unverified.
// 0x0100: connection in use or duplicate forward open.
pub const EXT_STATUS_CONNECTION_IN_USE: UINT = 0x0100;
// Ref: CIP Vol. 1, Connection Manager extended status codes (table number unverified).
// 0x0103: transport class and trigger combination not supported.
pub const EXT_STATUS_TRANSPORT_NOT_SUPP: UINT = 0x0103;
// Ref: CIP Vol. 1, Connection Manager extended status codes (table number unverified).
// 0x0106: ownership conflict.
pub const EXT_STATUS_OWNERSHIP_CONFLICT: UINT = 0x0106;
// Ref: CIP Vol. 1, Connection Manager extended status codes (table number unverified).
// 0x0107: target connection not found. (Go used 0x0109 for this, which is a Go bug; fixed.)
pub const EXT_STATUS_CONNECTION_NOT_FOUND: UINT = 0x0107;
// Ref: CIP Vol. 1, Connection Manager extended status codes (table number unverified).
// 0x0109: invalid connection size.
pub const EXT_STATUS_INVALID_CONNECTION_SIZE: UINT = 0x0109;
// Ref: CIP Vol. 1, Connection Manager extended status codes (table number unverified).
// Go value 0x0315 "invalid segment type in path".
pub const EXT_STATUS_INVALID_SEGMENT_TYPE: UINT = 0x0315;
// Ref: CIP Vol. 1, Connection Manager extended status codes (table number unverified).
// 0x0311: invalid port ID specified in the route path (Go called it "invalid parameter").
pub const EXT_STATUS_INVALID_PORT_ID: UINT = 0x0311;
// Ref: CIP Vol. 1, Connection Manager extended status codes (table number unverified).
// 0x031C: miscellaneous (Go called it "vendor specific"; vendor-specific extended status
// is a separate range, exact bounds unverified).
pub const EXT_STATUS_MISCELLANEOUS: UINT = 0x031C;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

// Ref: ODVA CIP Vol. 1, Connection Manager Object, Forward_Open service request parameters
// (section unverified). Carries all parameters the originator proposes: priority/time tick,
// time-out ticks, connection IDs, the connection triad (serial number, vendor ID, originator
// serial number), timeout multiplier, requested packet intervals (RPI), network connection
// parameters, transport class/trigger and the connection path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForwardOpenRequest {
    pub priority_time_tick: BYTE,
    pub timeout_ticks: USINT,
    pub ot_connection_id: UDINT,
    pub to_connection_id: UDINT,
    pub connection_serial_number: UINT,
    pub vendor_id: UINT,
    pub originator_serial_number: UDINT,
    pub connection_timeout_multiplier: USINT,
    pub reserved: [BYTE; 3],
    pub ot_rpi: UDINT,
    pub ot_network_connection_params: WORD,
    pub to_rpi: UDINT,
    pub to_network_connection_params: WORD,
    pub transport_type_trigger: BYTE,
    pub connection_path_size: USINT,
    // Padded to an even number of bytes
    pub connection_path: Vec<u8>,
}

// Ref: ODVA CIP Vol. 1, Connection Manager Object, Forward_Open success response (section
// unverified). Echoes the connection IDs and triad and gives the actual packet intervals
// (API) the target uses, which may differ from the requested RPIs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForwardOpenResponse {
    pub ot_connection_id: UDINT,
    pub to_connection_id: UDINT,
    pub connection_serial_number: UINT,
    pub vendor_id: UINT,
    pub originator_serial_number: UDINT,
    // Actual Packet Interval (O->T)
    pub ot_api: UDINT,
    // Actual Packet Interval (T->O)
    pub to_api: UDINT,
    pub application_reply_size: USINT,
    pub reserved: USINT,
    pub application_reply: Vec<u8>,
}

// Ref: ODVA CIP Vol. 1, Connection Manager Object, Forward_Close service request (section
// unverified). The connection is identified by its triad: connection serial number, vendor
// ID and originator serial number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForwardCloseRequest {
    pub priority_time_tick: BYTE,
    pub timeout_ticks: USINT,
    pub connection_serial_number: UINT,
    pub vendor_id: UINT,
    pub originator_serial_number: UDINT,
    pub connection_path_size: USINT,
    pub reserved: USINT,
    // Padded
    pub connection_path: Vec<u8>,
}

// Ref: ODVA CIP Vol. 1, Connection Manager Object, Forward_Close success response (section
// unverified). Echoes the connection triad.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForwardCloseResponse {
    pub connection_serial_number: UINT,
    pub vendor_id: UINT,
    pub originator_serial_number: UDINT,
    pub application_reply_size: USINT,
    pub reserved: USINT,
    pub application_reply: Vec<u8>,
}

// Ref: ODVA CIP Vol. 1, Connection Manager Object, Large_Forward_Open request (service 0x5B;
// section unverified). Identical to Forward_Open except the network connection parameters are
// 32 bits wide, allowing connection sizes above 511 bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LargeForwardOpenRequest {
    pub priority_time_tick: BYTE,
    pub timeout_ticks: USINT,
    pub ot_connection_id: UDINT,
    pub to_connection_id: UDINT,
    pub connection_serial_number: UINT,
    pub vendor_id: UINT,
    pub originator_serial_number: UDINT,
    pub connection_timeout_multiplier: USINT,
    pub reserved: [BYTE; 3],
    pub ot_rpi: UDINT,
    // 32-bit for Large
    pub ot_network_connection_params: DWORD,
    pub to_rpi: UDINT,
    // 32-bit for Large
    pub to_network_connection_params: DWORD,
    pub transport_type_trigger: BYTE,
    pub connection_path_size: USINT,
    pub connection_path: Vec<u8>,
}

// Ref: ODVA CIP Vol. 1, Connection Manager Object, Large_Forward_Open success response
// (section unverified).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LargeForwardOpenResponse {
    pub ot_connection_id: UDINT,
    pub to_connection_id: UDINT,
    pub connection_serial_number: UINT,
    pub vendor_id: UINT,
    pub originator_serial_number: UDINT,
    pub ot_api: UDINT,
    pub to_api: UDINT,
    pub application_reply_size: USINT,
    pub reserved: USINT,
    pub application_reply: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Network connection parameters
// ---------------------------------------------------------------------------

// Ref: ODVA CIP Vol. 1, Connection Manager Object, Network Connection Parameters: bits 0-8 of
// the 16-bit word hold the connection size in bytes (section unverified).
// Extracts the connection size (bytes) from a 16-bit network connection parameter word.
pub fn connection_size_from_params(params: WORD) -> u16 {
    params & 0x01FF
}

// Ref: ODVA CIP Vol. 1, Connection Manager Object, Network Connection Parameters bit 9 =
// Fixed/Variable (section unverified). Fixed size connection (bit 9 = 0).
pub const CONN_PARAM_FIXED_SIZE: WORD = 0x0000;
// Ref: CIP Vol. 1, Network Connection Parameters bit 9 (section unverified). Variable size
// connection (bit 9 = 1).
pub const CONN_PARAM_VARIABLE_SIZE: WORD = 0x0200;
// Ref: CIP Vol. 1, Network Connection Parameters bits 13-14 = Connection Type (section
// unverified). Point-to-point (10b).
pub const CONN_PARAM_POINT_TO_POINT: WORD = 0x4000;
// Ref: CIP Vol. 1, Network Connection Parameters bits 13-14 = Connection Type (section
// unverified). Multicast (01b).
pub const CONN_PARAM_MULTICAST: WORD = 0x2000;
// Ref: CIP Vol. 1, Network Connection Parameters bits 10-11 = Priority (section unverified).
// Low priority (00b).
pub const CONN_PARAM_PRIORITY_LOW: WORD = 0x0000;
// Ref: CIP Vol. 1, Network Connection Parameters bits 10-11 = Priority (section unverified).
// High priority (01b).
pub const CONN_PARAM_PRIORITY_HIGH: WORD = 0x0400;
// Ref: CIP Vol. 1, Network Connection Parameters bits 10-11 = Priority (section unverified).
// Scheduled priority (10b).
pub const CONN_PARAM_PRIORITY_SCHEDULED: WORD = 0x0800;

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

// Ref: ODVA CIP Vol. 1, Forward_Open request layout, all fields little-endian (section
// unverified). Path size is in 16-bit words.
impl ForwardOpenRequest {
    // Serializes the request. Like Go, the path size on the wire is derived from the length
    // of `connection_path` (len/2), not from the `connection_path_size` field.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(36 + self.connection_path.len());
        buf.push(self.priority_time_tick);
        buf.push(self.timeout_ticks);
        buf.extend_from_slice(&self.ot_connection_id.to_le_bytes());
        buf.extend_from_slice(&self.to_connection_id.to_le_bytes());
        buf.extend_from_slice(&self.connection_serial_number.to_le_bytes());
        buf.extend_from_slice(&self.vendor_id.to_le_bytes());
        buf.extend_from_slice(&self.originator_serial_number.to_le_bytes());
        buf.push(self.connection_timeout_multiplier);
        buf.extend_from_slice(&self.reserved);
        buf.extend_from_slice(&self.ot_rpi.to_le_bytes());
        buf.extend_from_slice(&self.ot_network_connection_params.to_le_bytes());
        buf.extend_from_slice(&self.to_rpi.to_le_bytes());
        buf.extend_from_slice(&self.to_network_connection_params.to_le_bytes());
        buf.push(self.transport_type_trigger);
        buf.push((self.connection_path.len() / 2) as USINT);
        buf.extend_from_slice(&self.connection_path);
        Ok(buf)
    }
}

// Ref: ODVA CIP Vol. 1, Large_Forward_Open request layout: as Forward_Open but with 32-bit
// network connection parameters (section unverified). Added in the Rust port; the Go source
// only defines the struct.
impl LargeForwardOpenRequest {
    // Serializes the request (path size derived from the path length, as for Forward_Open).
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(40 + self.connection_path.len());
        buf.push(self.priority_time_tick);
        buf.push(self.timeout_ticks);
        buf.extend_from_slice(&self.ot_connection_id.to_le_bytes());
        buf.extend_from_slice(&self.to_connection_id.to_le_bytes());
        buf.extend_from_slice(&self.connection_serial_number.to_le_bytes());
        buf.extend_from_slice(&self.vendor_id.to_le_bytes());
        buf.extend_from_slice(&self.originator_serial_number.to_le_bytes());
        buf.push(self.connection_timeout_multiplier);
        buf.extend_from_slice(&self.reserved);
        buf.extend_from_slice(&self.ot_rpi.to_le_bytes());
        buf.extend_from_slice(&self.ot_network_connection_params.to_le_bytes());
        buf.extend_from_slice(&self.to_rpi.to_le_bytes());
        buf.extend_from_slice(&self.to_network_connection_params.to_le_bytes());
        buf.push(self.transport_type_trigger);
        buf.push((self.connection_path.len() / 2) as USINT);
        buf.extend_from_slice(&self.connection_path);
        Ok(buf)
    }
}

// Ref: ODVA CIP Vol. 1, Forward_Close request layout (section unverified). Path size (words)
// precedes a reserved byte, then the path.
impl ForwardCloseRequest {
    // Serializes the request (path size derived from the path length, as in Go).
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(12 + self.connection_path.len());
        buf.push(self.priority_time_tick);
        buf.push(self.timeout_ticks);
        buf.extend_from_slice(&self.connection_serial_number.to_le_bytes());
        buf.extend_from_slice(&self.vendor_id.to_le_bytes());
        buf.extend_from_slice(&self.originator_serial_number.to_le_bytes());
        buf.push((self.connection_path.len() / 2) as USINT);
        buf.push(self.reserved);
        buf.extend_from_slice(&self.connection_path);
        Ok(buf)
    }
}

// ---------------------------------------------------------------------------
// Connection Manager object
// ---------------------------------------------------------------------------

// Ref: ODVA CIP Vol. 1, Connection Manager Object - a connection is identified by its
// connection triad (connection serial number, originator vendor ID, originator serial
// number); CIP Vol. 1 Section 3-5.5.2 as cited by the Go source.
// A logical CIP connection established via Forward_Open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Connection {
    pub ot_connection_id: u32,
    pub to_connection_id: u32,
    pub connection_serial_number: UINT,
    pub vendor_id: UINT,
    pub originator_serial_number: UDINT,
}

// Implementation detail (not spec): callback types for connection events.
type OnOpenFn = Arc<dyn Fn(&Connection, &ForwardOpenRequest) + Send + Sync>;
type OnCloseFn = Arc<dyn Fn(&Connection) + Send + Sync>;

// Implementation detail (not spec): mutable state guarded by the manager's lock.
struct State {
    connections: HashMap<u32, Arc<Connection>>,
    next_conn_id: u32,
}

// Ref: ODVA CIP Vol. 1, Connection Manager Object (class 0x06; section unverified).
// Processes Forward_Open and Forward_Close requests, keeps a table of active connections and
// invokes optional callbacks on open/close. All methods are safe for concurrent use.
//
// Implementation detail: Go's functional options (WithOnOpen / WithOnClose) become the
// builder methods `with_on_open` / `with_on_close`.
pub struct ConnectionManager {
    state: Mutex<State>,
    on_open: Option<OnOpenFn>,
    on_close: Option<OnCloseFn>,
}

impl Default for ConnectionManager {
    fn default() -> Self {
        ConnectionManager::new()
    }
}

impl ConnectionManager {
    // Creates a ConnectionManager. Target connection IDs are allocated starting after
    // 0x80000000 (first ID is 0x80000001) to avoid collisions with originator-assigned IDs
    // (implementation choice carried over from Go).
    pub fn new() -> ConnectionManager {
        ConnectionManager {
            state: Mutex::new(State { connections: HashMap::new(), next_conn_id: 0x8000_0000 }),
            on_open: None,
            on_close: None,
        }
    }

    // Sets a callback invoked after a successful Forward_Open (Go `WithOnOpen`).
    pub fn with_on_open<F>(mut self, f: F) -> ConnectionManager
    where
        F: Fn(&Connection, &ForwardOpenRequest) + Send + Sync + 'static,
    {
        self.on_open = Some(Arc::new(f));
        self
    }

    // Sets a callback invoked after a successful Forward_Close (Go `WithOnClose`).
    pub fn with_on_close<F>(mut self, f: F) -> ConnectionManager
    where
        F: Fn(&Connection) + Send + Sync + 'static,
    {
        self.on_close = Some(Arc::new(f));
        self
    }

    // Implementation detail: number of currently tracked connections.
    pub fn connection_count(&self) -> usize {
        self.lock().connections.len()
    }

    // Implementation detail: poison-tolerant lock.
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    // Ref: ODVA CIP Vol. 1, Forward_Open service (0x54) request/response layouts (section
    // unverified); duplicate triads are rejected per CIP Vol. 1 Section 3-5.5.2 (as cited in
    // the Go source) with General Status 0x01 and extended status 0x0100.
    // Processes a Forward_Open request from its raw service data: deserializes it, allocates
    // a new target connection ID, stores the connection, invokes the on-open callback and
    // returns the serialized ForwardOpenResponse.
    pub fn handle_forward_open(&self, req_data: &[u8]) -> Result<Vec<u8>> {
        let mut r = Rd::new(req_data);
        let mut req = ForwardOpenRequest::default();

        req.priority_time_tick = r.u8()?;
        req.timeout_ticks = r.u8()?;
        req.ot_connection_id = r.u32()?;
        req.to_connection_id = r.u32()?;
        req.connection_serial_number = r.u16()?;
        req.vendor_id = r.u16()?;
        req.originator_serial_number = r.u32()?;
        req.connection_timeout_multiplier = r.u8()?;
        req.reserved = [r.u8()?, r.u8()?, r.u8()?];
        req.ot_rpi = r.u32()?;
        req.ot_network_connection_params = r.u16()?;
        req.to_rpi = r.u32()?;
        req.to_network_connection_params = r.u16()?;
        req.transport_type_trigger = r.u8()?;
        req.connection_path_size = r.u8()?;
        req.connection_path = r.path(req.connection_path_size as usize * 2)?;

        let my_conn_id;
        let conn;
        {
            let mut st = self.lock();

            // Reject duplicate connection triads (CIP Vol. 1 Section 3-5.5.2).
            for existing in st.connections.values() {
                if existing.connection_serial_number == req.connection_serial_number
                    && existing.vendor_id == req.vendor_id
                    && existing.originator_serial_number == req.originator_serial_number
                {
                    return Err(CipError::Status(Error::with_ext(
                        STATUS_CONNECTION_FAILURE,
                        vec![EXT_STATUS_CONNECTION_IN_USE],
                    )));
                }
            }

            st.next_conn_id = st.next_conn_id.wrapping_add(1);
            my_conn_id = st.next_conn_id;

            conn = Arc::new(Connection {
                // Ref: CIP Vol. 1, Forward_Open reply (section unverified): the O->T
                // connection ID is chosen by the target; the T->O ID is the one the
                // originator proposed and is echoed back.
                ot_connection_id: my_conn_id,
                to_connection_id: req.to_connection_id,
                connection_serial_number: req.connection_serial_number,
                vendor_id: req.vendor_id,
                originator_serial_number: req.originator_serial_number,
            });
            st.connections.insert(my_conn_id, Arc::clone(&conn));
        }

        // Callback runs outside the lock (as in Go).
        if let Some(cb) = &self.on_open {
            cb(&conn, &req);
        }

        // The target echoes the requested RPIs as the actual packet intervals (as in Go).
        let resp = ForwardOpenResponse {
            ot_connection_id: my_conn_id,
            to_connection_id: req.to_connection_id,
            connection_serial_number: req.connection_serial_number,
            vendor_id: req.vendor_id,
            originator_serial_number: req.originator_serial_number,
            ot_api: req.ot_rpi,
            to_api: req.to_rpi,
            application_reply_size: 0,
            reserved: 0,
            application_reply: Vec::new(),
        };

        let mut buf = Vec::with_capacity(26);
        buf.extend_from_slice(&resp.ot_connection_id.to_le_bytes());
        buf.extend_from_slice(&resp.to_connection_id.to_le_bytes());
        buf.extend_from_slice(&resp.connection_serial_number.to_le_bytes());
        buf.extend_from_slice(&resp.vendor_id.to_le_bytes());
        buf.extend_from_slice(&resp.originator_serial_number.to_le_bytes());
        buf.extend_from_slice(&resp.ot_api.to_le_bytes());
        buf.extend_from_slice(&resp.to_api.to_le_bytes());
        buf.push(resp.application_reply_size);
        buf.push(resp.reserved);
        Ok(buf)
    }

    // Ref: ODVA CIP Vol. 1, Forward_Close service (0x4E) request/response layouts (section
    // unverified).
    // Processes a Forward_Close request from its raw service data: finds the connection by
    // its triad, removes it, invokes the on-close callback and returns the serialized
    // ForwardCloseResponse. A triad that matches nothing returns status 0x01 / ext 0x0107 (Go returned success).
    pub fn handle_forward_close(&self, req_data: &[u8]) -> Result<Vec<u8>> {
        let mut r = Rd::new(req_data);
        let mut req = ForwardCloseRequest::default();

        req.priority_time_tick = r.u8()?;
        req.timeout_ticks = r.u8()?;
        req.connection_serial_number = r.u16()?;
        req.vendor_id = r.u16()?;
        req.originator_serial_number = r.u32()?;
        req.connection_path_size = r.u8()?;
        req.reserved = r.u8()?;
        req.connection_path = r.path(req.connection_path_size as usize * 2)?;

        // Find and remove the connection by triad.
        let closed = {
            let mut st = self.lock();
            let key = st
                .connections
                .iter()
                .find(|(_, c)| {
                    c.connection_serial_number == req.connection_serial_number
                        && c.vendor_id == req.vendor_id
                        && c.originator_serial_number == req.originator_serial_number
                })
                .map(|(id, _)| *id);
            key.and_then(|k| st.connections.remove(&k))
        };

        // Ref: CIP Vol. 1, Forward_Close (section unverified): a triad that matches no
        // connection is general status 0x01 with extended status 0x0107.
        let Some(c) = &closed else {
            return Err(CipError::Status(Error::with_ext(
                STATUS_CONNECTION_FAILURE,
                vec![EXT_STATUS_CONNECTION_NOT_FOUND],
            )));
        };
        if let Some(cb) = &self.on_close {
            cb(c);
        }

        let resp = ForwardCloseResponse {
            connection_serial_number: req.connection_serial_number,
            vendor_id: req.vendor_id,
            originator_serial_number: req.originator_serial_number,
            application_reply_size: 0,
            reserved: 0,
            application_reply: Vec::new(),
        };

        let mut buf = Vec::with_capacity(10);
        buf.extend_from_slice(&resp.connection_serial_number.to_le_bytes());
        buf.extend_from_slice(&resp.vendor_id.to_le_bytes());
        buf.extend_from_slice(&resp.originator_serial_number.to_le_bytes());
        buf.push(resp.application_reply_size);
        buf.push(resp.reserved);
        Ok(buf)
    }
}

// Ref: ODVA CIP Vol. 1, Connection Manager Object services (section unverified).
// Dispatches to Forward_Open / Forward_Close by service code; any other service returns
// StatusServiceNotSupported (Large_Forward_Open is not handled, as in Go).
impl Object for ConnectionManager {
    fn handle_request(&self, service: USINT, _path: &Path, data: &[u8]) -> Result<Vec<u8>> {
        match service {
            SERVICE_FORWARD_OPEN => self.handle_forward_open(data),
            SERVICE_FORWARD_CLOSE => self.handle_forward_close(data),
            _ => Err(CipError::Status(Error::new(STATUS_SERVICE_NOT_SUPPORTED))),
        }
    }
}

// Implementation detail (not spec): bounds-checked little-endian reader that reproduces Go's
// `binary.Read` / `bytes.Reader` error kinds: nothing left -> EOF, partial -> unexpected EOF.
struct Rd<'a> {
    data: &'a [u8],
}

impl<'a> Rd<'a> {
    fn new(data: &'a [u8]) -> Self {
        Rd { data }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.data.is_empty() {
            return Err(CipError::Eof);
        }
        if self.data.len() < n {
            return Err(CipError::UnexpectedEof);
        }
        let (a, b) = self.data.split_at(n);
        self.data = b;
        Ok(a)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    // Mirrors Go's `make([]byte, n); r.Read(buf)`: a single Read returns EOF only when the
    // reader is already empty (even for n == 0); a short remainder yields a zero-padded path.
    fn path(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut out = vec![0u8; n];
        if self.data.is_empty() {
            return Err(CipError::Eof);
        }
        let m = n.min(self.data.len());
        out[..m].copy_from_slice(&self.data[..m]);
        self.data = &self.data[m..];
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // Builds an encoded ForwardOpenRequest with the given triad and connection path.
    fn build_open_request(serial: u16, vendor: u16, orig_sn: u32, conn_path: Vec<u8>) -> Vec<u8> {
        let req = ForwardOpenRequest {
            priority_time_tick: 0x0A,
            timeout_ticks: 0xF0,
            ot_connection_id: 0x1234,
            to_connection_id: 0x5678,
            connection_serial_number: serial,
            vendor_id: vendor,
            originator_serial_number: orig_sn,
            connection_timeout_multiplier: 3,
            ot_rpi: 10000,
            ot_network_connection_params: 0x43F4,
            to_rpi: 10000,
            to_network_connection_params: 0x43F4,
            transport_type_trigger: 0x01,
            connection_path: conn_path,
            ..Default::default()
        };
        req.encode().expect("Encode ForwardOpenRequest")
    }

    // Builds an encoded ForwardCloseRequest.
    fn build_close_request(serial: u16, vendor: u16, orig_sn: u32, conn_path: Vec<u8>) -> Vec<u8> {
        let req = ForwardCloseRequest {
            priority_time_tick: 0x0A,
            timeout_ticks: 0xF0,
            connection_serial_number: serial,
            vendor_id: vendor,
            originator_serial_number: orig_sn,
            connection_path: conn_path,
            ..Default::default()
        };
        req.encode().expect("Encode ForwardCloseRequest")
    }

    fn conn_path() -> Vec<u8> {
        let mut p = Path::new();
        p.add_class(0x04);
        p.add_instance(100);
        p.add_class(0x04);
        p.add_instance(101);
        p.bytes().to_vec()
    }

    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
    }
    fn u16_at(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }

    #[test]
    fn forward_open_request_encode_round_trip() {
        let cm = ConnectionManager::new();
        let data = build_open_request(0x0001, 0x0042, 0xABCD, conn_path());
        let resp = cm.handle_forward_open(&data).unwrap();

        // O->T ID is allocated by the target; T->O ID is echoed from the request.
        assert_eq!(u32_at(&resp, 0), 0x8000_0001, "OTConnectionID");
        assert_eq!(u32_at(&resp, 4), 0x5678, "TOConnectionID");
        assert_eq!(u16_at(&resp, 8), 0x0001);
        assert_eq!(u16_at(&resp, 10), 0x0042);
        assert_eq!(u32_at(&resp, 12), 0xABCD);
        // APIs echo the RPIs; app reply size and reserved are 0.
        assert_eq!(u32_at(&resp, 16), 10000);
        assert_eq!(u32_at(&resp, 20), 10000);
        assert_eq!(resp.len(), 26);
    }

    #[test]
    fn forward_close_request_encode_round_trip() {
        let cm = ConnectionManager::new();
        let open = build_open_request(0x0007, 0x0099, 0xDEAD, conn_path());
        cm.handle_forward_open(&open).unwrap();

        let close = build_close_request(0x0007, 0x0099, 0xDEAD, conn_path());
        let resp = cm.handle_forward_close(&close).unwrap();

        assert_eq!(u16_at(&resp, 0), 0x0007);
        assert_eq!(u16_at(&resp, 2), 0x0099);
        assert_eq!(u32_at(&resp, 4), 0xDEAD);
        assert_eq!(resp.len(), 10);
    }

    #[test]
    fn connection_size_from_params_cases() {
        let tests: [(&str, WORD, u16); 5] = [
            ("zero", 0x0000, 0),
            ("small fixed", CONN_PARAM_FIXED_SIZE | 64, 64),
            ("max 9-bit", 0x01FF, 511),
            (
                "variable with flags",
                CONN_PARAM_VARIABLE_SIZE | CONN_PARAM_POINT_TO_POINT | CONN_PARAM_PRIORITY_HIGH | 200,
                200,
            ),
            ("high bits ignored", 0xFE00 | 100, 100),
        ];
        for (name, params, want) in tests {
            assert_eq!(connection_size_from_params(params), want, "{}", name);
        }
    }

    #[test]
    fn on_open_on_close_callbacks() {
        let open_conn: Arc<StdMutex<Option<Connection>>> = Arc::new(StdMutex::new(None));
        let open_req: Arc<StdMutex<Option<ForwardOpenRequest>>> = Arc::new(StdMutex::new(None));
        let close_conn: Arc<StdMutex<Option<Connection>>> = Arc::new(StdMutex::new(None));

        let (oc, or, cc) = (open_conn.clone(), open_req.clone(), close_conn.clone());
        let cm = ConnectionManager::new()
            .with_on_open(move |c, req| {
                *oc.lock().unwrap() = Some(c.clone());
                *or.lock().unwrap() = Some(req.clone());
            })
            .with_on_close(move |c| {
                *cc.lock().unwrap() = Some(c.clone());
            });

        let open = build_open_request(0x0010, 0x0042, 0xBEEF, conn_path());
        cm.handle_forward_open(&open).unwrap();

        let c = open_conn.lock().unwrap().clone().expect("OnOpen callback not fired");
        assert_eq!(c.connection_serial_number, 0x0010);
        assert_eq!(c.vendor_id, 0x0042);
        let req = open_req.lock().unwrap().clone().expect("OnOpen request is nil");
        assert_eq!(req.ot_rpi, 10000);

        let close = build_close_request(0x0010, 0x0042, 0xBEEF, conn_path());
        cm.handle_forward_close(&close).unwrap();
        let c = close_conn.lock().unwrap().clone().expect("OnClose callback not fired");
        assert_eq!(c.connection_serial_number, 0x0010);
    }

    #[test]
    fn connection_tracking() {
        let cm = ConnectionManager::new();
        for i in 0..3u16 {
            let data = build_open_request(i + 1, 0x0042, i as u32 + 100, conn_path());
            cm.handle_forward_open(&data).unwrap();
        }
        assert_eq!(cm.connection_count(), 3);
        for i in 0..3u16 {
            let data = build_close_request(i + 1, 0x0042, i as u32 + 100, conn_path());
            cm.handle_forward_close(&data).unwrap();
        }
        assert_eq!(cm.connection_count(), 0);
    }

    #[test]
    fn duplicate_connection_rejected() {
        let cm = ConnectionManager::new();
        let data = build_open_request(0x0001, 0x0042, 0xAAAA, conn_path());

        cm.handle_forward_open(&data).unwrap();

        let err = cm.handle_forward_open(&data).unwrap_err();
        match err {
            CipError::Status(e) => {
                assert_eq!(e.status, STATUS_CONNECTION_FAILURE);
                assert_eq!(e.ext_status, vec![EXT_STATUS_CONNECTION_IN_USE]);
            }
            other => panic!("expected cip.Error, got {:?}", other),
        }
        assert_eq!(cm.connection_count(), 1);

        let close = build_close_request(0x0001, 0x0042, 0xAAAA, conn_path());
        cm.handle_forward_close(&close).unwrap();
        cm.handle_forward_open(&data).expect("re-open after close");
    }

    // Forward_Close of an unknown triad: general status 0x01, extended status 0x0107.
    #[test]
    fn forward_close_unknown_triad_fails() {
        let cm = ConnectionManager::new();
        let close = build_close_request(9, 9, 9, conn_path());
        match cm.handle_forward_close(&close).unwrap_err() {
            CipError::Status(e) => {
                assert_eq!(e.status, STATUS_CONNECTION_FAILURE);
                assert_eq!(e.ext_status, vec![EXT_STATUS_CONNECTION_NOT_FOUND]);
            }
            other => panic!("expected cip.Error, got {:?}", other),
        }
    }

    #[test]
    fn handle_request_dispatch() {
        let cm = ConnectionManager::new();
        let data = build_open_request(0x0001, 0x0042, 0x1111, conn_path());
        let none = Path::new();

        cm.handle_request(SERVICE_FORWARD_OPEN, &none, &data).unwrap();
        let close = build_close_request(0x0001, 0x0042, 0x1111, conn_path());
        cm.handle_request(SERVICE_FORWARD_CLOSE, &none, &close).unwrap();

        match cm.handle_request(0xFF, &none, &[]).unwrap_err() {
            CipError::Status(e) => assert_eq!(e.status, STATUS_SERVICE_NOT_SUPPORTED),
            other => panic!("expected cip.Error, got {:?}", other),
        }
    }

    // Added: Forward_Open wire layout and truncated-input behavior (no panics).
    #[test]
    fn forward_open_wire_layout_and_truncation() {
        let data = build_open_request(1, 2, 3, conn_path());
        // 34 fixed bytes + 16-byte path (8 words... 4 segments x 2 bytes... computed)
        assert_eq!(data[0], 0x0A);
        assert_eq!(data[1], 0xF0);
        assert_eq!(data[data.len() - conn_path().len() - 1] as usize, conn_path().len() / 2);
        assert_eq!(&data[data.len() - conn_path().len()..], &conn_path()[..]);

        let cm = ConnectionManager::new();
        assert_eq!(cm.handle_forward_open(&[]).unwrap_err(), CipError::Eof);
        assert_eq!(cm.handle_forward_open(&data[..3]).unwrap_err(), CipError::UnexpectedEof);
        // Fixed part complete but no path bytes at all: Go's Read returns EOF.
        assert_eq!(cm.handle_forward_open(&data[..data.len() - conn_path().len()]).unwrap_err(), CipError::Eof);
        // Short path is accepted and zero padded.
        assert!(cm.handle_forward_open(&data[..data.len() - 1]).is_ok());
    }

    // Added: Large_Forward_Open request carries 32-bit connection parameters.
    #[test]
    fn large_forward_open_encode() {
        let req = LargeForwardOpenRequest {
            ot_network_connection_params: 0x4200_0900,
            to_network_connection_params: 0x2200_0900,
            ot_rpi: 20000,
            connection_path: vec![0x20, 0x04, 0x24, 0x64],
            ..Default::default()
        };
        let b = req.encode().unwrap();
        assert_eq!(b.len(), 2 + 4 + 4 + 2 + 2 + 4 + 1 + 3 + 4 + 4 + 4 + 4 + 1 + 1 + 4);
        assert_eq!(u32_at(&b, 22), 20000);
        assert_eq!(u32_at(&b, 26), 0x4200_0900);
        assert_eq!(u32_at(&b, 34), 0x2200_0900);
        assert_eq!(b[b.len() - 5], 2);
    }
}
