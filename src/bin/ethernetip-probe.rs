//! ethernetip-probe: probes an EtherNet/IP device and prints what it exposes. Port of Go
//! `examples/ethernetip/probe`.
//!
//! Sections: device identity (Identity class 0x01), TCP/IP Interface (0xF5), Ethernet Link
//! (0xF6), Assembly instances (0x04), Connection Manager counters (0x06), a scan of common CIP
//! object classes, and optionally (`-tags`) Logix tags with sample values.
//!
//! Ref: CIP Vol. 1, Chapter 5 Object Library - Identity Object (class 0x01) Get_Attributes_All
//! layout, Assembly Object (class 0x04), Connection Manager (0x06); CIP Vol. 2, Chapter 5
//! TCP/IP Interface Object (0xF5) and Ethernet Link Object (0xF6) (chapter numbers
//! unverified); Rockwell 1756-PM020 Logix 5000 Data Access for the Symbol Object (0x6B).
//!
//! Usage:
//!   ethernetip-probe <PLC_IP> [-assemblies 255] [-tags] [-timeout 5s]

use std::time::Duration;

use rsethernetip::cip::{
    build_path, DataType, MessageRouterRequest, MessageRouterResponse, Path, CLASS_ASSEMBLY, CLASS_IDENTITY,
    SERVICE_GET_ATTRIBUTE_ALL, SERVICE_GET_ATTRIBUTE_SINGLE, TYPE_BOOL, TYPE_DINT, TYPE_INT, TYPE_LINT, TYPE_LREAL,
    TYPE_REAL, TYPE_SINT, TYPE_STRING, TYPE_STRUCT,
};
use rsethernetip::cli::{self, Cli};
use rsethernetip::client;
use rsethernetip::conn::{self, TcpConn};
use rsethernetip::plc::Context;
use rsethernetip::session::Session;

// Implementation detail (not spec): section banner used throughout.
const BAR: &str = "═══════════════════════════════════════════════════════════════";

fn banner(title: &str) {
    println!();
    println!("{BAR}");
    println!("  {title}");
    println!("{BAR}");
}

fn main() {
    let args = Cli::new("ethernetip-probe", "Probes an EtherNet/IP device.")
        .positional("<PLC_IP>")
        .flag("assemblies", "255", "Max assembly instance to probe")
        .switch("tags", "Attempt to list tags (Logix controllers only)")
        .flag("timeout", "5s", "Connection timeout")
        .parse_env();
    let max_assembly: u16 = args.get("assemblies");
    let probe_tags = args.on("tags");
    let timeout = args.duration("timeout");
    let addr = match args.positional().first() {
        Some(a) => a.clone(),
        None => {
            eprintln!("Usage: ethernetip-probe <PLC_IP> [flags]");
            std::process::exit(1);
        }
    };

    let ctx = Context::background().with_timeout(timeout);

    // Go appends the default port when there is no ':' at all.
    let tcp_addr = if addr.contains(':') { addr.clone() } else { format!("{addr}:44818") };

    let tc = match TcpConn::new(&tcp_addr, &[]) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Connect to {addr}: {e}");
            std::process::exit(1);
        }
    };
    let sess = Session::new(tc, None);
    if let Err(e) = sess.register(&ctx) {
        eprintln!("Register session: {e}");
        std::process::exit(1);
    }

    // Sends one explicit CIP request and returns the reply (Go `cipReq`).
    let cip_req = |service: u8, path: Path, data: Vec<u8>| -> conn::Result<MessageRouterResponse> {
        sess.send_cip_request(&ctx, &MessageRouterRequest { service, request_path: path, request_data: data })
    };
    // Successful reply data with at least `min` bytes, else None.
    let get = |service: u8, path: Path, min: usize| -> Option<Vec<u8>> {
        match cip_req(service, path, Vec::new()) {
            Ok(r) if r.is_success() && r.response_data.len() >= min => Some(r.response_data),
            _ => None,
        }
    };

    // --- Device identity ---
    // Ref: CIP Vol. 1, Identity Object Get_Attributes_All: Vendor ID(2) Device Type(2) Product
    // Code(2) Revision(2) Status(2) Serial(4) Product Name (SHORT_STRING) (section unverified).
    banner("DEVICE IDENTITY");
    match get(SERVICE_GET_ATTRIBUTE_ALL, build_path(CLASS_IDENTITY, 1, 0), 15) {
        Some(d) => {
            let vendor_id = u16::from_le_bytes([d[0], d[1]]);
            let dev_type = u16::from_le_bytes([d[2], d[3]]);
            let prod_code = u16::from_le_bytes([d[4], d[5]]);
            let (rev_major, rev_minor) = (d[6], d[7]);
            let status = u16::from_le_bytes([d[8], d[9]]);
            let serial = u32::from_le_bytes([d[10], d[11], d[12], d[13]]);
            let name_len = d[14] as usize;
            let name = d.get(15..15 + name_len).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();

            println!("  Product Name:    {name}");
            println!("  Vendor ID:       {vendor_id} ({})", vendor_name(vendor_id));
            println!("  Device Type:     {dev_type} ({})", device_type_name(dev_type));
            println!("  Product Code:    {prod_code}");
            println!("  Revision:        {rev_major}.{rev_minor}");
            println!("  Serial Number:   0x{serial:08X}");
            println!("  Status:          0x{status:04X}");
        }
        None => println!("  (identity read failed)"),
    }

    // --- TCP/IP Interface object (0xF5) ---
    // Ref: CIP Vol. 2, TCP/IP Interface Object attributes 1 Status, 3 Configuration Capability,
    // 5 Interface Configuration, 6 Host Name (section unverified).
    banner("TCP/IP INTERFACE");
    if let Some(d) = get(SERVICE_GET_ATTRIBUTE_SINGLE, build_path(0xF5, 1, 1), 4) {
        println!("  Interface Status: 0x{:08X}", u32::from_le_bytes([d[0], d[1], d[2], d[3]]));
    }
    if let Some(d) = get(SERVICE_GET_ATTRIBUTE_SINGLE, build_path(0xF5, 1, 3), 4) {
        let cap = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
        print!("  Config Capability: 0x{cap:08X}");
        let mut flags: Vec<&str> = Vec::new();
        if cap & 0x01 != 0 {
            flags.push("BOOTP");
        }
        if cap & 0x02 != 0 {
            flags.push("DNS");
        }
        if cap & 0x04 != 0 {
            flags.push("DHCP");
        }
        if cap & 0x10 != 0 {
            flags.push("HW-Configurable");
        }
        if cap & 0x20 != 0 {
            flags.push("ACD");
        }
        if !flags.is_empty() {
            print!(" [{}]", flags.join(", "));
        }
        println!();
    }
    if let Some(d) = get(SERVICE_GET_ATTRIBUTE_SINGLE, build_path(0xF5, 1, 5), 12) {
        // Addresses are UDINTs in little-endian: the IP's first octet is the most significant byte.
        println!("  IP Address:      {}.{}.{}.{}", d[3], d[2], d[1], d[0]);
        println!("  Subnet Mask:     {}.{}.{}.{}", d[7], d[6], d[5], d[4]);
        println!("  Gateway:         {}.{}.{}.{}", d[11], d[10], d[9], d[8]);
    }
    if let Some(d) = get(SERVICE_GET_ATTRIBUTE_SINGLE, build_path(0xF5, 1, 6), 2) {
        let n = u16::from_le_bytes([d[0], d[1]]) as usize;
        if let Some(b) = d.get(2..2 + n) {
            let hostname = String::from_utf8_lossy(b);
            if !hostname.is_empty() {
                println!("  Hostname:        {hostname}");
            }
        }
    }

    // --- Ethernet Link object (0xF6) ---
    // Ref: CIP Vol. 2, Ethernet Link Object attributes 1 Interface Speed, 2 Interface Flags,
    // 3 Physical Address (section unverified).
    banner("ETHERNET LINK");
    if let Some(d) = get(SERVICE_GET_ATTRIBUTE_SINGLE, build_path(0xF6, 1, 1), 4) {
        println!("  Interface Speed: {} Mbps", u32::from_le_bytes([d[0], d[1], d[2], d[3]]));
    }
    if let Some(d) = get(SERVICE_GET_ATTRIBUTE_SINGLE, build_path(0xF6, 1, 2), 4) {
        let flags = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
        let duplex = if flags & 0x01 != 0 { "full" } else { "half" };
        let active = if flags & 0x02 != 0 { "active" } else { "inactive" };
        println!("  Link Status:     {active}, {duplex} duplex");
    }
    if let Some(d) = get(SERVICE_GET_ATTRIBUTE_SINGLE, build_path(0xF6, 1, 3), 6) {
        println!("  MAC Address:     {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}", d[0], d[1], d[2], d[3], d[4], d[5]);
    }

    // --- Assembly instances ---
    // Ref: CIP Vol. 1, Assembly Object attribute 3 (Data).
    banner("ASSEMBLY INSTANCES");
    let mut found = 0;
    for inst in 1..=max_assembly {
        let data = match cip_req(SERVICE_GET_ATTRIBUTE_SINGLE, build_path(CLASS_ASSEMBLY, inst, 3), Vec::new()) {
            Ok(r) if r.is_success() => r.response_data,
            _ => continue,
        };
        found += 1;
        print!("  Instance {inst:3}:  {:3} bytes", data.len());
        let preview = &data[..data.len().min(24)];
        print!("  [{}", cli::hex_spaced(preview));
        if data.len() > 24 {
            print!("...");
        }
        print!("]");
        if data.len() == 12 {
            let w0 = u16::from_le_bytes([data[0], data[1]]);
            let w1 = u16::from_le_bytes([data[2], data[3]]);
            print!("  (word0=0x{w0:04X} word1=0x{w1:04X})");
        }
        println!();
    }
    if found == 0 {
        println!("  (none found — device may need I/O module configured)");
    } else {
        println!("  Total: {found} instances");
    }

    // --- Connection Manager ---
    // Ref: CIP Vol. 1, Connection Manager Object attributes 1..5: Open Requests, Open Format
    // Rejects, Open Resource Rejects, Open Other Rejects, Close Requests (section unverified).
    banner("CONNECTION MANAGER");
    for (attr, label) in [
        (1u16, "Open Requests:  "),
        (2, "Format Rejects: "),
        (3, "Resource Rejects:"),
        (4, "Other Rejects:  "),
        (5, "Close Requests: "),
    ] {
        if let Some(d) = get(SERVICE_GET_ATTRIBUTE_SINGLE, build_path(0x06, 1, attr), 2) {
            println!("  {label} {}", u16::from_le_bytes([d[0], d[1]]));
        }
    }

    // --- Object class scan ---
    // Ref: CIP Vol. 1, Chapter 5 Object Library - class codes (section unverified for individual
    // classes); 0x6B Symbol and 0xB2 are Rockwell classes.
    banner("CIP OBJECT CLASSES");
    let classes: &[(u16, &str)] = &[
        (0x01, "Identity"),
        (0x02, "Message Router"),
        (0x03, "DeviceNet"),
        (0x04, "Assembly"),
        (0x05, "Connection"),
        (0x06, "Connection Manager"),
        (0x07, "Register"),
        (0x08, "Discrete Input Point"),
        (0x09, "Discrete Output Point"),
        (0x0A, "Analog Input Point"),
        (0x0B, "Analog Output Point"),
        (0x0F, "Parameter"),
        (0x10, "Parameter Group"),
        (0x1D, "Discrete Input Group"),
        (0x1E, "Discrete Output Group"),
        (0x1F, "Discrete Group"),
        (0x20, "Analog Input Group"),
        (0x21, "Analog Output Group"),
        (0x22, "Analog Group"),
        (0x28, "Motor Data"),
        (0x29, "Control Supervisor"),
        (0x2A, "AC/DC Drive"),
        (0x2B, "Acknowledge Handler"),
        (0x37, "S-Analog Sensor"),
        (0x38, "S-Analog Actuator"),
        (0x39, "S-Single Stage Controller"),
        (0x3A, "S-Gas Calibration"),
        (0x6B, "Symbol"),
        (0xAC, "IO (Logix)"),
        (0xB2, "Logix5000"),
        (0xF5, "TCP/IP Interface"),
        (0xF6, "Ethernet Link"),
    ];
    for (id, name) in classes {
        // Try instance 0 (class attributes) first, then instance 1.
        let resp = match cip_req(SERVICE_GET_ATTRIBUTE_ALL, build_path(*id, 0, 0), Vec::new()) {
            Ok(r) if r.is_success() => r,
            _ => match cip_req(SERVICE_GET_ATTRIBUTE_ALL, build_path(*id, 1, 0), Vec::new()) {
                Ok(r) if r.is_success() => r,
                _ => continue,
            },
        };
        println!("  0x{id:02X}  {name:<28}  ({} bytes)", resp.response_data.len());
    }

    // --- Tags (Logix) ---
    if probe_tags {
        banner("TAGS (LOGIX)");
        let conn_ctx = Context::background().with_timeout(Duration::from_secs(10));
        match client::connect(&conn_ctx, &tcp_addr, vec![]) {
            Err(e) => println!("  (connect for tag list failed: {e})"),
            Ok(c) => {
                match c.list_tags(&conn_ctx) {
                    Err(e) => println!("  (list tags failed: {e})"),
                    Ok(tags) if tags.is_empty() => println!("  (no tags found)"),
                    Ok(tags) => {
                        println!("  Found {} tags:", tags.len());
                        let show = tags.len().min(50);
                        for t in &tags[..show] {
                            println!("    {:<40}  type=0x{:04X} ({})", t.name, t.type_.0, cip_type_name(t.type_.0));
                        }
                        if tags.len() > show {
                            println!("    ... and {} more", tags.len() - show);
                        }
                    }
                }
                let _ = c.close();
            }
        }
    }

    // --- Sample tag values ---
    if probe_tags {
        banner("SAMPLE TAG VALUES");
        let ctx2 = Context::background().with_timeout(Duration::from_secs(10));
        if let Ok(c2) = client::connect(&ctx2, &tcp_addr, vec![]) {
            let tags = c2.list_tags(&ctx2).unwrap_or_default();
            let mut count = 0;
            for t in &tags {
                if count >= 10 {
                    break;
                }
                if t.type_ == TYPE_DINT || t.type_ == TYPE_REAL || t.type_ == TYPE_INT || t.type_ == TYPE_BOOL {
                    let data = match c2.read_tag(&ctx2, &t.name) {
                        Ok(d) => d,
                        Err(_) => continue,
                    };
                    println!("    {:<40} = {}", t.name, format_tag_value(&data, t.type_));
                    count += 1;
                }
            }
            if count == 0 {
                println!("  (no readable scalar tags found)");
            }
            let _ = c2.close();
        }
    }

    println!();
    println!("{BAR}");
    println!("  PROBE COMPLETE");
    println!("{BAR}");

    let _ = sess.unregister(&Context::background());
    let _ = sess.close();
}

// Ref: ODVA vendor ID list (CIP Vol. 1, Appendix on Vendor IDs, section unverified).
fn vendor_name(id: u16) -> &'static str {
    match id {
        1 => "Rockwell Automation",
        424 => "Teknic",
        _ => "Unknown",
    }
}

// Ref: CIP Vol. 1, Chapter 6 Device Profiles - device type codes (section unverified).
fn device_type_name(dt: u16) -> String {
    match dt {
        0 => "Generic".into(),
        2 => "AC Drive".into(),
        7 => "General Purpose Discrete I/O".into(),
        12 => "Communications Adapter".into(),
        14 => "Programmable Logic Controller".into(),
        22 => "Safety Discrete I/O".into(),
        24 => "HMI".into(),
        33 => "Generic Safety".into(),
        43 => "Servo Drive".into(),
        150 => "AC/DC Drive (vendor specific)".into(),
        _ => format!("Type {dt}"),
    }
}

// Ref: CIP Vol. 1, Appendix C data type codes (section unverified); bit 15 = array, per
// Rockwell 1756-PM020 symbol type reporting.
fn cip_type_name(t: u16) -> String {
    if t & 0x8000 != 0 {
        return format!("{}[]", cip_type_name(t & 0x0FFF));
    }
    match DataType(t) {
        TYPE_BOOL => "BOOL".into(),
        TYPE_SINT => "SINT".into(),
        TYPE_INT => "INT".into(),
        TYPE_DINT => "DINT".into(),
        TYPE_LINT => "LINT".into(),
        TYPE_REAL => "REAL".into(),
        TYPE_LREAL => "LREAL".into(),
        TYPE_STRING => "STRING".into(),
        _ if t >= 0x0100 => "STRUCT".into(),
        _ => format!("0x{t:04X}"),
    }
}

// Ref: Rockwell 1756-PM020 Read Tag reply: 2-byte type code (4 bytes for structures), then data.
fn format_tag_value(data: &[u8], dt: DataType) -> String {
    if data.len() < 2 {
        return cli::hex_spaced(data);
    }
    let type_code = DataType(u16::from_le_bytes([data[0], data[1]]));
    let hdr_len = if type_code >= TYPE_STRUCT || type_code.0 >= 0x02A0 { 4 } else { 2 };
    let payload = match data.get(hdr_len..) {
        Some(p) => p,
        None => return cli::hex_spaced(data),
    };
    let out = match dt {
        TYPE_BOOL => payload.first().map(|b| (*b != 0).to_string()),
        TYPE_SINT => payload.first().map(|b| (*b as i8).to_string()),
        TYPE_INT => payload.get(..2).map(|b| i16::from_le_bytes([b[0], b[1]]).to_string()),
        TYPE_DINT => payload.get(..4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string()),
        TYPE_LINT => payload.get(..8).map(|b| i64::from_le_bytes(b.try_into().unwrap_or([0; 8])).to_string()),
        TYPE_REAL => payload.get(..4).map(|b| format!("{:.4}", f32::from_le_bytes([b[0], b[1], b[2], b[3]]))),
        TYPE_LREAL => {
            payload.get(..8).map(|b| format!("{:.6}", f64::from_le_bytes(b.try_into().unwrap_or([0; 8]))))
        }
        _ => None,
    };
    out.unwrap_or_else(|| cli::hex_spaced(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_values() {
        assert_eq!(cip_type_name(0x80C4), "DINT[]");
        assert_eq!(cip_type_name(0x02A0), "STRUCT");
        assert_eq!(cip_type_name(0x0001), "0x0001");
        assert_eq!(device_type_name(7), "General Purpose Discrete I/O");
        assert_eq!(device_type_name(99), "Type 99");
        let data = [0xC4, 0x00, 0x2A, 0, 0, 0];
        assert_eq!(format_tag_value(&data, TYPE_DINT), "42");
        assert_eq!(format_tag_value(&data[..4], TYPE_DINT), "C4 00 2A 00");
    }
}
