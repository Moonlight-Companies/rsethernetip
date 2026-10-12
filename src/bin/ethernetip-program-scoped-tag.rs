//! ethernetip-program-scoped-tag: reads a program-scoped Logix COUNTER, whole or by member.
//! Port of Go `examples/ethernetip/program_scoped_tag`.
//!
//! Logix tags are controller scoped ("MyTag") or program scoped
//! ("Program:MainProgram.MyTag"); structure members use dotted syntax
//! ("Program:MainProgram.MyCounter.ACC"). The controller needs these split on "." into multiple
//! symbolic segments (and Element segments for arrays); `cip::parse_tag_path` does that inside
//! the tag-level client APIs, so the natural tag string is passed through unchanged.
//!
//! Ref: Rockwell 1756-PM020 Logix 5000 Data Access - symbolic addressing with ANSI Extended
//! Symbol segments (CIP Vol. 1, Appendix C EPATH segment encodings, section unverified).
//!
//! Usage:
//!   ethernetip-program-scoped-tag -addr 192.168.1.10:44818 -tag Program:MainProgram.MyCounter
//!   ethernetip-program-scoped-tag -addr 192.168.1.10:44818 -tag Program:MainProgram.MyCounter -member ACC

use std::time::Duration;

use rsethernetip::cli::{self, Cli};
use rsethernetip::client;
use rsethernetip::plc::Context;

fn main() {
    let args = Cli::new("ethernetip-program-scoped-tag", "Reads a program-scoped COUNTER tag and optional member.")
        .flag("addr", "192.168.1.10:44818", "PLC address in host:port format")
        .flag("tag", "Program:MainProgram.MyCounter", "Counter tag (controller- or program-scoped)")
        .flag("member", "", "Optional member to read individually (e.g. ACC, PRE). Empty = whole struct only.")
        .parse_env();
    let addr = args.str("addr").to_string();
    let tag = args.str("tag").to_string();
    let member = args.str("member").to_string();

    let ctx = Context::background().with_timeout(Duration::from_secs(10));
    let client = match client::connect(&ctx, &addr, vec![]) {
        Ok(c) => c,
        Err(e) => cli::fatal(format!("connect {addr}: {e}")),
    };
    println!("Connected to {addr}\n");

    // 1. Whole COUNTER struct: Read Tag (0x4C) against the multi-segment EPATH built from `tag`,
    //    decoding the 14-byte payload into a Counter.
    println!("--- Whole struct: {tag} ---");
    let c = match client.read_counter(&ctx, &tag) {
        Ok(c) => c,
        Err(e) => cli::fatal(format!("ReadCounter: {e}")),
    };
    println!("PRE={}  ACC={}", c.pre, c.acc);
    println!("CU={}  CD={}  DN={}  OV={}  UN={}\n", c.cu, c.cd, c.dn, c.ov, c.un);

    // 2. Optional single member: the controller returns a plain DINT, so read an i32.
    if member.is_empty() {
        let _ = client.close();
        return;
    }
    let full = format!("{tag}.{member}");
    println!("--- Member read: {full} ---");
    match client::read_typed::<i32>(&client, &ctx, &full) {
        Ok(v) => println!("{member} = {v}"),
        Err(e) => cli::fatal(format!("Read[int32]({full}): {e}")),
    }
    let _ = client.close();
}
