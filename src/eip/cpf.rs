use super::wire;
use super::{Error, Result};
use std::io::Write;

// Ref: CIP Vol. 2, Section 2-6 "Common Packet Format" (section number from knowledge; verify),
// Common Packet Format item type IDs (table number unverified).
// Ref: CIP Vol. 2, CPF: Null Address Item (0x0000).
pub const ITEM_ID_NULL_ADDRESS: u16 = 0x0000;
// Ref: CIP Vol. 2, CPF: List Identity response item (0x000C).
pub const ITEM_ID_LIST_IDENTITY: u16 = 0x000C;
// Ref: CIP Vol. 2, CPF: Connected Address Item (0x00A1).
pub const ITEM_ID_CONNECTION_BASED: u16 = 0x00A1;
// Ref: CIP Vol. 2, CPF: Connected Address Item (0x00A1); alias of ConnectionBased.
pub const ITEM_ID_CONNECTED_ADDRESS: u16 = 0x00A1;
// Ref: CIP Vol. 2, CPF: Connected Data Item (0x00B1).
pub const ITEM_ID_CONNECTED_TRANSPORT: u16 = 0x00B1;
// Ref: CIP Vol. 2, CPF: Connected Data Item (0x00B1); alias of ConnectedTransport.
pub const ITEM_ID_CONNECTED_DATA: u16 = 0x00B1;
// Ref: CIP Vol. 2, CPF: Unconnected Data Item (0x00B2).
pub const ITEM_ID_UNCONNECTED_MESSAGE: u16 = 0x00B2;
// Ref: CIP Vol. 2, CPF: ListServices response item (0x0100).
pub const ITEM_ID_LIST_SERVICES: u16 = 0x0100;
// Ref: CIP Vol. 2, CPF: Sockaddr Info item, O->T (0x8000).
pub const ITEM_ID_SOCKADDR_INFO: u16 = 0x8000;
// Ref: CIP Vol. 2, CPF: Sequenced Address Item (0x8002).
pub const ITEM_ID_SEQUENCED_ADDRESS: u16 = 0x8002;

// Ref: CIP Vol. 2, Section 2-6 CPF item: Type ID (UINT), Length (UINT), Data.
/// A single Common Packet Format item.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CpfItem {
    pub type_id: u16,
    pub length: u16,
    pub data: Vec<u8>,
}

impl CpfItem {
    /// Creates an item; `length` is set from `data.len()` (truncated to u16 like the Go cast).
    pub fn new(type_id: u16, data: impl Into<Vec<u8>>) -> Self {
        let data = data.into();
        Self { type_id, length: data.len() as u16, data }
    }

    // Ref: CIP Vol. 2, Section 2-6 CPF item layout, little-endian.
    /// Writes type ID, length, and data (data only when `length > 0`, as in Go).
    pub fn encode<W: Write>(&self, w: &mut W) -> Result<()> {
        // Implementation detail (not spec): refuse to emit a frame whose declared length does
        // not match the payload (Go silently truncated the length, corrupting the frame).
        if self.data.len() != self.length as usize {
            return Err(Error::Cpf(format!(
                "cpf: item length {} does not match data length {} (max {})",
                self.length,
                self.data.len(),
                u16::MAX
            )));
        }
        w.write_all(&self.type_id.to_le_bytes())?;
        w.write_all(&self.length.to_le_bytes())?;
        if self.length > 0 {
            w.write_all(&self.data)?;
        }
        Ok(())
    }
}

// Ref: CIP Vol. 2, Section 2-6 Common Packet Format: Item Count (UINT) followed by items.
/// Collection of CPF items carried in SendRRData / SendUnitData.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommonPacketFormat {
    pub item_count: u16,
    pub items: Vec<CpfItem>,
}

impl CommonPacketFormat {
    /// Creates a CPF with `item_count` set from the items.
    pub fn new(items: Vec<CpfItem>) -> Self {
        Self { item_count: items.len() as u16, items }
    }

    // Ref: CIP Vol. 2, Section 2-6 Common Packet Format layout.
    /// Serializes the item count followed by each item.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.items.len() > u16::MAX as usize || self.items.len() != self.item_count as usize {
            return Err(Error::Cpf(format!(
                "cpf: item count {} does not match {} items (max {})",
                self.item_count,
                self.items.len(),
                u16::MAX
            )));
        }
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.item_count.to_le_bytes());
        for item in &self.items {
            item.encode(&mut buf)?;
        }
        Ok(buf)
    }

    // Ref: CIP Vol. 2, Section 2-6 Common Packet Format layout. Untrusted input: the declared
    // item count is validated against remaining bytes (>= 4 per item) and every read is checked.
    /// Decodes a CPF from bytes.
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = data;
        let item_count = wire::u16_(&mut r)?;
        let remaining = r.len();
        let need = 4 * item_count as usize;
        if remaining < need {
            return Err(Error::Cpf(format!(
                "cpf: item count {item_count} requires at least {need} bytes but only {remaining} remain"
            )));
        }
        let mut items = Vec::new();
        for _ in 0..item_count {
            let type_id = wire::u16_(&mut r)?;
            let length = wire::u16_(&mut r)?;
            let d = wire::bytes(&mut r, length as usize)?;
            items.push(CpfItem { type_id, length, data: d });
        }
        Ok(Self { item_count, items })
    }

    /// First item with the given type ID, if any.
    pub fn find_item_by_type(&self, type_id: u16) -> Option<&CpfItem> {
        self.items.iter().find(|i| i.type_id == type_id)
    }
}

/// Go-style free function alias for [`CommonPacketFormat::decode`].
pub fn decode_common_packet_format(data: &[u8]) -> Result<CommonPacketFormat> {
    CommonPacketFormat::decode(data)
}
