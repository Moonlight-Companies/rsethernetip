//! ethernetip-adapter: an EtherNet/IP adapter (I/O target) with implicit messaging. Port of Go
//! `examples/ethernetip/adapter`.
//!
//! It combines the pieces of the crate:
//! - `AssemblyObject` holds instance 100 (O->T, consumed from the scanner) and 101 (T->O,
//!   produced to the scanner).
//! - `Runtime` listens on UDP for Class 1 cyclic I/O; `Scheduler` produces at each RPI.
//! - `ConnectionManager` handles Forward_Open / Forward_Close and registers/removes the two
//!   I/O connections.
//! - `Server` serves the explicit-messaging TCP side.
//!
//! The main loop mirrors consumed data into the produced assembly: the first two produced bytes
//! carry a cycle counter, the rest echoes the consumed bytes.
//!
//! Ref: CIP Vol. 1, Chapter 3 Connection Management (Forward_Open 0x54 / Forward_Close 0x4E)
//! and Section 3-5 Connection Manager Object; CIP Vol. 2, Chapter 3 implicit I/O messaging over
//! UDP port 2222 (section unverified); CIP Vol. 1 Assembly Object (class 0x04, attribute 3 =
//! Data).
//!
//! Usage:
//!   ethernetip-adapter -tcp :44818 -udp :2222 -ot-size 12 -to-size 12

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rsethernetip::cip::{MessageRouter, CLASS_ASSEMBLY, CLASS_CONNECTION_MGR};
use rsethernetip::cli::{self, Cli};
use rsethernetip::eip::{ListIdentityItem, ITEM_ID_LIST_IDENTITY};
use rsethernetip::logging::{DefaultLogger, Level, Logger};
use rsethernetip::objects::assembly::AssemblyObject;
use rsethernetip::objects::connmgr::ConnectionManager;
use rsethernetip::plc::Context;
use rsethernetip::runtime::{IOConnection, Runtime, Scheduler, StopSignal};
use rsethernetip::server::Server;

fn main() {
    let args = Cli::new("ethernetip-adapter", "EtherNet/IP adapter with implicit I/O.")
        .flag("tcp", ":44818", "TCP listen address for EIP sessions")
        .flag("udp", ":2222", "UDP listen address for implicit I/O")
        .flag("ot-size", "12", "O->T (consume) assembly size in bytes")
        .flag("to-size", "12", "T->O (produce) assembly size in bytes")
        .parse_env();
    let tcp_addr = args.str("tcp").to_string();
    let udp_addr = args.str("udp").to_string();
    let ot_size: usize = args.get("ot-size");
    let to_size: usize = args.get("to-size");

    let logger: Arc<dyn Logger> = Arc::new(DefaultLogger::builder().level(Level::Info).build());

    // Assembly instances: 100 consumes (O->T), 101 produces (T->O).
    let ao = Arc::new(AssemblyObject::new());
    ao.register_assembly(100, vec![0; ot_size]);
    ao.register_assembly(101, vec![0; to_size]);
    println!("Assembly instances:");
    println!("  Instance 100 (consume): {ot_size} bytes");
    println!("  Instance 101 (produce): {to_size} bytes");

    // UDP runtime for implicit I/O.
    let rt = Runtime::new(ao.clone());
    if let Err(e) = rt.start(&udp_addr) {
        eprintln!("UDP runtime start: {e}");
        std::process::exit(1);
    }
    match rt.addr() {
        Some(a) => println!("UDP runtime listening on {a}"),
        None => println!("UDP runtime listening on {udp_addr}"),
    }

    let sched = Scheduler::new(&rt);
    sched.start();

    // Connection manager: Forward_Open registers the I/O connections, Forward_Close removes them.
    let active_conns = Arc::new(AtomicI32::new(0));
    let (rt_open, ao_open, ac_open) = (rt.clone(), ao.clone(), active_conns.clone());
    let (rt_close, ac_close) = (rt.clone(), active_conns.clone());
    let cm = ConnectionManager::new()
        .with_on_open(move |c, req| {
            // RPI is in microseconds on the wire; clamp tiny values to 10 ms.
            // Ref: CIP Vol. 1, Forward_Open O->T / T->O RPI fields in microseconds.
            let mut rpi = Duration::from_micros(req.ot_rpi as u64);
            if rpi < Duration::from_millis(1) {
                rpi = Duration::from_millis(10);
            }

            println!("\n[OPEN] Connection from scanner");
            println!("  OT Connection ID: 0x{:08X}", c.ot_connection_id);
            println!("  TO Connection ID: 0x{:08X}", c.to_connection_id);
            println!("  RPI:              {}ms", rpi.as_millis());
            println!("  Serial:           0x{:04X}", c.connection_serial_number);

            // Consumer: the scanner sends O->T data to the OT connection ID.
            rt_open.add_connection(IOConnection {
                connection_id: c.ot_connection_id,
                rpi,
                assembly: ao_open.get_instance(100),
                is_consumer: true,
                timeout_mult: req.connection_timeout_multiplier,
                run_idle_header: false,
                stop_chan: Some(StopSignal::new()),
                ..Default::default()
            });
            // Producer: we send T->O data on the TO connection ID.
            rt_open.add_connection(IOConnection {
                connection_id: c.to_connection_id,
                rpi,
                assembly: ao_open.get_instance(101),
                is_producer: true,
                run_idle_header: false,
                stop_chan: Some(StopSignal::new()),
                ..Default::default()
            });
            ac_open.fetch_add(1, Ordering::SeqCst);
        })
        .with_on_close(move |c| {
            println!("\n[CLOSE] Connection 0x{:08X} / 0x{:08X}", c.ot_connection_id, c.to_connection_id);
            rt_close.remove_connection(c.ot_connection_id);
            rt_close.remove_connection(c.to_connection_id);
            ac_close.fetch_sub(1, Ordering::SeqCst);
        });

    // Message router: Connection Manager (0x06) and Assembly (0x04) objects.
    let router = Arc::new(MessageRouter::new());
    router.register_object(CLASS_CONNECTION_MGR, Arc::new(cm));
    router.register_object(CLASS_ASSEMBLY, ao.clone());

    // TCP server with a ListIdentity answer. Ref: CIP Vol. 1 Device Profiles: device type 7 =
    // General Purpose Discrete I/O (section unverified).
    let srv = Server::builder(router)
        .logger(logger)
        .identity(ListIdentityItem {
            type_id: ITEM_ID_LIST_IDENTITY,
            encaps_version: 1,
            vendor_id: 1,
            device_type: 7,
            product_code: 1,
            revision: [1, 0],
            product_name: "GoIndustrial Adapter".to_string(),
            ..Default::default()
        })
        .build();
    if let Err(e) = srv.start(&Context::background(), &tcp_addr) {
        eprintln!("TCP server start: {e}");
        std::process::exit(1);
    }

    println!("EIP server listening on {tcp_addr}");
    println!();
    println!("Waiting for scanner connections (Forward_Open)...");
    println!("Press Ctrl+C to stop.");
    println!();

    cli::install_interrupt_handler();

    // Main loop (implementation detail, not spec): every 100 ms mirror consumed -> produced;
    // every second print a status line.
    let mut cycle: u16 = 0;
    let mut next_tick = Instant::now() + Duration::from_millis(100);
    let mut next_display = Instant::now() + Duration::from_secs(1);
    while !cli::interrupted() {
        std::thread::sleep(Duration::from_millis(10));
        let now = Instant::now();

        if now >= next_tick {
            next_tick += Duration::from_millis(100);
            if active_conns.load(Ordering::SeqCst) != 0 {
                cycle = cycle.wrapping_add(1);
                if let Ok(consumed) = ao.get_attribute_single(100, 3) {
                    let mut produce = vec![0u8; to_size];
                    if produce.len() >= 2 {
                        produce[0..2].copy_from_slice(&cycle.to_le_bytes());
                    }
                    let copy_len = consumed.len().min(produce.len().saturating_sub(2));
                    if copy_len > 0 {
                        produce[2..2 + copy_len].copy_from_slice(&consumed[..copy_len]);
                    }
                    let _ = ao.set_attribute_single(101, 3, &produce);
                }
            }
        }

        if now >= next_display {
            next_display += Duration::from_secs(1);
            if active_conns.load(Ordering::SeqCst) != 0 {
                let consumed = ao.get_attribute_single(100, 3).unwrap_or_default();
                let produced = ao.get_attribute_single(101, 3).unwrap_or_default();
                println!("[cycle {cycle:5}] consume={:<40} produce={}", preview(&consumed), preview(&produced));
            }
        }
    }

    println!("\nShutting down...");
    println!("\nActive connections: {}", active_conns.load(Ordering::SeqCst));
    println!("Done.");
    let _ = srv.stop();
    sched.stop();
    rt.stop();
}

// Implementation detail (not spec): first 16 bytes in hex, with an ellipsis if truncated.
fn preview(d: &[u8]) -> String {
    if d.len() > 16 {
        format!("{}...", cli::hex_spaced(&d[..16]))
    } else {
        cli::hex_spaced(d)
    }
}
