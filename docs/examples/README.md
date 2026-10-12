# EtherNet/IP examples

Ports of the Go examples from `goindustrial/examples/ethernetip`. Each is a small binary in
`src/bin/` named `ethernetip-<name>`; run one with `cargo run --bin ethernetip-<name> -- <flags>`.
Flags are parsed by the dependency-free `rsethernetip::cli` module: `-flag value`,
`-flag=value` and `--flag value` all work, and `-h` prints usage.

| Binary | Doc | What it shows |
| --- | --- | --- |
| `ethernetip-read-tag` | [read_tag](read_tag.md) | Raw and typed Logix tag reads |
| `ethernetip-read-tag-typed` | [read_tag_typed](read_tag_typed.md) | `read_typed` / `read_slice` by CIP type, TIMER, COUNTER |
| `ethernetip-write-tag` | [write_tag](write_tag.md) | Writing a tag and reading it back |
| `ethernetip-program-scoped-tag` | [program_scoped_tag](program_scoped_tag.md) | `Program:X.Y.MEMBER` tag paths |
| `ethernetip-timer-counter` | [timer_counter](timer_counter.md) | TIMER and COUNTER structures |
| `ethernetip-list-tags` | [list_tags](list_tags.md) | Symbol object tag enumeration |
| `ethernetip-list-identity` | [list_identity](list_identity.md) | ListIdentity and ListServices |
| `ethernetip-probe` | [probe](probe.md) | Device survey: identity, network, assemblies, classes |
| `ethernetip-hexdump` | [hexdump](hexdump.md) | Wire-level hex dump tracing |
| `ethernetip-custom-type` | [custom_type](custom_type.md) | Registering a custom CIP struct type |
| `ethernetip-reconnecting` | [reconnecting](reconnecting.md) | Reconnecting transport, infinite retries, hooks |
| `ethernetip-server` | [server](server.md) | A tag-serving EtherNet/IP server |
| `ethernetip-adapter` | [adapter](adapter.md) | I/O adapter: assemblies, Forward_Open, UDP runtime |
| `ethernetip-io-scanner` | [io_scanner](io_scanner.md) | Class 1 scanner and cyclic input display |
| `ethernetip-pccc-read` | [pccc_read](pccc_read.md) | PCCC read of an SLC/MicroLogix address |
| `ethernetip-pccc-write` | [pccc_write](pccc_write.md) | PCCC write, including bit read-modify-write |
| `ethernetip-pccc-monitor` | [pccc_monitor](pccc_monitor.md) | Polling PCCC addresses for changes |

Try them locally without a PLC: start `ethernetip-server -addr 127.0.0.1:44818`, then run
`ethernetip-read-tag -addr 127.0.0.1:44818 -tag MyDINT`.

## Go to Rust cheat sheet

| Go | Rust |
| --- | --- |
| `ethernetip.Connect(ctx, addr, opts...)` | `client::connect(&ctx, addr, vec![opt.into(), ...])` |
| `WithRetries(2)`, `WithRetryDelay(d)` | `options::with_retries(2)`, `options::with_retry_delay(d)` |
| `context.WithTimeout(ctx, d)` | `Context::background().with_timeout(d)` |
| `ethernetip.Read[T]` / `ReadSlice[T]` | `client::read_typed::<T>` / `client::read_slice::<T>` |
| `client.ReadTagInto(ctx, tag, &v)` | `client.read_tag_into(&ctx, tag, &mut v)` |
| `log.Fatalf` | `cli::fatal` |
| `signal.Notify` + `cancel()` | `cli::on_interrupt(...)` / `cli::interrupted()` |
