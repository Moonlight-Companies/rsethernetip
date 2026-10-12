//! ethernetip-timer-counter: reads Logix TIMER and COUNTER structures. Port of Go
//! `examples/ethernetip/timer_counter`.
//!
//! TIMER: PRE/ACC are DINT milliseconds; EN/TT/DN are status bits. COUNTER: PRE/ACC are DINT;
//! CU/CD/DN/OV/UN are status bits.
//!
//! Ref: Rockwell 1756-PM020 Logix 5000 Data Access and the Logix 5000 Instruction Set reference
//! for TIMER/COUNTER structure members (section unverified).
//!
//! Usage:
//!   ethernetip-timer-counter -addr 192.168.1.10:44818 -timer-tag Timer_1
//!   ethernetip-timer-counter -addr 192.168.1.10:44818 -counter-tag Counter_1

use std::time::Duration;

use rsethernetip::cip::{Counter, Timer};
use rsethernetip::cli::{self, Cli};
use rsethernetip::client;
use rsethernetip::options::{with_retries, with_retry_delay};
use rsethernetip::plc::Context;

fn main() {
    let spec = Cli::new("ethernetip-timer-counter", "Reads Logix TIMER and COUNTER tags.")
        .flag("addr", "192.168.1.10:44818", "PLC address in host:port format")
        .flag("timer-tag", "", "Timer tag name (e.g. Timer_1)")
        .flag("counter-tag", "", "Counter tag name (e.g. Counter_1)");
    let args = spec.parse_env();
    let addr = args.str("addr").to_string();
    let timer_tag = args.str("timer-tag").to_string();
    let counter_tag = args.str("counter-tag").to_string();

    if timer_tag.is_empty() && counter_tag.is_empty() {
        println!("error: at least one of -timer-tag or -counter-tag is required");
        print!("{}", spec.usage());
        return;
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

    if !timer_tag.is_empty() {
        println!("=== Timer: {timer_tag} ===\n");
        // ReadTimer wraps ReadTag and decodes the structure.
        let timer = match client.read_timer(&ctx, &timer_tag) {
            Ok(t) => t,
            Err(e) => cli::fatal(format!("ReadTimer({timer_tag:?}) failed: {e}")),
        };
        print_timer(&timer);
        println!();
    }

    if !counter_tag.is_empty() {
        println!("=== Counter: {counter_tag} ===\n");
        // ReadTagInto is the generic path: any Unmarshaler works.
        let mut counter = Counter::default();
        if let Err(e) = client.read_tag_into(&ctx, &counter_tag, &mut counter) {
            cli::fatal(format!("ReadTagInto({counter_tag:?}) failed: {e}"));
        }
        print_counter(&counter);
        println!();
    }

    println!("Done.");
    let _ = client.close();
}

// Ref: Logix TIMER semantics - EN (enable), TT (timer timing), DN (done) (Logix 5000
// Instruction Set reference, section unverified).
fn print_timer(t: &Timer) {
    print!("  PRE (Preset):       {} ms", t.pre);
    if t.pre > 0 {
        print!("  ({:.1} seconds)", t.pre as f64 / 1000.0);
    }
    println!();

    print!("  ACC (Accumulated):  {} ms", t.acc);
    if t.acc > 0 {
        print!("  ({:.1} seconds)", t.acc as f64 / 1000.0);
    }
    println!();

    if t.pre > 0 {
        let pct = (t.acc as f64 / t.pre as f64 * 100.0).min(100.0);
        println!("  Progress:           {pct:.1}%");
    }

    println!();
    println!("  EN  (Enable):       {}", t.en);
    println!("  TT  (Timer Timing): {}", t.tt);
    println!("  DN  (Done):         {}", t.dn);

    print!("\n  State: ");
    if !t.en {
        println!("Idle (timer is not enabled)");
    } else if t.tt && !t.dn {
        println!("Timing (actively counting up)");
    } else if t.dn {
        println!("Done (preset reached)");
    } else {
        println!("Unknown");
    }
}

// Ref: Logix COUNTER semantics - CU/CD/DN/OV/UN (Logix 5000 Instruction Set reference,
// section unverified).
fn print_counter(c: &Counter) {
    println!("  PRE (Preset):       {}", c.pre);
    println!("  ACC (Accumulated):  {}", c.acc);
    if c.pre > 0 {
        let pct = c.acc as f64 / c.pre as f64 * 100.0;
        println!("  Progress:           {pct:.1}%");
    }

    println!();
    println!("  CU  (Count Up):     {}", c.cu);
    println!("  CD  (Count Down):   {}", c.cd);
    println!("  DN  (Done):         {}", c.dn);
    println!("  OV  (Overflow):     {}", c.ov);
    println!("  UN  (Underflow):    {}", c.un);

    print!("\n  State: ");
    if c.ov {
        println!("Overflow (ACC wrapped past max int32)");
    } else if c.un {
        println!("Underflow (ACC wrapped past min int32)");
    } else if c.dn {
        println!("Done (preset reached)");
    } else if c.cu || c.cd {
        println!("Counting");
    } else {
        println!("Idle");
    }
}
