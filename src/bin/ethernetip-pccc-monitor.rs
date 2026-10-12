//! ethernetip-pccc-monitor: polls PCCC addresses and prints changes. Port of Go
//! `examples/ethernetip/pccc_monitor`.
//!
//! The Go example used the separate `monitor` package (subscriptions, subscriber channels),
//! which is outside this crate. This port keeps the observable behavior with a small
//! hand-written poller: each address is read every `-freq`, and a line is printed whenever the
//! value changes (or a read fails). Ctrl+C stops it.
//!
//! Ref: Allen-Bradley PCCC / DF1 protocol manual 1770-6.5.16 (Protected Typed Logical Read) via
//! `pccc::Client::read_address`.
//!
//! Usage:
//!   ethernetip-pccc-monitor -addr 10.30.40.71:44818 -tags N7:0,F8:0,B3:0/1 -freq 250ms

use std::sync::Arc;
use std::time::{Duration, Instant};

use rsethernetip::cli::{self, Cli};
use rsethernetip::client;
use rsethernetip::options::{with_retries, with_retry_delay};
use rsethernetip::pccc;
use rsethernetip::plc::{Context, DataType, Value};

fn main() {
    let args = Cli::new("ethernetip-pccc-monitor", "Polls PCCC addresses and prints changes.")
        .flag("addr", "10.30.40.71:44818", "SLC/MicroLogix address (host:port)")
        .flag("tags", "N7:0", "comma-separated PCCC addresses to poll")
        .flag("freq", "500ms", "poll frequency per address")
        .parse_env();
    let addr = args.str("addr").to_string();
    let freq = args.duration("freq");
    let tags: Vec<String> = args.str("tags").split(',').map(|t| t.trim().to_string()).collect();
    if tags.is_empty() || tags[0].is_empty() {
        eprintln!("error: -tags is required");
        std::process::exit(1);
    }

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

    // Validate the addresses up front, like Subscribe does.
    for t in &tags {
        if let Err(e) = pccc::parse_address(t) {
            cli::fatal(format!("subscribe {t:?}: {e}"));
        }
        println!("  subscribed {t} @ {}", fmt_duration(freq));
    }

    cli::install_interrupt_handler();
    println!("Polling - Ctrl-C to stop.");

    // Implementation detail (not spec): per-address last value, and the next due time.
    let mut last: Vec<Option<Vec<u8>>> = vec![None; tags.len()];
    let mut due: Vec<Instant> = vec![Instant::now(); tags.len()];
    while !cli::interrupted() {
        let now = Instant::now();
        for (i, t) in tags.iter().enumerate() {
            if now < due[i] {
                continue;
            }
            due[i] = now + freq;
            // Each read has its own deadline so a hung PLC does not wedge the loop forever.
            let rctx = Context::background().with_timeout(Duration::from_secs(10));
            let ts = cli::clock_hms();
            match client.read_address(&rctx, t) {
                Err(e) => println!("[{ts}] {t}: error: {e}"),
                Ok(v) => {
                    // First sample counts as a change (Go emitted an initial event).
                    if last[i].as_deref() != Some(v.raw.as_slice()) {
                        println!("[{ts}] {t} = {}", format_value(&v));
                        last[i] = Some(v.raw.clone());
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    println!("\nstopping...");
}

// Implementation detail (not spec): Go `formatValue`.
fn format_value(v: &Value) -> String {
    match v.data_type {
        DataType::Bool => return v.bool().to_string(),
        DataType::Float32 => {
            if let Ok(f) = v.float32() {
                return f.to_string();
            }
        }
        DataType::Int16 => {
            if let Ok(i) = v.int() {
                return i.to_string();
            }
        }
        _ => {}
    }
    cli::hex_spaced(&v.raw)
}

// Implementation detail (not spec): Go `time.Duration.String()` for the common cases.
fn fmt_duration(d: Duration) -> String {
    if d.subsec_nanos() == 0 {
        format!("{}s", d.as_secs())
    } else {
        format!("{}ms", d.as_millis())
    }
}
