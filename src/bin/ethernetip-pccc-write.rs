//! ethernetip-pccc-write: writes a value to a single PCCC data-table address. Port of Go
//! `examples/ethernetip/pccc_write`.
//!
//! Ref: Allen-Bradley PCCC / DF1 protocol manual 1770-6.5.16 - Protected Typed Logical Write with
//! 3 address fields (CMD 0x0F, FNC 0xAA); bit writes are a read-modify-write of the containing
//! word.
//!
//! Usage:
//!   ethernetip-pccc-write -addr 10.30.40.71:44818 -tag N7:0 -value 42
//!   ethernetip-pccc-write -addr 10.30.40.71:44818 -tag F8:5 -value 3.14
//!   ethernetip-pccc-write -addr 10.30.40.71:44818 -tag B3:0/2 -value 1

use std::sync::Arc;
use std::time::Duration;

use rsethernetip::cli::{self, Cli};
use rsethernetip::client;
use rsethernetip::options::{with_retries, with_retry_delay};
use rsethernetip::pccc::{self, FileType, WriteValue};
use rsethernetip::plc::Context;

fn main() {
    let args = Cli::new("ethernetip-pccc-write", "Writes one PCCC data-table address over EtherNet/IP.")
        .flag("addr", "10.30.40.71:44818", "SLC/MicroLogix address (host:port)")
        .flag("tag", "N7:0", "PCCC data-table address (e.g. N7:0, F8:5, B3:0/2)")
        .flag("value", "0", "value to write (int, float, or 0/1 for bits)")
        .parse_env();
    let addr = args.str("addr").to_string();
    let tag = args.str("tag").to_string();
    let val = args.str("value").to_string();

    let a = match pccc::parse_address(&tag) {
        Ok(a) => a,
        Err(e) => cli::fatal(format!("parse {tag:?}: {e}")),
    };

    // Coerce -value to the right type for the address (bit -> bool, F -> f32, else i16).
    let (go_val, shown): (WriteValue, String) = if a.bit_num >= 0 {
        match val.to_lowercase().as_str() {
            "1" | "true" | "on" | "high" => (true.into(), "true".into()),
            "0" | "false" | "off" | "low" => (false.into(), "false".into()),
            _ => cli::fatal(format!("bit write: -value must be 0 or 1 (got {val:?})")),
        }
    } else if a.file_type == FileType::FLOAT {
        match val.parse::<f32>() {
            Ok(f) => (f.into(), f.to_string()),
            Err(e) => cli::fatal(format!("float write: {e}")),
        }
    } else {
        // Go: strconv.Atoi then int16(n) (truncating), kept.
        match val.parse::<i64>() {
            Ok(n) => ((n as i16).into(), (n as i16).to_string()),
            Err(e) => cli::fatal(format!("integer write: {e}")),
        }
    };

    let ctx = Context::background().with_timeout(Duration::from_secs(10));
    let eip = match client::connect(
        &ctx,
        &addr,
        vec![with_retries(2).into(), with_retry_delay(Duration::from_millis(500)).into()],
    ) {
        Ok(c) => c,
        Err(e) => cli::fatal(format!("connect {addr}: {e}")),
    };
    println!("Connected to {addr}");

    let client = pccc::Client::new(Arc::new(eip));
    if let Err(e) = client.write_address(&ctx, &tag, go_val) {
        cli::fatal(format!("WriteAddress {tag} = {shown}: {e}"));
    }
    println!("wrote {tag} = {shown}");
}
