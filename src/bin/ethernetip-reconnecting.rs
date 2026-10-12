//! ethernetip-reconnecting: a polling loop that survives connection loss with a reconnecting
//! transport, infinite retries and lifecycle hooks. Port of Go `examples/ethernetip/reconnecting`.
//!
//! Layers: `SessionConnector` dials TCP + RegisterSession; `ReconnectingTransport` keeps one
//! session and re-dials after a failure; the `Client` retries transport errors (never CIP status
//! errors) with `-1` meaning forever.
//!
//! Ref: CIP Vol. 2, Section 2-3 Encapsulation Message - RegisterSession (0x0065) /
//! UnRegisterSession (0x0066) per connection; Rockwell 1756-PM020 Logix 5000 Data Access - Read
//! Tag (0x4C); CIP Vol. 1, Chapter 3 General Status Codes (section unverified).
//!
//! Usage:
//!   ethernetip-reconnecting -addr 127.0.0.1:44818 -tag MyDINT -interval 2s

use std::sync::Arc;
use std::time::Duration;

use rsethernetip::cip;
use rsethernetip::cli::{self, Cli};
use rsethernetip::client::{new_client, Client};
use rsethernetip::conn;
use rsethernetip::connector::{SessionCloser, SessionConnector};
use rsethernetip::logging::{DefaultLogger, Level, Logger};
use rsethernetip::options::{with_logger, with_retries, with_retry_delay};
use rsethernetip::plc::Context;
use rsethernetip::transport::{with_on_connect, with_on_disconnect, ReconnectingTransport};

fn main() {
    let args = Cli::new("ethernetip-reconnecting", "Polls a tag over a reconnecting transport.")
        .flag("addr", "127.0.0.1:44818", "EtherNet/IP server address (host:port)")
        .flag("tag", "MyDINT", "Tag name to read")
        .flag("interval", "2s", "Polling interval (e.g., 500ms, 2s, 10s)")
        .parse_env();
    let addr = args.str("addr").to_string();
    let tag = args.str("tag").to_string();
    let interval = args.duration("interval");

    let logger: Arc<dyn Logger> = Arc::new(DefaultLogger::builder().level(Level::Info).build());

    // The connector knows how to dial; the closer knows how to tear a session down.
    let connector = SessionConnector::new(addr.clone(), Some(logger.clone()), vec![]);

    // Lifecycle hooks (implementation detail, not spec): fire on connect and disconnect.
    let (l1, a1) = (logger.clone(), addr.clone());
    let l2 = logger.clone();
    let tp = ReconnectingTransport::new(
        Arc::new(connector),
        Arc::new(SessionCloser),
        &[
            with_on_connect(move || l1.info(format_args!("HOOK: EIP session established with {a1}"))),
            with_on_disconnect(move |err| match err {
                Some(e) => l2.warn(format_args!("HOOK: EIP session lost: {e}")),
                None => l2.info(format_args!("HOOK: EIP session closed cleanly")),
            }),
        ],
    );

    // Infinite retries (-1) for transport errors, 2 s between attempts.
    let client = new_client(
        Arc::new(tp),
        &[with_retries(-1), with_retry_delay(Duration::from_secs(2)), with_logger(logger.clone())],
    );

    // Ctrl+C cancels the context so a blocked infinite retry loop aborts too.
    let ctx = Context::new_cancellable();
    {
        let c = ctx.clone();
        cli::on_interrupt(move || {
            println!("\nReceived signal. Shutting down...");
            c.cancel();
        });
    }

    println!("Starting polling loop: reading tag {tag:?} every {}ms", interval.as_millis());
    println!("Press Ctrl+C to stop.");
    println!();

    read_and_print(&ctx, &client, &tag);
    while !ctx.is_cancelled() {
        // Sleep in slices so cancellation is noticed promptly.
        let mut waited = Duration::ZERO;
        while waited < interval && !ctx.is_cancelled() {
            let step = Duration::from_millis(50).min(interval - waited);
            std::thread::sleep(step);
            waited += step;
        }
        if ctx.is_cancelled() {
            break;
        }
        read_and_print(&ctx, &client, &tag);
    }
    println!("\nPolling loop stopped.");

    println!("Closing client...");
    if let Err(e) = client.close() {
        eprintln!("Error closing client: {e}");
    }
    println!("Done.");
}

// Implementation detail (not spec): walks `Wrapped` errors to the CIP status error (Go errors.As).
fn find_cip(e: &conn::Error) -> Option<&cip::Error> {
    match e {
        conn::Error::Cip(c) => Some(c),
        conn::Error::Wrapped { source, .. } => find_cip(source),
        _ => None,
    }
}

fn read_and_print(ctx: &Context, client: &Client, tag: &str) {
    let now = cli::clock_hms();
    let data = match client.read_tag(ctx, tag) {
        Ok(d) => d,
        Err(e) => {
            // CIP-level errors (the PLC answered with an error) are never retried.
            if let Some(c) = find_cip(&e) {
                println!("[{now}] CIP ERROR reading {tag:?}: status=0x{:02X} ({c})", c.status);
                // Ref: CIP Vol. 1, Appendix B General Status Codes (section unverified).
                match c.status {
                    cip::STATUS_PATH_DESTINATION_UNKNOWN => println!("  -> Tag does not exist on the PLC. Check the tag name."),
                    cip::STATUS_SERVICE_NOT_SUPPORTED => println!("  -> The target object does not support the ReadTag service."),
                    cip::STATUS_OBJECT_DOES_NOT_EXIST => println!("  -> The target CIP object class does not exist."),
                    _ => {}
                }
                return;
            }
            // A cancelled context is a normal shutdown, not an error to report.
            if ctx.is_done() {
                return;
            }
            println!("[{now}] TRANSPORT ERROR reading {tag:?}: {e}");
            return;
        }
    };

    if data.len() < 2 {
        println!("[{now}] Unexpected: response too short ({} bytes)", data.len());
        return;
    }
    let data_type = cip::DataType(u16::from_le_bytes([data[0], data[1]]));
    let value = format_value(data_type, &data[2..]);
    println!("[{now}] {tag} ({data_type}) = {value}");
}

// Ref: CIP Vol. 1, Appendix C data types (section unverified); STRING here is a 16-bit length
// plus characters, as in the Go example's simulated server.
fn format_value(dt: cip::DataType, v: &[u8]) -> String {
    match dt {
        cip::TYPE_DINT if v.len() >= 4 => return i32::from_le_bytes([v[0], v[1], v[2], v[3]]).to_string(),
        cip::TYPE_REAL if v.len() >= 4 => {
            return format!("{:.4}", f32::from_le_bytes([v[0], v[1], v[2], v[3]]));
        }
        cip::TYPE_INT if v.len() >= 2 => return i16::from_le_bytes([v[0], v[1]]).to_string(),
        cip::TYPE_STRING if v.len() >= 2 => {
            let n = u16::from_le_bytes([v[0], v[1]]) as usize;
            if v.len() >= 2 + n {
                return format!("{:?}", String::from_utf8_lossy(&v[2..2 + n]));
            }
        }
        cip::TYPE_BOOL if !v.is_empty() => return (v[0] != 0).to_string(),
        _ => {}
    }
    format!("0x{} (type=0x{:04X})", cli::hex_plain(v), dt.0)
}
