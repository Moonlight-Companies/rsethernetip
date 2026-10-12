//! Rockwell Logix TIMER structure.
//!
//! Port of Go `cip/timer.go`.

use super::decode::Unmarshaler;
use super::encode::Marshaler;
use super::types::*;

// Ref: Rockwell Logix 5000 Controllers General Instructions Reference Manual (1756-RM003) -
// TIMER structure (TON/TOF/RTO) with PRE, ACC and EN/TT/DN status bits; wire layout as stored in
// PLC memory per Rockwell 1756-PM020 Logix 5000 Data Access (vendor-specific structure; layout
// bit positions per Go source).
//
// Two wire layouts are supported:
//
// 14-byte layout (older firmware / ControlLogix):
//   Offset 0-1:   Reserved (INT)
//   Offset 2-5:   Status bits (DINT) - EN, TT, DN packed in the high bits
//   Offset 6-9:   PRE (DINT) - preset value in milliseconds
//   Offset 10-13: ACC (DINT) - accumulated time in milliseconds
//
// 12-byte layout (newer firmware / CompactLogix):
//   Offset 0-3:   Status bits (DINT)
//   Offset 4-7:   PRE (DINT)
//   Offset 8-11:  ACC (DINT)
//
// Use [`decode_timer`] or `Timer::unmarshal_cip` to decode from raw bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Timer {
    // Preset (ms)
    pub pre: i32,
    // Accumulated (ms)
    pub acc: i32,
    // Enable
    pub en: bool,
    // Timer Timing
    pub tt: bool,
    // Done
    pub dn: bool,
}

// Ref: Rockwell 1756-RM003 TIMER structure - EN is bit 31 of the status DINT. Timer is running.
pub const TIMER_STATUS_EN: u32 = 31;
// Ref: Rockwell 1756-RM003 TIMER structure - TT is bit 30 of the status DINT (ACC < PRE while enabled).
pub const TIMER_STATUS_TT: u32 = 30;
// Ref: Rockwell 1756-RM003 TIMER structure - DN is bit 29 of the status DINT (ACC >= PRE).
pub const TIMER_STATUS_DN: u32 = 29;

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - TIMER memory layout (see Timer docs above).
// Decodes a byte slice into a Timer. Accepts 12 bytes (status + PRE + ACC) or 14 bytes
// (reserved + status + PRE + ACC); more than 14 bytes uses the 14-byte layout.
pub fn decode_timer(data: &[u8]) -> Result<Timer> {
    let (status_off, pre_off, acc_off) = if data.len() >= 14 {
        // 14-byte layout: reserved(2) + status(4) + PRE(4) + ACC(4)
        (2, 6, 10)
    } else if data.len() >= 12 {
        // 12-byte layout: status(4) + PRE(4) + ACC(4)
        (0, 4, 8)
    } else {
        return Err(CipError::InsufficientData { kind: "Timer", expected: 12, got: data.len() });
    };

    let word = |off: usize| u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
    let status = word(status_off);
    let pre = word(pre_off) as i32;
    let acc = word(acc_off) as i32;

    Ok(Timer {
        pre,
        acc,
        en: (status & (1 << TIMER_STATUS_EN)) != 0,
        tt: (status & (1 << TIMER_STATUS_TT)) != 0,
        dn: (status & (1 << TIMER_STATUS_DN)) != 0,
    })
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - TIMER memory layout.
impl Unmarshaler for Timer {
    fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
        *self = decode_timer(data)?;
        Ok(())
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - encodes the canonical 14-byte memory layout:
// reserved INT (0), status DINT, PRE DINT, ACC DINT.
impl Marshaler for Timer {
    fn marshal_cip(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(14);

        // Reserved
        buf.extend_from_slice(&0u16.to_le_bytes());

        // Status Bits
        let mut status: u32 = 0;
        if self.en {
            status |= 1 << TIMER_STATUS_EN;
        }
        if self.tt {
            status |= 1 << TIMER_STATUS_TT;
        }
        if self.dn {
            status |= 1 << TIMER_STATUS_DN;
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

    // Ref: Rockwell 1756-RM003 TIMER structure status bits.
    #[test]
    fn timer_all_bits_set() {
        let mut data = vec![0u8; 14];
        data[0..2].copy_from_slice(&0u16.to_le_bytes());
        let status_bits: u32 = (1 << TIMER_STATUS_EN) | (1 << TIMER_STATUS_TT) | (1 << TIMER_STATUS_DN);
        data[2..6].copy_from_slice(&status_bits.to_le_bytes());
        data[6..10].copy_from_slice(&10000u32.to_le_bytes());
        data[10..14].copy_from_slice(&9999u32.to_le_bytes());

        let timer = decode_timer(&data).unwrap();
        assert!(timer.en && timer.tt && timer.dn, "expected all status bits true: {:?}", timer);
        assert!(timer.pre == 10000 && timer.acc == 9999, "PRE={} ACC={}, want 10000/9999", timer.pre, timer.acc);
    }

    #[test]
    fn timer_no_bits_set() {
        let mut data = vec![0u8; 14];
        data[6..10].copy_from_slice(&5000u32.to_le_bytes());

        let timer = decode_timer(&data).unwrap();
        assert!(!timer.en && !timer.tt && !timer.dn, "expected all status bits false");
    }

    #[test]
    fn timer_decode_short_data() {
        // Too short
        assert!(decode_timer(&[0u8; 10]).is_err(), "expected error for short data");
    }

    // Added: 12-byte layout, marshal round trip and error text (Go behavior).
    #[test]
    fn timer_12_byte_layout_and_round_trip() {
        let mut d = vec![0u8; 12];
        d[0..4].copy_from_slice(&(1u32 << TIMER_STATUS_DN).to_le_bytes());
        d[4..8].copy_from_slice(&(-5i32).to_le_bytes());
        d[8..12].copy_from_slice(&7i32.to_le_bytes());
        let t = decode_timer(&d).unwrap();
        assert_eq!(t, Timer { pre: -5, acc: 7, en: false, tt: false, dn: true });

        let orig = Timer { pre: 1000, acc: 250, en: true, tt: true, dn: false };
        let b = orig.marshal_cip().unwrap();
        assert_eq!(b.len(), 14);
        assert_eq!(decode_timer(&b).unwrap(), orig);

        assert_eq!(
            decode_timer(&[0u8; 3]).unwrap_err().to_string(),
            "insufficient data for Timer: expected at least 12 bytes, got 3"
        );
    }
}
