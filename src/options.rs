//! Functional options for [`TcpConn`](crate::conn::TcpConn) and
//! [`Client`](crate::client::Client).
//!
//! Port of Go `options.go`. Go's functional options (`func(*cfg)`) become enums
//! applied to a config struct; `Connect`/`NewReconnectingClient` route a mixed
//! list of conn/client/transport options through [`Opt`].
//!
//! Implementation detail, not spec: option plumbing is a library concern. The
//! only protocol-visible default here is the TCP port 44818 (see `conn.rs`).

use std::io::Write;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use crate::conn::{HexDumper, Result};
use crate::logging::{Logger, NopLogger};
use crate::transport::TransportOption;

// ---------- ConnOption ----------

// Implementation detail (not spec): Go `connConfig`.
/// Configuration for [`TcpConn`](crate::conn::TcpConn).
pub struct ConnConfig {
    /// Pre-existing stream bypassing the dial.
    pub conn: Option<TcpStream>,
    /// Timeout for the initial TCP dial (default 5 seconds).
    pub dial_timeout: Duration,
    /// Optional hex dump tracer.
    pub hex_dumper: Option<Arc<HexDumper>>,
}

impl Default for ConnConfig {
    fn default() -> Self {
        ConnConfig { conn: None, dial_timeout: Duration::from_secs(5), hex_dumper: None }
    }
}

impl ConnConfig {
    // Implementation detail: an injected stream is duplicated (`try_clone`) so the same option
    // list can be reused by every reconnect, like Go reusing one `net.Conn`.
    /// Applies one option.
    pub fn apply(&mut self, opt: &ConnOption) -> Result<()> {
        match opt {
            ConnOption::Conn(s) => self.conn = Some(s.try_clone()?),
            ConnOption::DialTimeout(d) => self.dial_timeout = *d,
            ConnOption::HexDump(h) => self.hex_dumper = Some(h.clone()),
        }
        Ok(())
    }
}

// Implementation detail (not spec): Go `ConnOption`.
/// Configures a [`TcpConn`](crate::conn::TcpConn).
pub enum ConnOption {
    /// Injects a pre-existing connection, bypassing the TCP dial.
    Conn(TcpStream),
    /// Timeout for the initial TCP dial.
    DialTimeout(Duration),
    /// Hex dump tracing of all wire traffic.
    HexDump(Arc<HexDumper>),
}

/// Injects a pre-existing stream, bypassing the TCP dial (useful for tests).
pub fn with_conn(conn: TcpStream) -> ConnOption {
    ConnOption::Conn(conn)
}

/// Sets the timeout for the initial TCP dial. Default is 5 seconds.
pub fn with_dial_timeout(d: Duration) -> ConnOption {
    ConnOption::DialTimeout(d)
}

/// Enables hex dump tracing of all EtherNet/IP wire traffic. Data read from
/// and written to the TCP connection is formatted in traditional hex dump
/// style and written to `out`.
pub fn with_hex_dump(out: impl Write + Send + 'static) -> ConnOption {
    ConnOption::HexDump(Arc::new(HexDumper::new(out)))
}

// ---------- ClientOption ----------

// Ref (defaults): Go `pccc.go` DefaultCIPVendorID / DefaultCIPSerialNumber - originator stamp in
// the Execute_PCCC requestor-ID header; an SLC/MicroLogix does not validate them
// (Allen-Bradley PCCC / DF1 protocol manual 1770-6.5.16; CIP requestor ID layout, section unverified).
// Implementation detail: held here so `Client` can initialise without depending on `pccc_glue`.
pub(crate) const DEFAULT_CIP_VENDOR_ID: u16 = 0x1234;
pub(crate) const DEFAULT_CIP_SERIAL_NUMBER: u32 = 0x1234_5678;

// Implementation detail (not spec): Go `ClientOption` (a `func(*Client)`), plus the
// `WithCIPVendorID` / `WithCIPSerialNumber` options defined in Go `pccc.go`.
/// Configures a [`Client`](crate::client::Client).
#[derive(Clone)]
pub enum ClientOption {
    /// Max retry attempts for failed operations; 0 = none, -1 = infinite.
    Retries(i32),
    /// Delay between retries (default 1 second).
    RetryDelay(Duration),
    /// Logger for diagnostic output.
    Logger(Arc<dyn Logger>),
    /// CIP vendor ID written into the Execute_PCCC requestor header.
    CipVendorId(u16),
    /// CIP serial number written into the Execute_PCCC requestor header.
    CipSerialNumber(u32),
}

/// Sets the maximum number of retry attempts. 0 means no retries (fail
/// immediately) and -1 means infinite retries.
pub fn with_retries(n: i32) -> ClientOption {
    ClientOption::Retries(n)
}

/// Sets the delay between retries. Default is 1 second.
pub fn with_retry_delay(d: Duration) -> ClientOption {
    ClientOption::RetryDelay(d)
}

/// Sets the logger used by the client for diagnostic output.
pub fn with_logger(l: Arc<dyn Logger>) -> ClientOption {
    ClientOption::Logger(l)
}

/// Overrides the CIP vendor ID of every Execute_PCCC (0x4B) request.
pub fn with_cip_vendor_id(id: u16) -> ClientOption {
    ClientOption::CipVendorId(id)
}

/// Overrides the CIP serial number of every Execute_PCCC (0x4B) request.
pub fn with_cip_serial_number(sn: u32) -> ClientOption {
    ClientOption::CipSerialNumber(sn)
}

// Implementation detail (not spec): the Go client struct fields that options mutate, split out
// so `connect` can read the logger before the transport exists.
/// Client settings produced by applying [`ClientOption`]s over the defaults.
#[derive(Clone)]
pub struct ClientSettings {
    /// Retry attempts (see [`ClientOption::Retries`]).
    pub retries: i32,
    /// Delay between retries.
    pub retry_delay: Duration,
    /// Logger.
    pub logger: Arc<dyn Logger>,
    /// PCCC requestor vendor ID.
    pub cip_vendor_id: u16,
    /// PCCC requestor serial number.
    pub cip_serial_number: u32,
}

impl Default for ClientSettings {
    fn default() -> Self {
        ClientSettings {
            retries: 0,
            retry_delay: Duration::from_secs(1),
            logger: Arc::new(NopLogger::new()),
            cip_vendor_id: DEFAULT_CIP_VENDOR_ID,
            cip_serial_number: DEFAULT_CIP_SERIAL_NUMBER,
        }
    }
}

impl ClientSettings {
    /// Applies one option.
    pub fn apply(&mut self, opt: &ClientOption) {
        match opt {
            ClientOption::Retries(n) => self.retries = *n,
            ClientOption::RetryDelay(d) => self.retry_delay = *d,
            ClientOption::Logger(l) => self.logger = l.clone(),
            ClientOption::CipVendorId(v) => self.cip_vendor_id = *v,
            ClientOption::CipSerialNumber(s) => self.cip_serial_number = *s,
        }
    }
}

// ---------- mixed option list ----------

// Implementation detail (not spec): Go passes `...any` and type-switches; Rust uses this sum type.
/// One option of any layer, as accepted by `connect` and `new_reconnecting_client`.
pub enum Opt {
    /// A [`ConnOption`].
    Conn(ConnOption),
    /// A [`ClientOption`].
    Client(ClientOption),
    /// A transport lifecycle option.
    Transport(TransportOption),
}

impl From<ConnOption> for Opt {
    fn from(o: ConnOption) -> Self {
        Opt::Conn(o)
    }
}
impl From<ClientOption> for Opt {
    fn from(o: ClientOption) -> Self {
        Opt::Client(o)
    }
}
impl From<TransportOption> for Opt {
    fn from(o: TransportOption) -> Self {
        Opt::Transport(o)
    }
}

/// Routes a mixed option list into its three layers.
pub(crate) fn split_opts(opts: Vec<Opt>) -> (Vec<ConnOption>, Vec<ClientOption>, Vec<TransportOption>) {
    let (mut c, mut cl, mut t) = (Vec::new(), Vec::new(), Vec::new());
    for o in opts {
        match o {
            Opt::Conn(x) => c.push(x),
            Opt::Client(x) => cl.push(x),
            Opt::Transport(x) => t.push(x),
        }
    }
    (c, cl, t)
}
