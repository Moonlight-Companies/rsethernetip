# rsethernetip

A zero-dependency Rust port of the EtherNet/IP and CIP protocol stack from
`goindustrial/protocol/ethernetip`: encapsulation, CIP message routing and tag access, PCCC for
SLC/MicroLogix, a client with retries and reconnection, a server/adapter, implicit I/O (scanner
and UDP runtime), and Rockwell Logix types. Standard library only: threads and `std::net`, no
async runtime.

Modules: `eip`, `cip`, `pccc`, `objects`, `vendor`, `runtime`, `plc`, `logging`, `client`,
`server`, `io_scanner`, `cli` (argument helpers for the examples).

Seventeen example binaries live in `src/bin/` (`ethernetip-*`); see
[docs/examples/README.md](docs/examples/README.md).

```sh
cargo test
cargo run --bin ethernetip-server -- -addr 127.0.0.1:44818
cargo run --bin ethernetip-read-tag -- -addr 127.0.0.1:44818 -tag MyDINT
```
