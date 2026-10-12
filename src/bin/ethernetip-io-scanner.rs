//! ethernetip-io-scanner: an EtherNet/IP I/O scanner that opens a Class 1 implicit connection to
//! a PLC/adapter and displays the cyclic input assembly. Port of Go
//! `examples/ethernetip/io_scanner`.
//!
//! Steps: (1) register a TCP session; (2) read the Identity object; (3) send Forward_Open with
//! an assembly connection path (config instance + O->T and T->O connection points); (4) print
//! the T->O input data twice a second while cyclic UDP I/O runs at the RPI; (5) on exit send
//! Forward_Close.
//!
//! Ref: CIP Vol. 1, Chapter 3 Connection Management - Forward_Open (service 0x54) and
//! Forward_Close (0x4E), connection timeout multiplier (timeout = RPI * (4 << multiplier));
//! CIP Vol. 1 Assembly Object (class 0x04) connection points; CIP Vol. 2 implicit messaging over
//! UDP port 2222 (section unverified).
//!
//! Usage:
//!   ethernetip-io-scanner -addr 192.168.1.20 [-ot-instance 150 -to-instance 100 ...]

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rsethernetip::cip::{build_path, MessageRouterRequest, Path, CLASS_ASSEMBLY, CLASS_IDENTITY, SERVICE_GET_ATTRIBUTE_ALL};
use rsethernetip::cli::{self, Cli};
use rsethernetip::conn::TcpConn;
use rsethernetip::io_scanner::{IoConnectionConfig, IoScanner};
use rsethernetip::plc::Context;
use rsethernetip::session::Session;

fn main() {
    let spec = Cli::new("ethernetip-io-scanner", "Opens a Class 1 implicit I/O connection and shows input data.")
        .flag("addr", "", "PLC address (host or host:port, default port 44818)")
        .flag("ot-instance", "150", "O->T (output) assembly instance on the PLC")
        .flag("to-instance", "100", "T->O (input) assembly instance on the PLC")
        .flag("config-instance", "151", "Config assembly instance (0 to skip)")
        .flag("ot-size", "8", "O->T assembly size in bytes")
        .flag("to-size", "8", "T->O assembly size in bytes")
        .flag("rpi", "10ms", "Requested Packet Interval")
        .flag("timeout-mult", "3", "Timeout multiplier (timeout = RPI * 4 << mult)")
        .flag("cycles", "0", "Number of poll cycles to display (0 = run until Ctrl+C)")
        .flag("udp-port", "2222", "Target UDP port on PLC");
    let args = spec.parse_env();
    let addr = args.str("addr").to_string();
    if addr.is_empty() {
        eprintln!("Usage: ethernetip-io-scanner -addr <PLC_IP> [options]");
        eprintln!("  Example: ethernetip-io-scanner -addr 192.168.1.20");
        eprint!("{}", spec.usage());
        std::process::exit(1);
    }
    let ot_instance: u16 = args.get("ot-instance");
    let to_instance: u16 = args.get("to-instance");
    let cfg_instance: u16 = args.get("config-instance");
    let ot_size: u16 = args.get("ot-size");
    let to_size: u16 = args.get("to-size");
    let rpi = args.duration("rpi");
    let timeout_mult: u8 = args.get("timeout-mult");
    let cycles: u32 = args.get("cycles");
    let udp_port: u16 = args.get("udp-port");

    // Ctrl+C cancels the context; the display loop then runs the shutdown sequence.
    let ctx = Context::new_cancellable();
    {
        let c = ctx.clone();
        cli::on_interrupt(move || {
            println!("\nShutting down...");
            c.cancel();
        });
    }

    // --- Step 1: TCP session ---
    // Ref: CIP Vol. 2 Section 2-3: RegisterSession (0x0065).
    println!("Connecting to {addr}...");
    let tcp_addr = cli::with_default_port(&addr, 44818);
    let tc = match TcpConn::new(&tcp_addr, &[]) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("NewTCPConn: {e}");
            std::process::exit(1);
        }
    };
    let sess = Arc::new(Session::new(tc, None));
    if let Err(e) = sess.register(&ctx) {
        eprintln!("Register session: {e}");
        std::process::exit(1);
    }
    println!("TCP session registered.");

    println!();
    println!("Connection parameters:");
    println!("  O->T assembly:  instance {ot_instance}, {ot_size} bytes");
    println!("  T->O assembly:  instance {to_instance}, {to_size} bytes");
    if cfg_instance > 0 {
        println!("  Config assembly: instance {cfg_instance}");
    }
    println!("  RPI:           {}ms", rpi.as_millis());
    // Ref: CIP Vol. 1, Connection Timeout Multiplier: the connection times out after RPI * (4 << mult).
    let timeout = rpi.checked_mul(4u32 << (timeout_mult as u32 & 15)).unwrap_or(Duration::MAX);
    println!("  Timeout mult:  {timeout_mult} (timeout = {}ms)", timeout.as_millis());

    // --- Step 2: Identity read ---
    // Ref: CIP Vol. 1 Identity Object (class 0x01) Get_Attributes_All.
    println!();
    let read_req = MessageRouterRequest {
        service: SERVICE_GET_ATTRIBUTE_ALL,
        request_path: build_path(CLASS_IDENTITY, 1, 0),
        request_data: Vec::new(),
    };
    match sess.send_cip_request(&ctx, &read_req) {
        Err(e) => println!("Warning: Identity read failed: {e}"),
        Ok(resp) if resp.is_success() => println!("Identity object read OK ({} bytes)", resp.response_data.len()),
        Ok(resp) => println!("Warning: Identity read status 0x{:02X}", resp.general_status),
    }

    // --- Step 3: scanner + Forward_Open ---
    // Ref: implicit I/O uses UDP port 2222 on the target (CIP Vol. 2, section unverified).
    let target_udp: SocketAddr = match resolve_udp(&tcp_addr, udp_port) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Resolve UDP addr: {e}");
            std::process::exit(1);
        }
    };

    let scanner = match IoScanner::new(sess.clone(), ":0", &[]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("NewIOScanner: {e}");
            std::process::exit(1);
        }
    };

    println!("\nSending Forward_Open...");

    // Ref: CIP Vol. 1, Forward_Open Connection_Path: Class (Assembly 0x04), Instance (config
    // assembly), then one Connection Point per direction (O->T, T->O).
    let mut conn_path = Path::new();
    conn_path.add_class(CLASS_ASSEMBLY);
    conn_path.add_instance(if cfg_instance > 0 { cfg_instance } else { 1 });
    conn_path.add_connection_point(ot_instance);
    conn_path.add_connection_point(to_instance);

    let cfg = IoConnectionConfig {
        ot_connection_point: ot_instance,
        to_connection_point: to_instance,
        config_instance: cfg_instance,
        ot_size,
        to_size,
        rpi,
        timeout_multiplier: timeout_mult,
        target_addr: Some(target_udp),
        connection_path: Some(conn_path.bytes().to_vec()),
        ..Default::default()
    };
    println!("  Connection path: {}", cli::hex_spaced(conn_path.bytes()));

    let io_conn = match scanner.open(&ctx, &cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Forward_Open failed: {e}");
            eprintln!("\nCommon causes:");
            eprintln!("  - Wrong assembly instance numbers for this PLC");
            eprintln!("  - Wrong assembly sizes");
            eprintln!("  - PLC not configured for I/O connections");
            eprintln!("  - Another scanner already owns this connection");
            let _ = scanner.shutdown(&Context::background());
            std::process::exit(1);
        }
    };

    println!("Forward_Open succeeded!");
    println!("  OT Connection ID: 0x{:08X}", io_conn.ot_connection_id);
    println!("  TO Connection ID: 0x{:08X}", io_conn.to_connection_id);
    println!("  Actual RPI:       {}ms", io_conn.rpi.as_millis());
    println!("\nCyclic I/O running. Displaying input assembly:");
    println!("---");

    // --- Step 4: display loop, every 500 ms ---
    let mut cycle: u32 = 0;
    'run: loop {
        let mut waited = Duration::ZERO;
        while waited < Duration::from_millis(500) {
            if ctx.is_cancelled() {
                break 'run;
            }
            std::thread::sleep(Duration::from_millis(20));
            waited += Duration::from_millis(20);
        }
        cycle += 1;
        if cycles > 0 && cycle > cycles {
            break;
        }

        let input = io_conn.input();
        let age = io_conn.input_age();
        let timed_out = if io_conn.is_timed_out() { " [TIMED OUT]" } else { "" };
        println!("[{cycle:4}] T->O: {}  (age: {:>6}ms){timed_out}", cli::hex_spaced(&input), age.as_millis());
    }

    // --- Step 5: shutdown ---
    println!("\n--- Shutting down ---");
    println!("Sending Forward_Close...");
    let close_ctx = Context::background().with_timeout(Duration::from_secs(5));
    match scanner.close(&close_ctx, &io_conn) {
        Ok(()) => println!("Forward_Close succeeded."),
        Err(e) => println!("Forward_Close: {e}"),
    }
    let _ = scanner.shutdown(&close_ctx);
    let _ = sess.unregister(&Context::background());
    let _ = sess.close();
    println!("Done.");
}

// Implementation detail (not spec): host part of the TCP address combined with the UDP port.
fn resolve_udp(tcp_addr: &str, port: u16) -> Result<SocketAddr, String> {
    let host = match tcp_addr.rsplit_once(':') {
        Some((h, _)) => h.trim_start_matches('[').trim_end_matches(']'),
        None => tcp_addr,
    };
    (host, port)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or_else(|| format!("no address for {host}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_udp_target() {
        let a = resolve_udp("127.0.0.1:44818", 2222).unwrap();
        assert_eq!(a.port(), 2222);
        assert!(a.ip().is_loopback());
        assert!(resolve_udp("[::1]:44818", 2222).unwrap().ip().is_loopback());
    }
}
