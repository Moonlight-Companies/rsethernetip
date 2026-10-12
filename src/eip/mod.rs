// Ref: ODVA EtherNet/IP Adaptation of CIP (CIP Vol. 2), Chapter 2 "Encapsulation Protocol"
// (chapter number per the Go source docs; section numbers cited per item below).
//! EtherNet/IP encapsulation layer: header, commands, CPF framing, discovery.
//! All multi-byte fields are little-endian per the EtherNet/IP specification.

pub mod header;
pub mod commands;
pub mod cpf;
pub mod discovery;
mod wire;

pub use commands::*;
pub use cpf::*;
pub use discovery::*;
pub use header::*;

use std::fmt;

// Implementation detail (not spec): error type for the eip layer. Replaces the Go
// `error` values (io errors and `cpf: ...` fmt.Errorf messages).
/// Errors produced by encoding/decoding encapsulation structures.
#[derive(Debug)]
pub enum Error {
    /// Underlying I/O error; short reads surface as `UnexpectedEof` (Go io.ErrUnexpectedEOF / io.EOF).
    Io(std::io::Error),
    /// CPF structural error (message mirrors the Go `cpf: ...` text).
    Cpf(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Cpf(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Result alias for the eip layer.
pub type Result<T> = std::result::Result<T, Error>;

// Implementation detail: ported tests from Go eip_test.go (plus discovery coverage).
#[cfg(test)]
mod tests;
