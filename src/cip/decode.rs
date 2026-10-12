//! Unmarshal: little-endian CIP wire bytes to Rust values.
//!
//! Port of Go `cip/decode.go`. Go uses reflection (`binary.Read`) for built-in types; in Rust
//! the same behavior is provided by [`Unmarshaler`] implementations for the fixed-size
//! primitive types and for slices/arrays/Vecs of them (existing length is filled).

use super::types::*;

// Ref: ODVA CIP Vol. 1, Appendix C "Data Management" - little-endian wire encoding (section
// unverified).
// Implemented by types that can unmarshal a CIP binary representation of themselves. Custom
// implementations (e.g. Rockwell structures) correspond to Go's `UnmarshalCIP`; the built-in
// impls below correspond to the default `binary.Read` decoding.
pub trait Unmarshaler {
    fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()>;
}

// Ref: CIP Vol. 1, Appendix C - little-endian encoding (section unverified).
// Parses CIP-encoded data and stores the result in `v`. Go's "non-pointer / nil pointer" errors
// cannot happen here (a `&mut T` is always valid).
pub fn unmarshal<T: Unmarshaler + ?Sized>(data: &[u8], v: &mut T) -> Result<()> {
    v.unmarshal_cip(data)
}

// Implementation detail (mirrors Go's io.EOF / io.ErrUnexpectedEOF from binary.Read): checks
// that `data` holds at least `n` bytes, wrapping the failure like "cip: binary.Read failed".
fn need(data: &[u8], n: usize) -> Result<()> {
    if data.len() >= n {
        return Ok(());
    }
    let inner = if data.is_empty() { CipError::Eof } else { CipError::UnexpectedEof };
    Err(CipError::BinaryRead(Box::new(inner)))
}

// Ref: CIP Vol. 1, Appendix C - BOOL: any non-zero byte is true (section unverified).
impl Unmarshaler for bool {
    fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
        need(data, 1)?;
        *self = data[0] != 0;
        Ok(())
    }
}

// Ref: CIP Vol. 1, Appendix C - fixed-size little-endian elementary types (section unverified).
macro_rules! impl_unmarshaler_num {
    ($($t:ty),*) => {$(
        impl Unmarshaler for $t {
            fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
                const N: usize = std::mem::size_of::<$t>();
                need(data, N)?;
                let mut b = [0u8; N];
                b.copy_from_slice(&data[..N]);
                *self = <$t>::from_le_bytes(b);
                Ok(())
            }
        }
    )*};
}
impl_unmarshaler_num!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64);

// Implementation detail (mirrors binary.Read into a pre-sized slice/array): each existing
// element is filled in order; short input is an EOF / unexpected EOF error.
impl<T: Unmarshaler + SizedCip> Unmarshaler for [T] {
    fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
        need(data, self.len() * T::CIP_SIZE)?;
        for (i, e) in self.iter_mut().enumerate() {
            e.unmarshal_cip(&data[i * T::CIP_SIZE..])?;
        }
        Ok(())
    }
}

impl<T: Unmarshaler + SizedCip, const N: usize> Unmarshaler for [T; N] {
    fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
        self.as_mut_slice().unmarshal_cip(data)
    }
}

impl<T: Unmarshaler + SizedCip> Unmarshaler for Vec<T> {
    fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
        self.as_mut_slice().unmarshal_cip(data)
    }
}

// Implementation detail: fixed wire size of a primitive element, needed to step through slices.
pub trait SizedCip {
    const CIP_SIZE: usize;
}

macro_rules! impl_sized {
    ($($t:ty),*) => {$(
        impl SizedCip for $t {
            const CIP_SIZE: usize = std::mem::size_of::<$t>();
        }
    )*};
}
impl_sized!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64);

impl SizedCip for bool {
    const CIP_SIZE: usize = 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== Unmarshal Error Cases =====
    // Go TestUnmarshalNonPointer / TestUnmarshalNilPointer have no Rust equivalent (a `&mut T`
    // is always a valid non-nil pointer). The nearest failure mode, short input, is covered here.

    #[test]
    fn unmarshal_short_input() {
        let mut v: i32 = 0;
        let e = unmarshal(&[0x01], &mut v).unwrap_err();
        assert_eq!(e.to_string(), "cip: binary.Read failed: unexpected EOF");
        let e = unmarshal(&[], &mut v).unwrap_err();
        assert_eq!(e.to_string(), "cip: binary.Read failed: EOF");
    }

    // --- cpppo vectors ---

    // b'\x01\x00' -> INT value 1 (little-endian)
    #[test]
    fn encode_decode_int_little_endian() {
        let mut out: i16 = 0;
        unmarshal(&[0x01, 0x00], &mut out).unwrap();
        assert_eq!(out, 1);
    }

    // 4 x REAL all zeros -> 0.0
    #[test]
    fn encode_decode_real_zero() {
        let mut out: f32 = 1.0;
        unmarshal(&[0, 0, 0, 0], &mut out).unwrap();
        assert_eq!(out, 0.0);
    }

    // BOOL truthiness: any non-zero byte is true.
    #[test]
    fn bool_truthiness() {
        let cases: [(u8, bool); 10] = [
            (0x00, false),
            (0x01, true),
            (0x02, true),
            (0x04, true),
            (0x08, true),
            (0x10, true),
            (0x20, true),
            (0x40, true),
            (0x80, true),
            (0xFF, true),
        ];
        for (raw, want) in cases {
            let mut out = false;
            unmarshal(&[raw], &mut out).unwrap();
            assert_eq!(out, want, "BOOL 0x{:02X}", raw);
        }
    }

    // Slice fill (binary.Read into a pre-sized slice; added).
    #[test]
    fn unmarshal_slice() {
        let mut v = [0u16; 2];
        unmarshal(&[1, 0, 2, 0, 9, 9], &mut v).unwrap();
        assert_eq!(v, [1, 2]);
        assert!(unmarshal(&[1, 0, 2], &mut v).is_err());
    }
}
