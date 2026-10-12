# pccc_monitor — poll PCCC data-table addresses for changes

> Rust port of the Go example `examples/ethernetip/pccc_monitor`. Binary: `ethernetip-pccc-monitor` (`src/bin/ethernetip-pccc-monitor.rs`). Flags, defaults and output match the Go program; Go API names in the text below map to snake_case Rust items in the `rsethernetip` crate (for example `Client.ReadTag` is `Client::read_tag`, and `context.Context` is `plc::Context`). Run it with `cargo run --bin ethernetip-pccc-monitor -- <flags>`.

> **Rust port note.** The Go `monitor` package is outside this crate, so the Rust binary uses a small built-in poller with the same flags: every address is read each `-freq` through `pccc::Client::read_address` and a line is printed when the raw value changes or a read fails. Ctrl+C stops it.


Wraps `pccc.Client` in the protocol-agnostic `monitor` package. Subscribes
to one or more SLC addresses and prints every value change.

## Usage

```bash
# Poll a single integer at 500 ms:
cargo run --bin ethernetip-pccc-monitor -- -addr 10.30.40.71:44818 -tags N7:0

# Poll several addresses, all at 200 ms:
cargo run --bin ethernetip-pccc-monitor -- -addr 10.30.40.71:44818 \
  -tags N7:0,N7:1,F8:5,B3:0/2,T4:0.ACC \
  -freq 200ms
```

## Why this works

`pccc.Client` implements `plc.Reader`, so it slots into
`monitor.NewMonitor` exactly like `modbus.Client` or `ethernetip.Client`.
Subscribed `pccc.File` data points are read on the configured cadence; the
monitor compares each result to the previous one and emits an `Event` with
`Changed = true` whenever the value differs.
