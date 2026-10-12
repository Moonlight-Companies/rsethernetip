//! PCCC (Programmable Controller Communication Commands) application protocol
//! used by Allen-Bradley SLC 500, MicroLogix and PLC-5 controllers.
//!
//! Port of Go `pccc/`. Only the framing layer plus a high-level client are
//! implemented: encoding PCCC command packets, decoding replies and
//! translating PCCC STS / EXT STS codes into [`Error`] values. On EtherNet/IP a
//! PCCC command is tunnelled in the CIP Execute_PCCC service (class 0x67,
//! service 0x4B); the wrapper that puts a frame on the wire lives in the
//! top-level client and is reached through the [`client::Executor`] trait.
//!
//! The SLC typed read (FNC 0xA2) and typed write (FNC 0xAA) commands cover
//! SLC 5/0x and MicroLogix data-table access. PLC-5 word-range commands are
//! out of scope. All multi-byte values are little-endian.
//!
//! Ref: Allen-Bradley PCCC / DF1 Protocol and Command Set Reference Manual,
//! publication 1770-6.5.16.

pub mod address;
pub mod client;
pub mod datapoint;
pub mod frame;
pub mod timer;

pub use address::{Address, parse_address};
pub use client::{Client, Connector, Executor, WriteValue};
pub use datapoint::File;
pub use frame::{
    CMD_PROTECTED_TYPED_LOGICAL, Error, FUNC_PROTECTED_TYPED_LOGICAL_READ,
    FUNC_PROTECTED_TYPED_LOGICAL_WRITE, FileType, REPLY_BIT, Reply, STATUS_USE_EXT_STS,
    decode_reply, encode_typed_read, encode_typed_write, file_type_from_letter, is_pccc_error,
    status_message,
};
pub use timer::{Counter, Timer, decode_counter, decode_timer};

use std::fmt;

// Implementation detail (not spec): layer error type replacing Go's `error`
// values returned by the pccc package. `Status` carries the PCCC protocol
// error (Go `*pccc.Error`); `Message` carries the Go `fmt.Errorf` strings;
// `Exec` wraps whatever the underlying transport returned.
/// Error returned by the `pccc` layer.
#[derive(Debug)]
pub enum PcccError {
    /// A PCCC protocol error: non-zero STS (Go `*pccc.Error`).
    Status(Error),
    /// A validation / framing / parse error, with the Go message text.
    Message(String),
    /// An error from the underlying [`Executor`] (transport or CIP layer).
    Exec(Box<dyn std::error::Error + Send + Sync>),
}

impl PcccError {
    // Implementation detail (not spec): convenience constructor.
    pub(crate) fn msg(s: impl Into<String>) -> PcccError {
        PcccError::Message(s.into())
    }
}

impl fmt::Display for PcccError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PcccError::Status(e) => e.fmt(f),
            PcccError::Message(s) => f.write_str(s),
            PcccError::Exec(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for PcccError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PcccError::Status(e) => Some(e),
            PcccError::Message(_) => None,
            PcccError::Exec(e) => Some(e.as_ref()),
        }
    }
}

impl From<Error> for PcccError {
    fn from(e: Error) -> PcccError {
        PcccError::Status(e)
    }
}

// Implementation detail (not spec): layer result alias.
/// Result alias for the `pccc` layer.
pub type Result<T> = std::result::Result<T, PcccError>;
