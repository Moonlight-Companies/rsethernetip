//! Rockwell Logix COUNTER structure.
//!
//! Port of Go `cip/counter.go`.

use super::decode::Unmarshaler;
use super::encode::Marshaler;
use super::types::*;

// Ref: Rockwell Logix 5000 Controllers General Instructions Reference Manual (1756-RM003) -
// COUNTER structure (CTU/CTD/CTUD) with PRE, ACC and CU/CD/DN/OV/UN status bits; wire layout as
// stored in PLC memory per Rockwell 1756-PM020 Logix 5000 Data Access (vendor-specific
// structure; layout bit positions per Go source).
//
// Two wire layouts are supported:
//
// 14-byte layout (older firmware / ControlLogix):
//   Offset 0-1:   Reserved (INT)
//   Offset 2-5:   Status bits (DINT) - CU, CD, DN, OV, UN packed in the high bits
//   Offset 6-9:   PRE (DINT) - preset value
//   Offset 10-13: ACC (DINT) - accumulated count
//
// 12-byte layout (newer firmware / CompactLogix):
//   Offset 0-3:   Status bits (DINT)
//   Offset 4-7:   PRE (DINT)
//   Offset 8-11:  ACC (DINT)
//
// Use [`decode_counter`] or `Counter::unmarshal_cip` to decode from raw bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Counter {
    // Preset
    pub pre: i32,
    // Accumulated
    pub acc: i32,
    // Count Up
    pub cu: bool,
    // Count Down
    pub cd: bool,
    // Done
    pub dn: bool,
    // Overflow
    pub ov: bool,
    // Underflow
    pub un: bool,
}

// Ref: Rockwell 1756-RM003 COUNTER structure - CU is bit 31 of the status DINT (count up enabled).
pub const COUNTER_STATUS_CU: u32 = 31;
// Ref: Rockwell 1756-RM003 COUNTER structure - CD is bit 30 of the status DINT (count down enabled).
pub const COUNTER_STATUS_CD: u32 = 30;
// Ref: Rockwell 1756-RM003 COUNTER structure - DN is bit 29 of the status DINT (ACC >= PRE).
pub const COUNTER_STATUS_DN: u32 = 29;
// Ref: Rockwell 1756-RM003 COUNTER structure - OV is bit 28 of the status DINT (overflow).
pub const COUNTER_STATUS_OV: u32 = 28;
// Ref: Rockwell 1756-RM003 COUNTER structure - UN is bit 27 of the status DINT (underflow).
pub const COUNTER_STATUS_UN: u32 = 27;

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - COUNTER memory layout (see Counter docs).
// Decodes a byte slice into a Counter. Accepts 12 bytes (status + PRE + ACC) or 14 bytes
// (reserved + status + PRE + ACC); more than 14 bytes uses the 14-byte layout.
pub fn decode_counter(data: &[u8]) -> Result<Counter> {
    let (status_off, pre_off, acc_off) = if data.len() >= 14 {
        // 14-byte layout: reserved(2) + status(4) + PRE(4) + ACC(4)
        (2, 6, 10)
    } else if data.len() >= 12 {
        // 12-byte layout: status(4) + PRE(4) + ACC(4)
        (0, 4, 8)
    } else {
        return Err(CipError::InsufficientData { kind: "Counter", expected: 12, got: data.len() });
    };

    let word = |off: usize| u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
    let status = word(status_off);
    let pre = word(pre_off) as i32;
    let acc = word(acc_off) as i32;

    Ok(Counter {
        pre,
        acc,
        cu: (status & (1 << COUNTER_STATUS_CU)) != 0,
        cd: (status & (1 << COUNTER_STATUS_CD)) != 0,
        dn: (status & (1 << COUNTER_STATUS_DN)) != 0,
        ov: (status & (1 << COUNTER_STATUS_OV)) != 0,
        un: (status & (1 << COUNTER_STATUS_UN)) != 0,
    })
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - COUNTER memory layout.
impl Unmarshaler for Counter {
    fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
        *self = decode_counter(data)?;
        Ok(())
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - encodes the canonical 14-byte memory layout:
// reserved INT (0), status DINT, PRE DINT, ACC DINT.
impl Marshaler for Counter {
    fn marshal_cip(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(14);

        // Reserved
        buf.extend_from_slice(&0u16.to_le_bytes());

        // Status Bits
        let mut status: u32 = 0;
        if self.cu {
            status |= 1 << COUNTER_STATUS_CU;
        }
        if self.cd {
            status |= 1 << COUNTER_STATUS_CD;
        }
        if self.dn {
            status |= 1 << COUNTER_STATUS_DN;
        }
        if self.ov {
            status |= 1 << COUNTER_STATUS_OV;
        }
        if self.un {
            status |= 1 << COUNTER_STATUS_UN;
        }
        buf.extend_from_slice(&status.to_le_bytes());

        // PRE
        buf.extend_from_slice(&self.pre.to_le_bytes());

        // ACC
        buf.extend_from_slice(&self.acc.to_le_bytes());

        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ref: Rockwell 1756-RM003 COUNTER structure status bits.
    #[test]
    fn counter_all_bits_set() {
        let mut data = vec![0u8; 14];
        data[0..2].copy_from_slice(&0u16.to_le_bytes());
        let status_bits: u32 = (1 << COUNTER_STATUS_CU)
            | (1 << COUNTER_STATUS_CD)
            | (1 << COUNTER_STATUS_DN)
            | (1 << COUNTER_STATUS_OV)
            | (1 << COUNTER_STATUS_UN);
        data[2..6].copy_from_slice(&status_bits.to_le_bytes());
        data[6..10].copy_from_slice(&500u32.to_le_bytes());
        data[10..14].copy_from_slice(&501u32.to_le_bytes());

        let counter = decode_counter(&data).unwrap();
        assert!(counter.cu && counter.cd && counter.dn && counter.ov && counter.un, "expected all counter bits true");
        assert!(counter.pre == 500 && counter.acc == 501, "PRE={} ACC={}, want 500/501", counter.pre, counter.acc);
    }

    #[test]
    fn counter_decode_short_data() {
        assert!(decode_counter(&[0u8; 5]).is_err(), "expected error for short data");
    }

    #[test]
    fn counter_marshal_round_trip() {
        let original = Counter { pre: 200, acc: 150, cu: true, cd: false, dn: true, ov: false, un: true };
        let data = original.marshal_cip().unwrap();

        let decoded = decode_counter(&data).unwrap();
        assert!(decoded.pre == original.pre && decoded.acc == original.acc, "PRE/ACC mismatch: {:?} vs {:?}", decoded, original);
        assert!(
            decoded.cu == original.cu
                && decoded.cd == original.cd
                && decoded.dn == original.dn
                && decoded.ov == original.ov
                && decoded.un == original.un,
            "status mismatch: decoded={:?} original={:?}",
            decoded,
            original
        );
    }

    // Added: 12-byte layout and error text (Go behavior).
    #[test]
    fn counter_12_byte_layout() {
        let mut d = vec![0u8; 12];
        d[0..4].copy_from_slice(&(1u32 << COUNTER_STATUS_OV).to_le_bytes());
        d[4..8].copy_from_slice(&10i32.to_le_bytes());
        d[8..12].copy_from_slice(&11i32.to_le_bytes());
        let c = decode_counter(&d).unwrap();
        assert_eq!(c, Counter { pre: 10, acc: 11, ov: true, ..Default::default() });
        assert_eq!(
            decode_counter(&[0u8; 2]).unwrap_err().to_string(),
            "insufficient data for Counter: expected at least 12 bytes, got 2"
        );
    }
}
