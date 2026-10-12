//! EtherNet/IP encapsulation transport over a single TCP connection, the
//! crate-level [`Error`] type for the client side, and the hex-dump tracer.
//!
//! Port of Go `conn.go` (plus the `hexdump` package it uses and the error
//! values `ethernetip` produces).

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};

use crate::cip;
use crate::eip::{self, Command, EncapsulationHeader, SessionHandle};
use crate::options::{ConnConfig, ConnOption};
use crate::plc;

// Implementation detail (not spec): client-side error type replacing Go `error`
// values. Messages mirror the Go `fmt.Errorf` strings.
/// Errors from the EtherNet/IP client side.
#[derive(Debug)]
pub enum Error {
    /// Socket I/O failure (dial, read, write, close).
    Io(io::Error),
    /// Encapsulation-layer encode/decode failure.
    Eip(eip::Error),
    /// CIP general-status error returned by the target (Go `cip.Error`). The
    /// client never retries these (Go `cipError` wrapper).
    Cip(cip::Error),
    /// CIP-layer encode/decode failure.
    CipLayer(cip::CipError),
    /// `plc`-layer error (context cancellation, value decoding).
    Plc(plc::Error),
    /// Plain message error (Go `fmt.Errorf` without `%w`).
    Msg(String),
    /// Context message wrapping a cause (Go `fmt.Errorf("...: %w", err)`).
    Wrapped {
        /// Message prefix.
        msg: String,
        /// Underlying cause.
        source: Box<Error>,
    },
}

impl Error {
    /// Creates a plain message error.
    pub fn msg(m: impl Into<String>) -> Error {
        Error::Msg(m.into())
    }

    /// Wraps `source` with a context message (displayed as `msg: source`).
    pub fn wrap(msg: impl Into<String>, source: Error) -> Error {
        Error::Wrapped { msg: msg.into(), source: Box::new(source) }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Eip(e) => write!(f, "{e}"),
            Error::Cip(e) => write!(f, "{e}"),
            Error::CipLayer(e) => write!(f, "{e}"),
            Error::Plc(e) => write!(f, "{e}"),
            Error::Msg(m) => f.write_str(m),
            Error::Wrapped { msg, source } => write!(f, "{msg}: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Eip(e) => Some(e),
            Error::Cip(e) => Some(e),
            Error::CipLayer(e) => Some(e),
            Error::Plc(e) => Some(e),
            Error::Wrapped { source, .. } => Some(source.as_ref()),
            Error::Msg(_) => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}
impl From<eip::Error> for Error {
    fn from(e: eip::Error) -> Self {
        Error::Eip(e)
    }
}
impl From<cip::Error> for Error {
    fn from(e: cip::Error) -> Self {
        Error::Cip(e)
    }
}
impl From<cip::CipError> for Error {
    fn from(e: cip::CipError) -> Self {
        Error::CipLayer(e)
    }
}
impl From<plc::Error> for Error {
    fn from(e: plc::Error) -> Self {
        Error::Plc(e)
    }
}

// Implementation detail: lets the `plc` traits report client errors while
// preserving context cancellation kinds.
impl From<Error> for plc::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Plc(p) => p,
            other => plc::Error::other(other),
        }
    }
}

/// Result alias for the client side.
pub type Result<T> = std::result::Result<T, Error>;

// ---------- hex dump (Go `hexdump` package) ----------

// Implementation detail (not spec): hex dump direction marker (Go `hexdump.Direction`).
/// Whether dumped data was read or written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Data received from the remote side.
    Read,
    /// Data sent to the remote side.
    Write,
}

// Implementation detail (not spec): traditional hex dump tracer. Output is
// serialized by a mutex so concurrent reads and writes produce clean blocks.
/// Writes hex dump traces of wire traffic. Safe for concurrent use.
pub struct HexDumper {
    out: Mutex<Box<dyn Write + Send>>,
}

impl HexDumper {
    /// Creates a dumper writing to `out`.
    pub fn new(out: impl Write + Send + 'static) -> HexDumper {
        HexDumper { out: Mutex::new(Box::new(out)) }
    }

    // Implementation detail: formats and writes one dump block.
    /// Formats `data` in hex dump style and writes it, preceded by the
    /// direction marker and byte count. Write errors are ignored (tracing only).
    pub fn dump(&self, data: &[u8], dir: Direction) {
        let text = format_dump(data, dir);
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        let _ = out.write_all(text.as_bytes());
    }
}

impl fmt::Debug for HexDumper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HexDumper")
    }
}

// Implementation detail (not spec): 16 bytes per line, extra gap after byte 8, ASCII gutter,
// short lines space-padded so the ASCII column aligns.
/// Builds a complete hex dump block.
pub fn format_dump(data: &[u8], dir: Direction) -> String {
    use fmt::Write as _;
    let mut b = String::new();
    match dir {
        Direction::Read => {
            let _ = writeln!(b, "<<< READ {} bytes", data.len());
        }
        Direction::Write => {
            let _ = writeln!(b, ">>> WRITE {} bytes", data.len());
        }
    }
    for (line, chunk) in data.chunks(16).enumerate() {
        let _ = write!(b, "{:08x}  ", line * 16);
        for j in 0..16 {
            if j == 8 {
                b.push(' ');
            }
            match chunk.get(j) {
                Some(c) => {
                    let _ = write!(b, "{c:02x} ");
                }
                None => b.push_str("   "),
            }
        }
        b.push('|');
        for j in 0..16 {
            match chunk.get(j) {
                Some(&c) if (0x20..=0x7e).contains(&c) => b.push(c as char),
                Some(_) => b.push('.'),
                None => b.push(' '),
            }
        }
        b.push('|');
        b.push('\n');
    }
    b
}

// ---------- TCP connection ----------

// Implementation detail (not spec): default EtherNet/IP TCP port.
// Ref: CIP Vol. 2, EtherNet/IP uses TCP/UDP port 0xAF12 = 44818 for explicit messaging
// (section unverified).
const DEFAULT_PORT: &str = ":44818";

// Ref: CIP Vol. 2, Section 2-3 "Encapsulation Message" (header + command-specific data over TCP).
// Implementation detail: TCP socket with serialized writes. Safe for concurrent use.
/// Carries EtherNet/IP encapsulation packets over one TCP connection.
pub struct TcpConn {
    stream: TcpStream,
    hex: Option<Arc<HexDumper>>,
    // Implementation detail: protects concurrent writes (Go `wmu`).
    wmu: Mutex<()>,
}

// Implementation detail: Go `io.ReadFull` semantics: "EOF" if nothing was read, "unexpected
// EOF" if the stream ended part-way. Dumps every successful read when tracing is enabled.
fn read_full(stream: &TcpStream, hex: &Option<Arc<HexDumper>>, buf: &mut [u8]) -> io::Result<()> {
    let mut filled = 0;
    let mut r = stream;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => {
                let msg = if filled == 0 { "EOF" } else { "unexpected EOF" };
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, msg));
            }
            Ok(n) => {
                if let Some(h) = hex {
                    h.dump(&buf[filled..filled + n], Direction::Read);
                }
                filled += n;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

impl TcpConn {
    // Implementation detail (not spec): dial. If no port is given the default 44818 is appended.
    /// Dials a TCP connection to `address` (default port 44818 if none given).
    /// [`ConnOption::Conn`] injects a pre-existing stream and bypasses the dial.
    pub fn new(address: &str, opts: &[ConnOption]) -> Result<TcpConn> {
        let mut cfg = ConnConfig::default();
        for o in opts {
            cfg.apply(o)?;
        }
        let stream = match cfg.conn.take() {
            Some(s) => s,
            None => {
                let addr = if address.contains(':') { address.to_string() } else { format!("{address}{DEFAULT_PORT}") };
                dial(&addr, cfg.dial_timeout)?
            }
        };
        Ok(TcpConn { stream, hex: cfg.hex_dumper, wmu: Mutex::new(()) })
    }

    // Ref: CIP Vol. 2, Section 2-3 "Encapsulation Message": 24-byte header followed by data;
    // sender context and options are zero for requests.
    /// Writes a complete encapsulation packet (24-byte header + data).
    pub fn send(&self, cmd: Command, data: &[u8], session_handle: SessionHandle) -> Result<()> {
        // Go: `Length: uint16(len(data))` (truncating cast, ported as-is).
        let header = EncapsulationHeader {
            command: cmd,
            length: data.len() as u16,
            session_handle,
            status: 0,
            sender_context: [0; 8],
            options: 0,
        };

        let _g = self.wmu.lock().unwrap_or_else(|e| e.into_inner());
        let mut w = &self.stream;

        let hdr = header.bytes();
        w.write_all(&hdr).map_err(|e| Error::wrap("failed to write header", e.into()))?;
        if let Some(h) = &self.hex {
            h.dump(&hdr, Direction::Write);
        }

        if !data.is_empty() {
            w.write_all(data).map_err(|e| Error::wrap("failed to write data", e.into()))?;
            if let Some(h) = &self.hex {
                h.dump(data, Direction::Write);
            }
        }
        Ok(())
    }

    // Ref: CIP Vol. 2, Section 2-3 "Encapsulation Message": Length field gives the size of the
    // command-specific data following the header (16-bit, so allocation is bounded to 64 KiB).
    /// Reads a complete encapsulation packet and returns the header and data payload.
    pub fn receive(&self) -> Result<(EncapsulationHeader, Vec<u8>)> {
        let mut hb = [0u8; eip::HEADER_SIZE];
        read_full(&self.stream, &self.hex, &mut hb)
            .map_err(|e| Error::wrap("failed to read header", e.into()))?;
        let header = EncapsulationHeader::from_bytes(&hb)
            .map_err(|e| Error::wrap("failed to read header", e.into()))?;

        let mut data = Vec::new();
        if header.length > 0 {
            data = vec![0u8; header.length as usize];
            read_full(&self.stream, &self.hex, &mut data)
                .map_err(|e| Error::wrap("failed to read data", e.into()))?;
        }
        Ok((header, data))
    }

    // Implementation detail: remote socket address (Go `conn.RemoteAddr()`), used by the I/O
    // scanner to derive the UDP target.
    /// Returns the remote address of the TCP connection, if known.
    pub fn peer_addr(&self) -> Option<std::net::SocketAddr> {
        self.stream.peer_addr().ok()
    }

    // Implementation detail: shutting down both directions also unblocks a reader in another thread.
    /// Closes the underlying TCP connection.
    pub fn close(&self) -> Result<()> {
        self.stream.shutdown(std::net::Shutdown::Both).map_err(Error::from)
    }
}

// Implementation detail: resolves `addr` and tries each candidate with the dial timeout
// (Go `net.DialTimeout`).
fn dial(addr: &str, timeout: std::time::Duration) -> Result<TcpStream> {
    let mut last: Option<io::Error> = None;
    for sa in addr.to_socket_addrs()? {
        match TcpStream::connect_timeout(&sa, timeout) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(last
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no addresses resolved"))
        .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_dump_layout() {
        let s = format_dump(b"AB\x00", Direction::Write);
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines[0], ">>> WRITE 3 bytes");
        assert!(lines[1].starts_with("00000000  41 42 00 "));
        assert!(lines[1].ends_with("|AB.             |"));
        let r = format_dump(&[], Direction::Read);
        assert_eq!(r, "<<< READ 0 bytes\n");
    }

    #[test]
    fn error_display_wrapping() {
        let e = Error::wrap("failed to read header", Error::msg("EOF"));
        assert_eq!(e.to_string(), "failed to read header: EOF");
    }
}
