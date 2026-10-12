use super::cpf::ITEM_ID_LIST_IDENTITY;
use super::wire;
use super::{Error, Result};

// Ref: CIP Vol. 2, ListIdentity response "CIP Identity Item" (type 0x000C) (section unverified):
// Encapsulation Protocol Version, Socket Address (sockaddr_in), Vendor ID, Device Type,
// Product Code, Revision, Status, Serial Number, Product Name (SHORT_STRING), State.
/// Device identity returned by ListIdentity.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListIdentityItem {
    pub type_id: u16,
    pub length: u16,
    pub encaps_version: u16,
    /// struct sockaddr_in (big-endian family/port/addr per CPF sockaddr convention).
    pub socket_addr: [u8; 16],
    pub vendor_id: u16,
    pub device_type: u16,
    pub product_code: u16,
    /// Major, Minor.
    pub revision: [u8; 2],
    pub status: u16,
    pub serial_number: u32,
    /// Max 32 chars on encode.
    pub product_name: String,
    pub state: u8,
}

// Ref: CIP Vol. 2, ListServices response "Communications Service" item (type 0x0100)
// (section unverified): Version, Capability Flags, Name of Service (16 bytes).
/// Communication service advertised by ListServices.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListServicesItem {
    pub type_id: u16,
    pub length: u16,
    pub version: u16,
    pub capability_flags: u16,
    /// 16 bytes fixed on the wire.
    pub name: String,
}

// Ref: CIP Vol. 2, ListIdentity reply: item count (UINT) followed by CIP Identity items.
/// Encodes identity items into a ListIdentity response body.
pub fn encode_list_identity_response(items: &[ListIdentityItem]) -> Result<Vec<u8>> {
    if items.len() > u16::MAX as usize {
        return Err(Error::Cpf(format!("discovery: too many items ({})", items.len())));
    }
    let mut buf = Vec::new();
    buf.extend_from_slice(&(items.len() as u16).to_le_bytes());
    for item in items {
        let nb = item.product_name.as_bytes();
        let nb = &nb[..nb.len().min(32)];
        buf.extend_from_slice(&item.type_id.to_le_bytes());
        // Length: EncapsVersion(2) + SocketAddr(16) + VendorID(2) + DeviceType(2) +
        // ProductCode(2) + Revision(2) + Status(2) + SerialNumber(4) + NameLen(1) + Name(n)
        // + State(1) = 34 + len(name)
        buf.extend_from_slice(&((34 + nb.len()) as u16).to_le_bytes());
        buf.extend_from_slice(&item.encaps_version.to_le_bytes());
        buf.extend_from_slice(&item.socket_addr);
        buf.extend_from_slice(&item.vendor_id.to_le_bytes());
        buf.extend_from_slice(&item.device_type.to_le_bytes());
        buf.extend_from_slice(&item.product_code.to_le_bytes());
        buf.extend_from_slice(&item.revision);
        buf.extend_from_slice(&item.status.to_le_bytes());
        buf.extend_from_slice(&item.serial_number.to_le_bytes());
        buf.push(nb.len() as u8);
        buf.extend_from_slice(nb);
        buf.push(item.state);
    }
    Ok(buf)
}

// Ref: CIP Vol. 2, ListServices reply: item count (UINT) followed by service items.
/// Encodes service items into a ListServices response body.
pub fn encode_list_services_response(items: &[ListServicesItem]) -> Result<Vec<u8>> {
    if items.len() > u16::MAX as usize {
        return Err(Error::Cpf(format!("discovery: too many items ({})", items.len())));
    }
    let mut buf = Vec::new();
    buf.extend_from_slice(&(items.len() as u16).to_le_bytes());
    for item in items {
        buf.extend_from_slice(&item.type_id.to_le_bytes());
        // Length: Version(2) + Flags(2) + Name(16)
        buf.extend_from_slice(&20u16.to_le_bytes());
        buf.extend_from_slice(&item.version.to_le_bytes());
        buf.extend_from_slice(&item.capability_flags.to_le_bytes());
        let mut name = [0u8; 16];
        let nb = item.name.as_bytes();
        let n = nb.len().min(16);
        name[..n].copy_from_slice(&nb[..n]);
        buf.extend_from_slice(&name);
    }
    Ok(buf)
}

// Ref: CIP Vol. 2, ListServices "Communications Service" item layout (section unverified).
/// Decodes a single service item from the cursor, advancing it.
pub fn decode_list_services_item(r: &mut &[u8]) -> Result<ListServicesItem> {
    let type_id = wire::u16_(r)?;
    let length = wire::u16_(r)?;
    let version = wire::u16_(r)?;
    let capability_flags = wire::u16_(r)?;
    let nb = wire::arr::<16>(r)?;
    // Trim trailing null bytes
    let end = nb.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    Ok(ListServicesItem {
        type_id,
        length,
        version,
        capability_flags,
        name: String::from_utf8_lossy(&nb[..end]).into_owned(),
    })
}

// Ref: CIP Vol. 2, ListIdentity reply / CIP Identity Item layout (section unverified).
// Untrusted input: all reads checked; capacity bounded by input size, not the wire count.
/// Decodes a ListIdentity response body; unknown item types are skipped.
pub fn decode_list_identity_response(data: &[u8]) -> Result<Vec<ListIdentityItem>> {
    let mut r = data;
    let count = wire::u16_(&mut r)?;
    let mut items = Vec::with_capacity((count as usize).min(data.len() / 4));
    for _ in 0..count {
        let type_id = wire::u16_(&mut r)?;
        let length = wire::u16_(&mut r)?;
        if type_id == ITEM_ID_LIST_IDENTITY {
            let encaps_version = wire::u16_(&mut r)?;
            let socket_addr = wire::arr::<16>(&mut r)?;
            let vendor_id = wire::u16_(&mut r)?;
            let device_type = wire::u16_(&mut r)?;
            let product_code = wire::u16_(&mut r)?;
            let revision = wire::arr::<2>(&mut r)?;
            let status = wire::u16_(&mut r)?;
            let serial_number = wire::u32_(&mut r)?;
            // ProductName is a length-prefixed string (1 byte length)
            let name_len = wire::u8_(&mut r)? as usize;
            let nb = wire::bytes(&mut r, name_len)?;
            let state = wire::u8_(&mut r)?;
            items.push(ListIdentityItem {
                type_id,
                length,
                encaps_version,
                socket_addr,
                vendor_id,
                device_type,
                product_code,
                revision,
                status,
                serial_number,
                product_name: String::from_utf8_lossy(&nb).into_owned(),
                state,
            });
        } else {
            // Unknown item type, skip data
            wire::bytes(&mut r, length as usize)?;
        }
    }
    Ok(items)
}

// Ref: CIP Vol. 2, ListServices reply: item count (UINT) followed by service items.
/// Decodes a ListServices response body.
pub fn decode_list_services_response(data: &[u8]) -> Result<Vec<ListServicesItem>> {
    let mut r = data;
    let count = wire::u16_(&mut r)?;
    let mut items = Vec::with_capacity((count as usize).min(data.len() / 4));
    for _ in 0..count {
        items.push(decode_list_services_item(&mut r)?);
    }
    Ok(items)
}
