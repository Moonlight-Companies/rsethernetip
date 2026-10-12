//! ethernetip-list-identity: queries an EtherNet/IP device with ListIdentity and ListServices.
//! Port of Go `examples/ethernetip/list_identity`.
//!
//! Ref: CIP Vol. 2, Section 2-3 Encapsulation Message - ListIdentity (0x0063) and ListServices
//! (0x0004) commands; their reply items are CIP Identity data (CIP Vol. 1, Chapter 5 Identity
//! Object, section unverified) and the Communications Service item (type 0x0100).
//!
//! Usage:
//!   ethernetip-list-identity -addr 192.168.1.10:44818

use std::time::Duration;

use rsethernetip::cli::{self, Cli};
use rsethernetip::client;
use rsethernetip::options::{with_retries, with_retry_delay};
use rsethernetip::plc::Context;

fn main() {
    let args = Cli::new("ethernetip-list-identity", "Queries ListIdentity and ListServices.")
        .flag("addr", "192.168.1.10:44818", "EtherNet/IP device address in host:port format")
        .parse_env();
    let addr = args.str("addr").to_string();

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

    println!("=== ListIdentity ===");
    println!();
    let identities = match client.list_identity(&ctx) {
        Ok(i) => i,
        Err(e) => cli::fatal(format!("ListIdentity failed: {e}")),
    };
    if identities.is_empty() {
        println!("No identity items returned.");
    }
    for (i, id) in identities.iter().enumerate() {
        if identities.len() > 1 {
            println!("--- Device {} ---", i + 1);
        }
        println!("  Product Name:   {}", id.product_name);
        println!("  Vendor ID:      {}", id.vendor_id);
        println!("  Device Type:    {}", id.device_type);
        println!("  Product Code:   {}", id.product_code);
        println!("  Revision:       {}.{}", id.revision[0], id.revision[1]);
        println!("  Serial Number:  0x{:08X}", id.serial_number);
        println!("  Status:         0x{:04X}", id.status);
        println!("  State:          {}", id.state);

        // Ref: ODVA vendor ID list (CIP Vol. 1, Appendix B Object/Vendor lists, section unverified).
        match id.vendor_id {
            1 => println!("  Vendor:         Rockwell Automation / Allen-Bradley"),
            9 => println!("  Vendor:         Schneider Electric"),
            47 => println!("  Vendor:         WAGO"),
            283 => println!("  Vendor:         Beckhoff"),
            _ => {}
        }
        // Ref: CIP Vol. 1, Chapter 6 Device Profiles - device type codes (section unverified).
        match id.device_type {
            0x00 => println!("  Device Class:   Generic Device"),
            0x0C => println!("  Device Class:   Communications Adapter"),
            0x0E => println!("  Device Class:   Programmable Logic Controller"),
            0x21 => println!("  Device Class:   CIP Motion Drive"),
            _ => {}
        }
        println!();
    }

    println!("=== ListServices ===");
    println!();
    let services = match client.list_services(&ctx) {
        Ok(s) => s,
        Err(e) => cli::fatal(format!("ListServices failed: {e}")),
    };
    if services.is_empty() {
        println!("No service items returned.");
    }
    for svc in &services {
        println!("  Service Name:    {}", svc.name);
        println!("  Type ID:         0x{:04X}", svc.type_id);
        println!("  Version:         {}", svc.version);
        println!("  Capabilities:    0x{:04X}", svc.capability_flags);

        // Ref: CIP Vol. 2, ListServices Capability Flags: bit 5 = CIP encapsulation via TCP,
        // bit 8 = CIP class 0/1 via UDP (section unverified).
        if svc.capability_flags & (1 << 5) != 0 {
            println!("    - Supports CIP encapsulation via TCP");
        }
        if svc.capability_flags & (1 << 8) != 0 {
            println!("    - Supports CIP encapsulation via UDP");
        }
        println!();
    }

    println!("Done.");
    let _ = client.close();
}
