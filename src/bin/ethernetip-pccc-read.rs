//! ethernetip-pccc-read: reads a single PCCC data-table address from an SLC 500 / MicroLogix /
//! PLC-5 over EtherNet/IP, showing every step. Port of Go `examples/ethernetip/pccc_read`.
//!
//! The PCCC command rides inside CIP Execute_PCCC (service 0x4B on the PCCC object, class 0x67);
//! the EtherNet/IP client wraps it with the requestor ID (vendor, serial).
//!
//! Ref: Allen-Bradley PCCC / DF1 protocol manual 1770-6.5.16 - Protected Typed Logical Read with
//! 3 address fields (CMD 0x0F, FNC 0xA2); CIP Execute_PCCC service 0x4B (CIP Vol. 1 PCCC object,
//! section unverified).
//!
//! Usage:
//!   ethernetip-pccc-read -addr 10.30.40.71:44818 -tag N7:0
//!   ethernetip-pccc-read -addr 10.30.40.71:44818 -tag F8:5
//!   ethernetip-pccc-read -addr 10.30.40.71:44818 -tag B3:0/2
//!   ethernetip-pccc-read -addr 10.30.40.71:44818 -tag T4:0.ACC

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rsethernetip::cli::{self, Cli};
use rsethernetip::client;
use rsethernetip::options::{with_retries, with_retry_delay};
use rsethernetip::pccc::{self, Address, FileType, PcccError};
use rsethernetip::plc::Context;

fn main() {
    let args = Cli::new("ethernetip-pccc-read", "Reads one PCCC data-table address over EtherNet/IP.")
        .flag("addr", "10.30.40.71:44818", "SLC/MicroLogix address in host:port format (PCCC default port is 44818)")
        .flag("tag", "N7:0", "PCCC data-table address (e.g. N7:0, F8:5, B3:0/2, T4:0.ACC, S:1)")
        .parse_env();
    let addr = args.str("addr").to_string();
    let tag = args.str("tag").to_string();
    if tag.is_empty() {
        eprintln!("error: -tag is required");
        std::process::exit(1);
    }

    // Ref: PCCC data-table address syntax (Allen-Bradley SLC 500 / MicroLogix instruction set
    // reference, section unverified).
    let a = match pccc::parse_address(&tag) {
        Ok(a) => a,
        Err(e) => cli::fatal(format!("parse address {tag:?}: {e}")),
    };
    println!("Parsed address: {a}");
    println!("  FileType:   {}", a.file_type);
    println!("  FileNumber: {}", a.file_number);
    println!("  Element:    {}", a.element);
    println!("  SubElement: {}", a.sub_element);
    if a.bit_num >= 0 {
        println!("  Bit:        {}", a.bit_num);
    }

    let byte_size = element_bytes(&a);
    println!("  Read size:  {byte_size} bytes");

    let ctx = Context::background().with_timeout(Duration::from_secs(10));
    let client = match client::connect(
        &ctx,
        &addr,
        vec![with_retries(2).into(), with_retry_delay(Duration::from_millis(500)).into()],
    ) {
        Ok(c) => c,
        Err(e) => cli::fatal(format!("connect {addr}: {e}")),
    };
    println!("\nConnected to {addr}");

    // Ref: PCCC manual 1770-6.5.16 - the TNS (transaction number) is a 16-bit value echoed in the reply.
    let tns = (SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) & 0xFFFF) as u16;
    let cmd = match pccc::encode_typed_read(tns, byte_size, a.file_number, a.file_type, a.element, a.sub_element) {
        Ok(c) => c,
        Err(e) => cli::fatal(format!("encode PCCC read: {e}")),
    };
    println!("PCCC request:   {}", cli::hex_spaced(&cmd));

    let raw_reply = match client.execute_pccc(&ctx, &cmd) {
        Ok(r) => r,
        Err(e) => cli::fatal(format!("ExecutePCCC: {e}")),
    };
    println!("PCCC reply:     {}", cli::hex_spaced(&raw_reply));

    let reply = match pccc::decode_reply(&raw_reply) {
        Ok(r) => r,
        Err(PcccError::Status(pe)) => cli::fatal(format!("PCCC error: {pe}")),
        Err(e) => cli::fatal(format!("decode PCCC reply: {e}")),
    };
    if reply.tns != tns {
        eprintln!("warning: TNS mismatch (sent 0x{tns:04X}, got 0x{:04X})", reply.tns);
    }

    println!();
    println!("--- Decoded value ---");
    print_value(&a, &reply.data);
    let _ = client.close();
}

// Ref: PCCC manual 1770-6.5.16 - element sizes: N/B/S/I/O words are 2 bytes, F is 4 bytes,
// T/C/R structures are 3 words (6 bytes) and a sub-element is one word.
fn element_bytes(a: &Address) -> i64 {
    match a.file_type {
        FileType::FLOAT => 4,
        FileType::TIMER | FileType::COUNTER | FileType::CONTROL => {
            if a.sub_element > 0 || a.bit_num >= 0 {
                2
            } else {
                6
            }
        }
        _ => 2,
    }
}

// Implementation detail (not spec): decodes the reply payload the way the Go example does.
fn print_value(a: &Address, data: &[u8]) {
    match a.file_type {
        FileType::FLOAT => {
            if data.len() < 4 {
                println!("(short reply for F: {} bytes)", data.len());
                return;
            }
            let f = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            println!("{a} = {f}  (REAL)");
            return;
        }
        FileType::TIMER | FileType::COUNTER | FileType::CONTROL if a.sub_element == 0 && a.bit_num < 0 => {
            if data.len() < 6 {
                println!("(short reply for full {} element: {} bytes)", a.file_type.letter(), data.len());
                return;
            }
            let ctl = u16::from_le_bytes([data[0], data[1]]);
            let pre = i16::from_le_bytes([data[2], data[3]]);
            let acc = i16::from_le_bytes([data[4], data[5]]);
            println!("{a} = {{ control: 0x{ctl:04X}, PRE: {pre}, ACC: {acc} }}");
            return;
        }
        _ => {}
    }
    if data.len() < 2 {
        println!("(short reply: {} bytes)", data.len());
        return;
    }
    let word = u16::from_le_bytes([data[0], data[1]]);
    if a.bit_num >= 0 {
        let bit = (word >> (a.bit_num as u32 & 15)) & 1;
        println!("{a} = {bit}  (bit {} of 0x{word:04X})", a.bit_num);
        return;
    }
    println!("{a} = {} (0x{word:04X})", word as i16);
}
