use super::wire;
use super::{Command, Result};
use std::fmt;
use std::io::{Read, Write};

// Ref: CIP Vol. 2, Section 2-3 "Encapsulation Message": fixed 24-byte header
// (section number from knowledge; verify).
/// Fixed size in bytes of an encapsulation header.
pub const HEADER_SIZE: usize = 24;

// Ref: CIP Vol. 2, Ch. 2 Session Handle field (UDINT) assigned by the target on RegisterSession.
/// 32-bit session handle assigned by the target during session registration.
pub type SessionHandle = u32;

// Ref: CIP Vol. 2, Section 2-3 "Encapsulation Message": Command(UINT), Length(UINT),
// Session Handle(UDINT), Status(UDINT), Sender Context(8 bytes), Options(UDINT).
/// The 24-byte encapsulation header that prefixes every EIP message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EncapsulationHeader {
    pub command: Command,
    /// Length of the data following the header.
    pub length: u16,
    pub session_handle: SessionHandle,
    pub status: u32,
    pub sender_context: [u8; 8],
    pub options: u32,
}

impl EncapsulationHeader {
    // Ref: CIP Vol. 2, Section 2-3 header layout, little-endian.
    /// Writes the 24-byte header to `w`.
    pub fn encode<W: Write>(&self, w: &mut W) -> Result<()> {
        w.write_all(&self.bytes())?;
        Ok(())
    }

    // Ref: CIP Vol. 2, Section 2-3 header layout, little-endian.
    /// Reads and parses a 24-byte header from `r` (short input yields an I/O UnexpectedEof error).
    pub fn decode<R: Read>(r: &mut R) -> Result<Self> {
        let mut b = [0u8; HEADER_SIZE];
        r.read_exact(&mut b)?;
        Self::from_bytes(&b)
    }

    /// Parses a header from the start of a byte slice (checked; errors if shorter than 24 bytes).
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = data;
        Ok(Self {
            command: Command(wire::u16_(&mut r)?),
            length: wire::u16_(&mut r)?,
            session_handle: wire::u32_(&mut r)?,
            status: wire::u32_(&mut r)?,
            sender_context: wire::arr::<8>(&mut r)?,
            options: wire::u32_(&mut r)?,
        })
    }

    // Ref: CIP Vol. 2, Section 2-3 header layout, little-endian.
    /// Returns the header serialized as 24 bytes.
    pub fn bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(HEADER_SIZE);
        v.extend_from_slice(&self.command.0.to_le_bytes());
        v.extend_from_slice(&self.length.to_le_bytes());
        v.extend_from_slice(&self.session_handle.to_le_bytes());
        v.extend_from_slice(&self.status.to_le_bytes());
        v.extend_from_slice(&self.sender_context);
        v.extend_from_slice(&self.options.to_le_bytes());
        v
    }
}

// Implementation detail (not spec): human-readable form, same text as Go String().
impl fmt::Display for EncapsulationHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Cmd: {} (0x{:04X}), Len: {}, Session: 0x{:08X}, Status: 0x{:08X}",
            self.command, self.command.0, self.length, self.session_handle, self.status
        )
    }
}
