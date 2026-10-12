//! SLC data-table address parsing. Port of Go `pccc/address.go`.
//!
//! Ref: Allen-Bradley PCCC / DF1 Protocol and Command Set Reference Manual,
//! publication 1770-6.5.16 (logical addressing); SLC 500 Instruction Set
//! Reference Manual 1747-RM001 for file mnemonics and timer/counter/control
//! element layouts (section unverified).

use super::frame::{FileType, file_type_from_letter};
use super::{PcccError, Result};
use std::fmt;

// Ref: PCCC/DF1 manual 1770-6.5.16, logical address fields (file number, file
// type, element, sub-element); bit suffix per SLC addressing (section unverified).
/// A parsed SLC data-table reference such as N7:0, B3:0/2, F8:5 or T4:0.ACC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address {
    /// SLC file type code.
    pub file_type: FileType,
    /// File number (7 in N7:0). For S addresses without a number this is 2.
    pub file_number: i64,
    /// Element index within the file (the integer after ':').
    pub element: i64,
    /// 16-bit word within a multi-word element: 0 control, 1 PRE/LEN, 2 ACC/POS.
    pub sub_element: i64,
    /// Bit position within the word, or -1 if no bit suffix / named bit.
    pub bit_num: i64,
}

impl Default for Address {
    // Implementation detail (not spec): Go zero value has BitNum 0; kept as-is.
    fn default() -> Address {
        Address { file_type: FileType(0), file_number: 0, element: 0, sub_element: 0, bit_num: 0 }
    }
}

// Implementation detail (not spec): Go `String()`; inverse of parse_address.
impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        match implicit_file_number(self.file_type) {
            // Omit the file number for S, I, O when it equals the default.
            Some(def) if self.file_number == def => {
                s.push_str(self.file_type.letter());
                s.push(':');
                s.push_str(&self.element.to_string());
            }
            _ => {
                s.push_str(self.file_type.letter());
                s.push_str(&self.file_number.to_string());
                s.push(':');
                s.push_str(&self.element.to_string());
            }
        }
        // Prefer the named field form when we recognise (subElem, bit).
        if let Some(name) = field_name_for(self.file_type, self.sub_element, self.bit_num) {
            s.push('.');
            s.push_str(name);
            return f.write_str(&s);
        }
        if self.bit_num >= 0 {
            s.push('/');
            s.push_str(&self.bit_num.to_string());
        }
        f.write_str(&s)
    }
}

// Ref: PCCC/DF1 manual 1770-6.5.16 logical addressing: <type><file>:<element>[.<field>|/<bit>]; S:<element> implies file 2.
/// Parses an SLC address such as "N7:0", "B3:0/2", "F8:5", "T4:0.ACC" or
/// "S:1". Case-insensitive; surrounding whitespace is ignored.
pub fn parse_address(s: &str) -> Result<Address> {
    let e = |m: String| Err(PcccError::msg(m));
    let input = s.trim();
    if input.is_empty() {
        return e("pccc: empty address".to_string());
    }
    let up = input.to_uppercase();

    // 1. Split off the optional .FIELD or /BIT suffix.
    let (main_part, suffix, sep) = split_suffix(&up);

    // 2. Split main into <letters><digits>:<element>.
    let Some((left, right)) = main_part.split_once(':') else {
        return e(format!("pccc: missing ':' in {s:?}"));
    };
    if right.is_empty() {
        return e(format!("pccc: missing element number in {s:?}"));
    }

    let (letters, file_num_str) = split_leading_letters(left);
    if letters.is_empty() {
        return e(format!("pccc: missing file type in {s:?}"));
    }
    let Some(ft) = file_type_from_letter(letters) else {
        return e(format!("pccc: unknown file type {letters:?} in {s:?}"));
    };

    // 3. File number. S/I/O have implicit defaults (2/1/0); others require
    //    an explicit number. An explicit number conflicting with the
    //    implicit default is rejected.
    let file_num: i64;
    if file_num_str.is_empty() {
        match implicit_file_number(ft) {
            Some(def) => file_num = def,
            None => return e(format!("pccc: missing file number in {s:?}")),
        }
    } else {
        if !is_all_digits(file_num_str) {
            return e(format!("pccc: invalid file number {file_num_str:?} in {s:?}"));
        }
        let Ok(n) = file_num_str.parse::<i64>() else {
            return e(format!("pccc: invalid file number {file_num_str:?} in {s:?}"));
        };
        if !(0..=254).contains(&n) {
            return e(format!("pccc: file number {n} out of range [0,254] in {s:?}"));
        }
        if let Some(def) = implicit_file_number(ft)
            && n != def
        {
            return e(format!(
                "pccc: file type {} requires file number {} (got {}) in {:?}",
                ft.letter(),
                def,
                n,
                s
            ));
        }
        file_num = n;
    }

    // 4. Element — must be a plain integer; trailing garbage reported.
    if !is_all_digits_or_leading_sign(right) {
        return e(format!("pccc: trailing garbage after element in {s:?}"));
    }
    let Ok(elem) = right.parse::<i64>() else {
        return e(format!("pccc: invalid element {right:?} in {s:?}"));
    };
    if !(0..=254).contains(&elem) {
        return e(format!("pccc: element {elem} out of range [0,254] in {s:?}"));
    }

    let mut addr =
        Address { file_type: ft, file_number: file_num, element: elem, sub_element: 0, bit_num: -1 };

    // 5. Suffix: ".FIELD" (named field) or "/N" (bit 0..15).
    match sep {
        None => {}
        Some('.') => {
            let Some((sub, bit)) = lookup_named_field(ft, suffix) else {
                return e(format!(
                    "pccc: unknown field {:?} for file type {} in {:?}",
                    suffix,
                    ft.letter(),
                    s
                ));
            };
            addr.sub_element = sub;
            addr.bit_num = bit;
        }
        Some(_) => {
            if suffix.contains(['.', '/']) {
                return e(format!("pccc: trailing garbage after bit in {s:?}"));
            }
            if !is_all_digits_or_leading_sign(suffix) {
                return e(format!("pccc: invalid bit {suffix:?} in {s:?}"));
            }
            let Ok(bit) = suffix.parse::<i64>() else {
                return e(format!("pccc: invalid bit {suffix:?} in {s:?}"));
            };
            if !(0..=15).contains(&bit) {
                return e(format!("pccc: bit {bit} out of range [0,15] in {s:?}"));
            }
            if !supports_bit_suffix(ft) {
                return e(format!(
                    "pccc: bit suffix not supported for file type {} in {:?}",
                    ft.letter(),
                    s
                ));
            }
            addr.bit_num = bit;
        }
    }

    Ok(addr)
}

// Implementation detail (not spec): suffix splitter. Leftmost of '.'/'/'
// wins; any further delimiter is caught as trailing garbage later.
fn split_suffix(up: &str) -> (&str, &str, Option<char>) {
    let dot = up.find('.');
    let slash = up.find('/');
    let (idx, sep) = match (dot, slash) {
        (None, None) => return (up, "", None),
        (Some(d), Some(sl)) => {
            if sl < d { (sl, '/') } else { (d, '.') }
        }
        (Some(d), None) => (d, '.'),
        (None, Some(sl)) => (sl, '/'),
    };
    (&up[..idx], &up[idx + 1..], Some(sep))
}

// Implementation detail (not spec): separates file-type mnemonic from number
// ("N7" -> "N","7"; "ST10" -> "ST","10").
fn split_leading_letters(s: &str) -> (&str, &str) {
    let i = s.bytes().position(|b| !b.is_ascii_alphabetic()).unwrap_or(s.len());
    (&s[..i], &s[i..])
}

// Implementation detail (not spec): byte-wise digit check.
fn is_all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

// Implementation detail (not spec): allows a leading sign so out-of-range
// inputs like "N7:-1" yield a range error rather than a parse error.
fn is_all_digits_or_leading_sign(s: &str) -> bool {
    match s.as_bytes().first() {
        None => false,
        Some(b'-') | Some(b'+') => s.len() > 1 && is_all_digits(&s[1..]),
        Some(_) => is_all_digits(s),
    }
}

// Ref: SLC 500 convention: O file = 0, I file = 1, S file = 2 (SLC 500
// Instruction Set Reference Manual 1747-RM001, section unverified).
fn implicit_file_number(ft: FileType) -> Option<i64> {
    match ft {
        FileType::OUTPUT => Some(0),
        FileType::INPUT => Some(1),
        FileType::STATUS => Some(2),
        _ => None,
    }
}

// Ref: SLC bit addressing applies to word-sized integer/bit files (SLC 500
// Instruction Set Reference Manual 1747-RM001, section unverified).
fn supports_bit_suffix(ft: FileType) -> bool {
    matches!(
        ft,
        FileType::INTEGER
            | FileType::BIT
            | FileType::STATUS
            | FileType::INPUT
            | FileType::OUTPUT
            | FileType::BCD
    )
}

// Implementation detail (not spec): (name, sub-element, bit). The sub-element
// selects the word inside the multi-word element; bit is -1 unless the field
// names a specific bit.
type NamedField = (&'static str, i64, i64);

// Ref: SLC 500 Instruction Set Reference Manual 1747-RM001, timer element
// (3 words): word 0 EN 15 / TT 14 / DN 13, word 1 PRE, word 2 ACC (section unverified).
static TIMER_FIELDS: [NamedField; 5] =
    [("PRE", 1, -1), ("ACC", 2, -1), ("EN", 0, 15), ("TT", 0, 14), ("DN", 0, 13)];

// Ref: SLC 500 Instruction Set Reference Manual 1747-RM001, counter element:
// word 0 CU 15 / CD 14 / DN 13 / OV 12 / UN 11 / UA 10, word 1 PRE, word 2 ACC (section unverified).
static COUNTER_FIELDS: [NamedField; 8] = [
    ("PRE", 1, -1),
    ("ACC", 2, -1),
    ("CU", 0, 15),
    ("CD", 0, 14),
    ("DN", 0, 13),
    ("OV", 0, 12),
    ("UN", 0, 11),
    ("UA", 0, 10),
];

// Ref: SLC 500 Instruction Set Reference Manual 1747-RM001, control element:
// word 0 EN 15 / EU 14 / DN 13 / EM 12 / ER 11 / UL 10 / IN 9 / FD 8, word 1 LEN, word 2 POS (section unverified).
static CONTROL_FIELDS: [NamedField; 10] = [
    ("LEN", 1, -1),
    ("POS", 2, -1),
    ("EN", 0, 15),
    ("EU", 0, 14),
    ("DN", 0, 13),
    ("EM", 0, 12),
    ("ER", 0, 11),
    ("UL", 0, 10),
    ("IN", 0, 9),
    ("FD", 0, 8),
];

fn table_for(ft: FileType) -> Option<&'static [NamedField]> {
    match ft {
        FileType::TIMER => Some(&TIMER_FIELDS),
        FileType::COUNTER => Some(&COUNTER_FIELDS),
        FileType::CONTROL => Some(&CONTROL_FIELDS),
        _ => None,
    }
}

fn lookup_named_field(ft: FileType, name: &str) -> Option<(i64, i64)> {
    table_for(ft)?.iter().find(|f| f.0 == name).map(|f| (f.1, f.2))
}

// Implementation detail (not spec): canonical mnemonic for (type, sub, bit).
fn field_name_for(ft: FileType, sub: i64, bit: i64) -> Option<&'static str> {
    table_for(ft)?.iter().find(|f| f.1 == sub && f.2 == bit).map(|f| f.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_address_basic() {
        let tests: [(&str, FileType, i64, i64, i64, i64); 9] = [
            ("N7:0", FileType::INTEGER, 7, 0, 0, -1),
            ("N7:42", FileType::INTEGER, 7, 42, 0, -1),
            ("F8:5", FileType::FLOAT, 8, 5, 0, -1),
            ("B3:0", FileType::BIT, 3, 0, 0, -1),
            ("O:0", FileType::OUTPUT, 0, 0, 0, -1),
            ("I:1", FileType::INPUT, 1, 1, 0, -1),
            ("ST10:0", FileType::STRING, 10, 0, 0, -1),
            ("S:1", FileType::STATUS, 2, 1, 0, -1),
            ("S:7", FileType::STATUS, 2, 7, 0, -1),
        ];
        for (inp, ft, fnum, elem, sub, bit) in tests {
            let a = parse_address(inp).unwrap_or_else(|e| panic!("{inp}: {e}"));
            assert_eq!(a.file_type, ft, "{inp}");
            assert_eq!(a.file_number, fnum, "{inp}");
            assert_eq!(a.element, elem, "{inp}");
            assert_eq!(a.sub_element, sub, "{inp}");
            assert_eq!(a.bit_num, bit, "{inp}");
        }
    }

    #[test]
    fn parse_address_bit_suffix() {
        for (inp, bit) in [("B3:0/0", 0), ("B3:0/15", 15), ("N7:5/2", 2), ("N7:5/7", 7)] {
            let a = parse_address(inp).unwrap();
            assert_eq!(a.bit_num, bit);
            assert_eq!(a.sub_element, 0);
        }
    }

    fn check_fields(ft: FileType, cases: &[(&str, i64, i64)]) {
        for (inp, sub, bit) in cases {
            let a = parse_address(inp).unwrap();
            assert_eq!(a.file_type, ft, "{inp}");
            assert_eq!(a.sub_element, *sub, "{inp}");
            assert_eq!(a.bit_num, *bit, "{inp}");
        }
    }

    #[test]
    fn parse_address_timer_fields() {
        check_fields(
            FileType::TIMER,
            &[("T4:0.PRE", 1, -1), ("T4:0.ACC", 2, -1), ("T4:0.EN", 0, 15), ("T4:0.TT", 0, 14), ("T4:0.DN", 0, 13)],
        );
    }

    #[test]
    fn parse_address_counter_fields() {
        check_fields(
            FileType::COUNTER,
            &[
                ("C5:0.PRE", 1, -1),
                ("C5:0.ACC", 2, -1),
                ("C5:0.CU", 0, 15),
                ("C5:0.CD", 0, 14),
                ("C5:0.DN", 0, 13),
                ("C5:0.OV", 0, 12),
                ("C5:0.UN", 0, 11),
            ],
        );
    }

    #[test]
    fn parse_address_control_fields() {
        check_fields(
            FileType::CONTROL,
            &[("R6:0.LEN", 1, -1), ("R6:0.POS", 2, -1), ("R6:0.EN", 0, 15), ("R6:0.DN", 0, 13), ("R6:0.ER", 0, 11)],
        );
    }

    #[test]
    fn parse_address_case_insensitive() {
        assert_eq!(parse_address("n7:0").unwrap(), parse_address("N7:0").unwrap());
        assert_eq!(parse_address("t4:0.acc").unwrap().sub_element, 2);
    }

    #[test]
    fn parse_address_trims_whitespace() {
        let a = parse_address("  N7:0  ").unwrap();
        assert!(a.file_type == FileType::INTEGER && a.file_number == 7 && a.element == 0);
    }

    #[test]
    fn parse_address_string() {
        for inp in ["N7:0", "F8:5", "B3:0/2", "T4:0.ACC", "T4:0.EN", "C5:0.PRE", "S:1"] {
            assert_eq!(parse_address(inp).unwrap().to_string(), inp);
        }
    }

    #[test]
    fn parse_address_errors() {
        let tests = [
            ("", "empty"),
            ("   ", "empty"),
            ("N", "missing"),
            ("N7", "missing"),
            ("N7:", "missing"),
            (":0", "file type"),
            ("X7:0", "file type"),
            ("N-1:0", "file"),
            ("N7:-1", "element"),
            ("N7:0/16", "bit"),
            ("N7:0/-1", "bit"),
            ("N7:0.XYZ", "field"),
            ("T4:0.FOO", "field"),
            ("F8:0/0", "bit"),
            ("N7:0abc", "trailing"),
            ("N7:0/2/3", "trailing"),
        ];
        for (inp, must) in tests {
            let err = parse_address(inp).expect_err(inp);
            assert!(err.to_string().to_lowercase().contains(must), "{inp}: {err}");
        }
    }

    #[test]
    fn parse_address_status_file_number() {
        assert_eq!(parse_address("S2:1").unwrap().file_number, 2);
        assert!(parse_address("S5:1").is_err());
    }
}
