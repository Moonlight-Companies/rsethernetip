//! ethernetip-hexdump: wire-level hex dump tracing of EtherNet/IP traffic. Port of Go
//! `examples/ethernetip/hexdump`.
//!
//! Every byte read from or written to the TCP connection is printed in traditional hexdump
//! format, showing the complete EIP encapsulation frames (24-byte header, CPF items and the CIP
//! payload). Output goes to stdout, and optionally also to a trace file.
//!
//! Ref: CIP Vol. 2, Section 2-3 Encapsulation Message (24-byte header) and Section 2-6 Common
//! Packet Format - the structures visible in the dump.
//!
//! Usage:
//!   ethernetip-hexdump -addr 192.168.1.10:44818 -tag MyDINT
//!   ethernetip-hexdump -addr 192.168.1.10:44818 -tag MyDINT -log trace.hex

use std::fs::File;
use std::io::{self, Write};
use std::time::Duration;

use rsethernetip::cli::{self, Cli};
use rsethernetip::client;
use rsethernetip::options::{with_hex_dump, with_retries};
use rsethernetip::plc::Context;

// Implementation detail (not spec): Go `io.MultiWriter(os.Stdout, f)`.
struct Tee {
    file: File,
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::stdout().write_all(buf)?;
        self.file.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()?;
        self.file.flush()
    }
}

fn main() {
    let args = Cli::new("ethernetip-hexdump", "Hex dump tracing of EtherNet/IP traffic.")
        .flag("addr", "192.168.1.10:44818", "PLC address (host:port)")
        .flag("tag", "MyDINT", "Tag name to read")
        .flag("log", "", "Write hex dump to this file (in addition to stdout)")
        .parse_env();
    let addr = args.str("addr").to_string();
    let tag = args.str("tag").to_string();
    let log_file = args.str("log").to_string();

    // Set up the hex dump writer: stdout, or stdout + file.
    let opt = if log_file.is_empty() {
        with_hex_dump(io::stdout())
    } else {
        let f = match File::create(&log_file) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Failed to create log file: {e}");
                std::process::exit(1);
            }
        };
        println!("Hex dump output also written to {log_file}\n");
        with_hex_dump(Tee { file: f })
    };

    let ctx = Context::background().with_timeout(Duration::from_secs(10));
    println!("Connecting to {addr}...\n");

    // The dump includes the RegisterSession handshake at connection time.
    let client = match client::connect(&ctx, &addr, vec![with_retries(2).into(), opt.into()]) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect: {e}");
            std::process::exit(1);
        }
    };

    // The dump captures the SendRRData exchange (CIP Vol. 2, Section 2-3: SendRRData 0x006F).
    println!("--- Reading tag ---");
    println!();
    let data = match client.read_tag(&ctx, &tag) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ReadTag: {e}");
            std::process::exit(1);
        }
    };

    println!();
    println!("--- Decoded ---");
    println!("  Tag:  {tag}");
    println!("  Data: {}", cli::hex_spaced(&data));
    println!();
    println!("Done.");
    let _ = client.close();
}
