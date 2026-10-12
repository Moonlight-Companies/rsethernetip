//! SLC timer / counter element decoding. Port of Go `pccc/timer.go`.
//!
//! SLC timer/counter/control elements occupy three 16-bit words:
//! word 0 control bits, word 1 PRE (preset) or LEN, word 2 ACC or POS.

use super::{PcccError, Result};
use std::fmt;

// Ref: Allen-Bradley SLC 500 Instruction Set Reference Manual (publication
// 1747-RM001), timer file structure (section unverified).
/// Decoded 3-word SLC timer element (file type T, code 0x86). PRE and ACC are
/// signed 16-bit values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Timer {
    /// Raw control word; use the bit accessors.
    pub control: u16,
    /// Preset.
    pub pre: i16,
    /// Accumulated.
    pub acc: i16,
}

impl Timer {
    // Ref: SLC 500 Instruction Set Reference Manual, timer EN bit = bit 15 of word 0 (section unverified).
    /// Timer Enable bit (bit 15 of the control word).
    pub fn en(&self) -> bool {
        self.control & (1 << 15) != 0
    }
    // Ref: SLC 500 Instruction Set Reference Manual, timer TT bit = bit 14 of word 0 (section unverified).
    /// Timer Timing bit (bit 14).
    pub fn tt(&self) -> bool {
        self.control & (1 << 14) != 0
    }
    // Ref: SLC 500 Instruction Set Reference Manual, timer DN bit = bit 13 of word 0 (section unverified).
    /// Timer Done bit (bit 13).
    pub fn dn(&self) -> bool {
        self.control & (1 << 13) != 0
    }
}

// Implementation detail (not spec): diagnostic formatting (Go `String()`).
impl fmt::Display for Timer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Timer{{EN={} TT={} DN={} PRE={} ACC={}}}",
            self.en(),
            self.tt(),
            self.dn(),
            self.pre,
            self.acc
        )
    }
}

// Implementation detail (not spec): checked little-endian word read.
fn word(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

// Ref: SLC 500 Instruction Set Reference Manual, timer element = 3 words (control, PRE, ACC), little-endian on the wire (PCCC/DF1 1770-6.5.16).
/// Parses the 6 raw little-endian bytes of an SLC timer element.
pub fn decode_timer(b: &[u8]) -> Result<Timer> {
    if b.len() < 6 {
        return Err(PcccError::msg(format!("pccc: timer requires 6 bytes, got {}", b.len())));
    }
    Ok(Timer { control: word(b, 0), pre: word(b, 2) as i16, acc: word(b, 4) as i16 })
}

// Ref: SLC 500 Instruction Set Reference Manual, counter file structure (section unverified).
/// Decoded 3-word SLC counter element (file type C, code 0x87).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counter {
    /// Raw control word.
    pub control: u16,
    /// Preset.
    pub pre: i16,
    /// Accumulated.
    pub acc: i16,
}

impl Counter {
    // Ref: SLC 500 Instruction Set Reference Manual, counter CU bit = bit 15 (section unverified).
    /// Count Up bit (bit 15).
    pub fn cu(&self) -> bool {
        self.control & (1 << 15) != 0
    }
    // Ref: SLC 500 Instruction Set Reference Manual, counter CD bit = bit 14 (section unverified).
    /// Count Down bit (bit 14).
    pub fn cd(&self) -> bool {
        self.control & (1 << 14) != 0
    }
    // Ref: SLC 500 Instruction Set Reference Manual, counter DN bit = bit 13 (section unverified).
    /// Done bit (bit 13).
    pub fn dn(&self) -> bool {
        self.control & (1 << 13) != 0
    }
    // Ref: SLC 500 Instruction Set Reference Manual, counter OV bit = bit 12 (section unverified).
    /// Overflow bit (bit 12).
    pub fn ov(&self) -> bool {
        self.control & (1 << 12) != 0
    }
    // Ref: SLC 500 Instruction Set Reference Manual, counter UN bit = bit 11 (section unverified).
    /// Underflow bit (bit 11).
    pub fn un(&self) -> bool {
        self.control & (1 << 11) != 0
    }
}

// Implementation detail (not spec): diagnostic formatting (Go `String()`).
impl fmt::Display for Counter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Counter{{CU={} CD={} DN={} OV={} UN={} PRE={} ACC={}}}",
            self.cu(),
            self.cd(),
            self.dn(),
            self.ov(),
            self.un(),
            self.pre,
            self.acc
        )
    }
}

// Ref: SLC 500 Instruction Set Reference Manual, counter element = 3 words (control, PRE, ACC), little-endian (section unverified).
/// Parses the 6 raw little-endian bytes of an SLC counter element.
pub fn decode_counter(b: &[u8]) -> Result<Counter> {
    if b.len() < 6 {
        return Err(PcccError::msg(format!("pccc: counter requires 6 bytes, got {}", b.len())));
    }
    Ok(Counter { control: word(b, 0), pre: word(b, 2) as i16, acc: word(b, 4) as i16 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_timer_ok() {
        // Control = 0xE000 (EN=1, TT=1, DN=1), PRE=1000 (0x03E8), ACC=250 (0x00FA)
        let raw = [0x00, 0xE0, 0xE8, 0x03, 0xFA, 0x00];
        let tm = decode_timer(&raw).unwrap();
        assert!(tm.en() && tm.tt() && tm.dn());
        assert_eq!(tm.pre, 1000);
        assert_eq!(tm.acc, 250);
    }

    #[test]
    fn decode_timer_short() {
        assert!(decode_timer(&[0, 0]).is_err());
    }

    #[test]
    fn decode_counter_ok() {
        // Control = 0xF800 (CU,CD,DN,OV,UN), PRE=100, ACC=50
        let raw = [0x00, 0xF8, 0x64, 0x00, 0x32, 0x00];
        let c = decode_counter(&raw).unwrap();
        assert!(c.cu() && c.cd() && c.dn() && c.ov() && c.un(), "{c}");
        assert_eq!((c.pre, c.acc), (100, 50));
    }

    #[test]
    fn decode_counter_short() {
        assert!(decode_counter(&[]).is_err());
    }
}
