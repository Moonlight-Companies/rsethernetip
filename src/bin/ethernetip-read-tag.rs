//! ethernetip-read-tag: reads one or more elements of a tag from a Rockwell Logix PLC over
//! EtherNet/IP. Port of Go `examples/ethernetip/read_tag`.
//!
//! Every CIP request travels inside an EIP encapsulation frame:
//!
//! ```text
//! [TCP] -> [EIP Header (24 bytes)] -> [CPF] -> [CIP Message Router Request]
//! ```
//!
//! Ref: ODVA EtherNet/IP Adaptation of CIP (CIP Vol. 2), Section 2-3 Encapsulation Message and
//! Section 2-6 Common Packet Format. Reading a tag uses CIP service 0x4C (Read Tag) with a
//! symbolic segment path: Rockwell 1756-PM020 Logix 5000 Data Access.
//!
//! Usage:
//!   ethernetip-read-tag -addr 192.168.1.10:44818 -tag MyDINT
//!   ethernetip-read-tag -addr 192.168.1.10:44818 -tag MyArray -count 10

use std::fmt::{Debug, Display};
use std::time::Duration;

use rsethernetip::cip::{self, SizedCip, Unmarshaler};
use rsethernetip::cli::{self, Cli};
use rsethernetip::client::{self, Client};
use rsethernetip::options::{with_retries, with_retry_delay};
use rsethernetip::plc::Context;

fn main() {
    let args = Cli::new("ethernetip-read-tag", "Reads one or more elements of a tag from a Logix PLC.")
        .flag("addr", "192.168.1.10:44818", "PLC address in host:port format (EIP default port is 44818)")
        .flag("tag", "MyDINT", "Tag name on the PLC (symbolic addressing)")
        .flag("count", "1", "Number of elements to read (1 for scalar, >1 for array slice)")
        .parse_env();
    let addr = args.str("addr").to_string();
    let tag = args.str("tag").to_string();
    let count: u16 = args.get("count");

    if tag.is_empty() {
        eprintln!("error: -tag is required");
        std::process::exit(1);
    }

    // Implementation detail (not spec): one deadline covers dial, session registration and CIP traffic.
    let ctx = Context::background().with_timeout(Duration::from_secs(10));

    // Ref: CIP Vol. 2 Section 2-3: TCP connect to port 44818 + RegisterSession (0x0065) yields the
    // session handle used by every later encapsulated request. Connect does both. Transport
    // errors are retried; CIP status errors never are.
    let client = match client::connect(
        &ctx,
        &addr,
        vec![with_retries(2).into(), with_retry_delay(Duration::from_millis(500)).into()],
    ) {
        Ok(c) => c,
        Err(e) => cli::fatal(format!("Failed to connect to {addr}: {e}")),
    };

    println!("Connected to {addr}\n");

    // Ref: Rockwell 1756-PM020 Logix 5000 Data Access: the Read Tag reply starts with a 2-byte
    // little-endian CIP type code (structures: 0x02A0 plus a 2-byte handle), then the data.
    println!("--- Raw byte read ---");
    let raw = if count == 1 {
        client.read_tag(&ctx, &tag)
    } else {
        client.read_tag_elements(&ctx, &tag, count)
    };
    let raw = match raw {
        Ok(r) => r,
        Err(e) => cli::fatal(format!("ReadTag failed: {e}")),
    };

    if raw.len() < 2 {
        cli::fatal(format!("Response too short ({} bytes) - expected at least 2 for type code", raw.len()));
    }
    let type_code = cip::DataType(u16::from_le_bytes([raw[0], raw[1]]));

    println!("Tag:        {tag}");
    println!("Type Code:  0x{:04X} ({})", type_code.0, type_code);
    println!("Raw bytes:  {}", cli::hex_spaced(&raw));
    println!("Data bytes: {}", cli::hex_spaced(&raw[2..]));

    // Ref: CIP Vol. 1, Appendix C data type codes (section unverified): the code selects the Rust
    // type used for the generic typed read below.
    println!("\n--- Typed read ---");
    match type_code {
        cip::TYPE_BOOL => typed_read::<bool>(&client, &ctx, &tag, count, "bool"),
        cip::TYPE_SINT => typed_read::<i8>(&client, &ctx, &tag, count, "i8"),
        cip::TYPE_INT => typed_read::<i16>(&client, &ctx, &tag, count, "i16"),
        cip::TYPE_DINT => typed_read::<i32>(&client, &ctx, &tag, count, "i32"),
        cip::TYPE_LINT => typed_read::<i64>(&client, &ctx, &tag, count, "i64"),
        cip::TYPE_USINT => typed_read::<u8>(&client, &ctx, &tag, count, "u8"),
        cip::TYPE_UINT => typed_read::<u16>(&client, &ctx, &tag, count, "u16"),
        cip::TYPE_UDINT => typed_read::<u32>(&client, &ctx, &tag, count, "u32"),
        cip::TYPE_ULINT => typed_read::<u64>(&client, &ctx, &tag, count, "u64"),
        cip::TYPE_REAL => typed_read::<f32>(&client, &ctx, &tag, count, "f32"),
        cip::TYPE_LREAL => typed_read::<f64>(&client, &ctx, &tag, count, "f64"),
        _ => println!("No typed read for type code 0x{:04X}; use raw bytes above.", type_code.0),
    }

    // Common CIP type codes (CIP Vol. 1 Appendix C, section unverified):
    //   0x00C1 BOOL, 0x00C2 SINT, 0x00C3 INT, 0x00C4 DINT, 0x00C5 LINT, 0x00C6 USINT,
    //   0x00C7 UINT, 0x00C8 UDINT, 0x00C9 ULINT, 0x00CA REAL, 0x00CB LREAL, 0x00D0 STRING,
    //   0x02A0 STRUCT (extra 2-byte handle).
    println!("\nDone.");
    let _ = client.close();
}

// Implementation detail (not spec): Go generic `typedRead[T]`.
fn typed_read<T>(client: &Client, ctx: &Context, tag: &str, count: u16, type_name: &str)
where
    T: Unmarshaler + SizedCip + Default + Clone + Display + Debug,
{
    if count == 1 {
        match client::read_typed::<T>(client, ctx, tag) {
            Ok(v) => println!("Value ({type_name}): {v}"),
            Err(e) => cli::fatal(format!("Typed Read failed: {e}")),
        }
    } else {
        match client::read_slice::<T>(client, ctx, tag, count) {
            Ok(v) => println!("Values ([]{type_name}): {}", cli::fmt_list(&v)),
            Err(e) => cli::fatal(format!("Typed ReadSlice failed: {e}")),
        }
    }
}
