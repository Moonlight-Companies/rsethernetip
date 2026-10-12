//! ethernetip-read-tag-typed: reads a tag using the typed generic helpers and the `-type` flag.
//! Port of Go `examples/ethernetip/read_tag_typed`.
//!
//! The scalar types go through `read_typed::<T>` / `read_slice::<T>`, which strip the CIP type
//! header and unmarshal the data little-endian. TIMER and COUNTER use `read_tag_into` with the
//! Rockwell structures.
//!
//! Ref: Rockwell 1756-PM020 Logix 5000 Data Access (Read Tag 0x4C reply layout); CIP Vol. 1,
//! Appendix C data type encodings (section unverified); Logix TIMER/COUNTER structure layout
//! (Rockwell 1756-PM020, section unverified).
//!
//! Usage:
//!   ethernetip-read-tag-typed -addr 192.168.1.10:44818 -tag MyDINT -type DINT
//!   ethernetip-read-tag-typed -addr 192.168.1.10:44818 -tag MyArray -type REAL -count 8

use std::fmt::{Debug, Display};
use std::time::Duration;

use rsethernetip::cip::{self, SizedCip, Unmarshaler};
use rsethernetip::cli::{self, Cli};
use rsethernetip::client::{self, Client};
use rsethernetip::options::{with_retries, with_retry_delay};
use rsethernetip::plc::Context;

const SUPPORTED: &str = "BOOL, SINT, INT, DINT, LINT, USINT, UINT, UDINT, ULINT, REAL, LREAL, TIMER, COUNTER";

fn main() {
    let args = Cli::new("ethernetip-read-tag-typed", "Reads a tag as a specific CIP type.")
        .flag("addr", "192.168.1.10:44818", "PLC address in host:port format")
        .flag("tag", "", "Tag name on the PLC")
        .flag("type", "DINT", &format!("CIP type: {SUPPORTED}"))
        .flag("count", "1", "Number of elements to read (1 for scalar, >1 for array)")
        .parse_env();
    let addr = args.str("addr").to_string();
    let tag = args.str("tag").to_string();
    let type_name = args.str("type").to_string();
    let elements: u16 = args.get("count");

    if tag.is_empty() {
        eprintln!("error: -tag is required");
        std::process::exit(1);
    }

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

    // Ref: CIP Vol. 1, Appendix C elementary data types (section unverified).
    match type_name.to_uppercase().as_str() {
        "BOOL" => scalar::<bool>(&client, &ctx, &tag, elements, "BOOL", "bool", false),
        "SINT" => scalar::<i8>(&client, &ctx, &tag, elements, "SINT", "i8", false),
        "INT" => scalar::<i16>(&client, &ctx, &tag, elements, "INT", "i16", false),
        "DINT" => scalar::<i32>(&client, &ctx, &tag, elements, "DINT", "i32", false),
        "LINT" => scalar::<i64>(&client, &ctx, &tag, elements, "LINT", "i64", false),
        "USINT" => scalar::<u8>(&client, &ctx, &tag, elements, "USINT", "u8", false),
        "UINT" => scalar::<u16>(&client, &ctx, &tag, elements, "UINT", "u16", false),
        "UDINT" => scalar::<u32>(&client, &ctx, &tag, elements, "UDINT", "u32", false),
        "ULINT" => scalar::<u64>(&client, &ctx, &tag, elements, "ULINT", "u64", false),
        "REAL" => scalar::<f32>(&client, &ctx, &tag, elements, "REAL", "f32", true),
        "LREAL" => scalar::<f64>(&client, &ctx, &tag, elements, "LREAL", "f64", true),

        // Ref: Rockwell Logix TIMER structure (PRE/ACC DINT ms, EN/TT/DN status bits)
        // (1756-PM020, section unverified).
        "TIMER" => {
            println!("--- Using ReadTagInto for Timer ---");
            let mut timer = cip::Timer::default();
            if let Err(e) = client.read_tag_into(&ctx, &tag, &mut timer) {
                cli::fatal(format!("ReadTagInto (Timer) failed: {e}"));
            }
            println!("Timer:");
            println!("  PRE (preset):      {} ms", timer.pre);
            println!("  ACC (accumulated):  {} ms", timer.acc);
            println!("  EN  (enable):       {}", timer.en);
            println!("  TT  (timer timing): {}", timer.tt);
            println!("  DN  (done):         {}", timer.dn);
        }

        // Ref: Rockwell Logix COUNTER structure (PRE/ACC DINT, CU/CD/DN/OV/UN bits)
        // (1756-PM020, section unverified).
        "COUNTER" => {
            println!("--- Using ReadTagInto for Counter ---");
            let mut counter = cip::Counter::default();
            if let Err(e) = client.read_tag_into(&ctx, &tag, &mut counter) {
                cli::fatal(format!("ReadTagInto (Counter) failed: {e}"));
            }
            println!("Counter:");
            println!("  PRE (preset):     {}", counter.pre);
            println!("  ACC (accumulated): {}", counter.acc);
            println!("  CU  (count up):    {}", counter.cu);
            println!("  CD  (count down):  {}", counter.cd);
            println!("  DN  (done):        {}", counter.dn);
            println!("  OV  (overflow):    {}", counter.ov);
            println!("  UN  (underflow):   {}", counter.un);
        }

        _ => {
            eprintln!("Unsupported type: {type_name}");
            eprintln!("Supported: {SUPPORTED}");
            std::process::exit(1);
        }
    }

    println!("\nDone.");
    let _ = client.close();
}

// Implementation detail (not spec): one generic arm replaces the Go per-type copy/paste
// (`Read[T]` for one element, `ReadSlice[T]` for more). Floats print with `%f` formatting
// for scalars like the Go example.
fn scalar<T>(client: &Client, ctx: &Context, tag: &str, elements: u16, name: &str, rust_ty: &str, is_float: bool)
where
    T: Unmarshaler + SizedCip + Default + Clone + Display + Debug,
{
    if elements == 1 {
        match client::read_typed::<T>(client, ctx, tag) {
            Ok(v) => {
                if is_float {
                    // Go `%f` -> six decimals.
                    let s = format!("{v}");
                    let shown = s.parse::<f64>().map(|f| format!("{f:.6}")).unwrap_or(s);
                    println!("{name} value: {shown}");
                } else {
                    println!("{name} value: {v}");
                }
            }
            Err(e) => cli::fatal(format!("Read[{rust_ty}] failed: {e}")),
        }
    } else {
        match client::read_slice::<T>(client, ctx, tag, elements) {
            Ok(v) => println!("{name} values: {}", cli::fmt_list(&v)),
            Err(e) => cli::fatal(format!("ReadSlice[{rust_ty}] failed: {e}")),
        }
    }
}
