# pccc_write — write a single PCCC value

> Rust port of the Go example `examples/ethernetip/pccc_write`. Binary: `ethernetip-pccc-write` (`src/bin/ethernetip-pccc-write.rs`). Flags, defaults and output match the Go program; Go API names in the text below map to snake_case Rust items in the `rsethernetip` crate (for example `Client.ReadTag` is `Client::read_tag`, and `context.Context` is `plc::Context`). Run it with `cargo run --bin ethernetip-pccc-write -- <flags>`.


Quick CLI for writing one SLC data-table value. The Go type used by the
write is inferred from the address: float files take `-value 3.14`, bit
suffixes (`B3:0/2`, `T4:0.EN`) take `-value 0` or `-value 1` and use a
read-modify-write cycle, everything else takes an integer.

## Usage

```bash
# Integer
cargo run --bin ethernetip-pccc-write -- -addr 10.30.40.71:44818 -tag N7:0 -value 42

# Float
cargo run --bin ethernetip-pccc-write -- -addr 10.30.40.71:44818 -tag F8:5 -value 3.14

# Bit (read-modify-write)
cargo run --bin ethernetip-pccc-write -- -addr 10.30.40.71:44818 -tag B3:0/2 -value 1
```
