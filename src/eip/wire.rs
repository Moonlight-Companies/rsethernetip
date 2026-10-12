// Implementation detail (not spec): little-endian read helpers over a byte slice cursor.
use super::Result;
use std::io::Read;

pub(crate) fn u8_(r: &mut &[u8]) -> Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

pub(crate) fn u16_(r: &mut &[u8]) -> Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}

pub(crate) fn u32_(r: &mut &[u8]) -> Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

pub(crate) fn arr<const N: usize>(r: &mut &[u8]) -> Result<[u8; N]> {
    let mut b = [0u8; N];
    r.read_exact(&mut b)?;
    Ok(b)
}

/// Reads exactly `n` bytes (checked against remaining input; no wire-length preallocation).
pub(crate) fn bytes(r: &mut &[u8], n: usize) -> Result<Vec<u8>> {
    if r.len() < n {
        return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
    }
    let (a, b) = r.split_at(n);
    *r = b;
    Ok(a.to_vec())
}
