//! ethernetip-write-tag: writes a value to a Logix tag, then reads it back. Port of Go
//! `examples/ethernetip/write_tag`.
//!
//! Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Write Tag service (0x4D): request data is
//! the 2-byte CIP type code, a 2-byte element count, then the value bytes. CIP Vol. 1,
//! Appendix C data type encodings (section unverified).
//!
//! Usage:
//!   ethernetip-write-tag -addr 192.168.1.10:44818 -tag MyDINT -type DINT -value 42
//!   ethernetip-write-tag -addr 192.168.1.10:44818 -tag MyREAL -type REAL -value 3.14

use std::time::Duration;

use rsethernetip::cip;
use rsethernetip::cli::{self, Cli};
use rsethernetip::client;
use rsethernetip::options::{with_retries, with_retry_delay};
use rsethernetip::plc::Context;

// Implementation detail (not spec): a parsed value tagged with its CIP type, the Rust stand-in
// for Go's `any` return of `parseValue`.
enum Val {
    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    Str(String),
}

impl std::fmt::Display for Val {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Val::Bool(v) => write!(f, "{v}"),
            Val::I8(v) => write!(f, "{v}"),
            Val::I16(v) => write!(f, "{v}"),
            Val::I32(v) => write!(f, "{v}"),
            Val::I64(v) => write!(f, "{v}"),
            Val::U8(v) => write!(f, "{v}"),
            Val::U16(v) => write!(f, "{v}"),
            Val::U32(v) => write!(f, "{v}"),
            Val::U64(v) => write!(f, "{v}"),
            Val::F32(v) => write!(f, "{v}"),
            Val::F64(v) => write!(f, "{v}"),
            Val::Str(v) => write!(f, "{v}"),
        }
    }
}

fn main() {
    let args = Cli::new("ethernetip-write-tag", "Writes a value to a Logix tag and reads it back.")
        .flag("addr", "192.168.1.10:44818", "PLC address in host:port format (EIP default port is 44818)")
        .flag("tag", "", "Tag name on the PLC (symbolic addressing)")
        .flag(
            "type",
            "DINT",
            "CIP data type: BOOL, SINT, INT, DINT, LINT, USINT, UINT, UDINT, ULINT, REAL, LREAL, STRING",
        )
        .flag("value", "", "Value to write (parsed according to -type)")
        .parse_env();
    let addr = args.str("addr").to_string();
    let tag = args.str("tag").to_string();
    let type_name = args.str("type").to_string();
    let value = args.str("value").to_string();

    if tag.is_empty() || value.is_empty() {
        eprintln!("error: -tag and -value are required");
        std::process::exit(1);
    }

    let go_value = match parse_value(&type_name, &value) {
        Ok(v) => v,
        Err(e) => cli::fatal(format!("Failed to parse value {value:?} as {type_name}: {e}")),
    };

    let ctx = Context::background().with_timeout(Duration::from_secs(10));
    let client = match client::connect(
        &ctx,
        &addr,
        vec![with_retries(2).into(), with_retry_delay(Duration::from_millis(500)).into()],
    ) {
        Ok(c) => c,
        Err(e) => cli::fatal(format!("Failed to connect to {addr}: {e}")),
    };
    println!("Connected to {addr}\n");

    println!("Writing tag {tag:?} = {go_value} (type {type_name})");
    let res = match &go_value {
        Val::Bool(v) => client.write_tag(&ctx, &tag, v),
        Val::I8(v) => client.write_tag(&ctx, &tag, v),
        Val::I16(v) => client.write_tag(&ctx, &tag, v),
        Val::I32(v) => client.write_tag(&ctx, &tag, v),
        Val::I64(v) => client.write_tag(&ctx, &tag, v),
        Val::U8(v) => client.write_tag(&ctx, &tag, v),
        Val::U16(v) => client.write_tag(&ctx, &tag, v),
        Val::U32(v) => client.write_tag(&ctx, &tag, v),
        Val::U64(v) => client.write_tag(&ctx, &tag, v),
        Val::F32(v) => client.write_tag(&ctx, &tag, v),
        Val::F64(v) => client.write_tag(&ctx, &tag, v),
        // Go's WriteTag(string) type-maps to STRING but then fails in encoding/binary; the Rust
        // `Marshaler` has no string impl, so report the same failure instead of writing.
        Val::Str(_) => Err(rsethernetip::conn::Error::msg("binary.Write: some values are not fixed-sized in type string")),
    };
    if let Err(e) = res {
        cli::fatal(format!("WriteTag failed: {e}"));
    }
    println!("Write successful.");

    println!("\n--- Read-back verification ---");
    let raw = match client.read_tag(&ctx, &tag) {
        Ok(r) => r,
        Err(e) => cli::fatal(format!("ReadTag (verification) failed: {e}")),
    };
    if raw.len() < 2 {
        cli::fatal(format!("Response too short ({} bytes)", raw.len()));
    }
    let type_code = cip::DataType(u16::from_le_bytes([raw[0], raw[1]]));
    println!("Type Code:  0x{:04X} ({})", type_code.0, type_code);
    println!("Raw bytes:  {}", cli::hex_spaced(&raw));

    match decode_value(&type_name, &raw) {
        Ok(v) => println!("Read-back:  {v}"),
        Err(e) => cli::fatal(format!("Failed to decode read-back: {e}")),
    }

    println!("\nDone.");
    let _ = client.close();
}

// Implementation detail (not spec): Go `strconv.ParseBool` accepts these spellings.
fn parse_bool(s: &str) -> Result<bool, String> {
    match s {
        "1" | "t" | "T" | "true" | "TRUE" | "True" => Ok(true),
        "0" | "f" | "F" | "false" | "FALSE" | "False" => Ok(false),
        _ => Err(format!("strconv.ParseBool: parsing {s:?}: invalid syntax")),
    }
}

// Implementation detail (not spec): maps the -type name to a typed value, as Go `parseValue` does.
fn parse_value(type_name: &str, s: &str) -> Result<Val, String> {
    fn num<T: std::str::FromStr>(s: &str) -> Result<T, String>
    where
        T::Err: std::fmt::Display,
    {
        s.parse::<T>().map_err(|e| format!("parsing {s:?}: {e}"))
    }
    match type_name.to_uppercase().as_str() {
        "BOOL" => parse_bool(s).map(Val::Bool),
        "SINT" => num(s).map(Val::I8),
        "INT" => num(s).map(Val::I16),
        "DINT" => num(s).map(Val::I32),
        "LINT" => num(s).map(Val::I64),
        "USINT" => num(s).map(Val::U8),
        "UINT" => num(s).map(Val::U16),
        "UDINT" => num(s).map(Val::U32),
        "ULINT" => num(s).map(Val::U64),
        "REAL" => num(s).map(Val::F32),
        "LREAL" => num(s).map(Val::F64),
        "STRING" => Ok(Val::Str(s.to_string())),
        _ => Err(format!("unsupported CIP type {type_name:?}")),
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access: the reply is a 2-byte type code (4 bytes for
// structures, 0x02A0 and up) followed by little-endian data; STRING here is a 4-byte length
// prefix plus characters, as in the Go example (the Logix STRING structure layout is
// section unverified).
fn decode_value(type_name: &str, raw: &[u8]) -> Result<String, String> {
    if raw.len() < 2 {
        return Err("raw data too short".into());
    }
    let type_code = cip::DataType(u16::from_le_bytes([raw[0], raw[1]]));
    let hdr_len = if type_code >= cip::TYPE_STRUCT { 4 } else { 2 };
    if raw.len() < hdr_len {
        return Err("raw data too short for header".into());
    }
    let data = &raw[hdr_len..];

    // Checked fixed-size read helper (no panics on short data).
    fn take<const N: usize>(data: &[u8], what: &str) -> Result<[u8; N], String> {
        data.get(..N)
            .and_then(|b| <[u8; N]>::try_from(b).ok())
            .ok_or_else(|| format!("not enough data for {what}"))
    }

    let t = type_name.to_uppercase();
    Ok(match t.as_str() {
        "BOOL" => (take::<1>(data, "BOOL")?[0] != 0).to_string(),
        "SINT" => i8::from_le_bytes(take(data, "SINT")?).to_string(),
        "INT" => i16::from_le_bytes(take(data, "INT")?).to_string(),
        "DINT" => i32::from_le_bytes(take(data, "DINT")?).to_string(),
        "LINT" => i64::from_le_bytes(take(data, "LINT")?).to_string(),
        "USINT" => u8::from_le_bytes(take(data, "USINT")?).to_string(),
        "UINT" => u16::from_le_bytes(take(data, "UINT")?).to_string(),
        "UDINT" => u32::from_le_bytes(take(data, "UDINT")?).to_string(),
        "ULINT" => u64::from_le_bytes(take(data, "ULINT")?).to_string(),
        "REAL" => f32::from_le_bytes(take(data, "REAL")?).to_string(),
        "LREAL" => f64::from_le_bytes(take(data, "LREAL")?).to_string(),
        "STRING" => {
            let n = u32::from_le_bytes(take(data, "STRING length prefix")?) as usize;
            let body = data.get(4..).unwrap_or(&[]);
            if body.len() < n {
                return Err(format!("not enough data for STRING body (need {n}, have {})", body.len()));
            }
            String::from_utf8_lossy(&body[..n]).into_owned()
        }
        _ => return Err(format!("unsupported CIP type {type_name:?} for decoding")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_decode_round_trip() {
        assert!(matches!(parse_value("dint", "42"), Ok(Val::I32(42))));
        assert!(parse_value("SINT", "300").is_err());
        assert!(parse_value("NOPE", "1").is_err());
        let mut raw = vec![0xC4, 0x00];
        raw.extend_from_slice(&42i32.to_le_bytes());
        assert_eq!(decode_value("DINT", &raw).unwrap(), "42");
        assert!(decode_value("DINT", &raw[..4]).is_err());
        assert!(decode_value("STRING", &[0xD0, 0, 9, 0, 0, 0, b'a']).is_err());
    }
}
