//! Tiny hand-written command-line helpers shared by the `ethernetip-*` example binaries.
//!
//! Implementation detail, not spec: stands in for Go's `flag` package (single-dash and
//! double-dash long flags, `-name value` or `-name=value`, boolean switches, positionals) plus a
//! few helpers the Go examples used (`log.Fatalf`, `signal.Notify`, `% X` formatting,
//! Go-style duration parsing). Zero dependencies; std only.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// Implementation detail (not spec): one declared flag.
#[derive(Debug, Clone)]
struct Def {
    name: String,
    default: String,
    help: String,
    is_switch: bool,
}

// Implementation detail (not spec): flag-set builder, the analogue of Go's `flag.FlagSet`.
/// Declares flags and parses a command line.
#[derive(Debug, Clone)]
pub struct Cli {
    prog: String,
    about: String,
    positional_usage: Option<String>,
    defs: Vec<Def>,
}

// Implementation detail (not spec): result of parsing.
/// Parsed flag values and positional arguments.
#[derive(Debug, Clone)]
pub struct Parsed {
    prog: String,
    values: HashMap<String, String>,
    positional: Vec<String>,
}

// Implementation detail (not spec): parse failure or help request.
/// Command-line parse outcome other than success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// `-h` / `--help` was given; the payload is the usage text.
    Help(String),
    /// A bad or unknown flag; payload is the message.
    Invalid(String),
}

impl Cli {
    /// Starts a flag set for program `prog` with a one-line description.
    pub fn new(prog: &str, about: &str) -> Cli {
        Cli { prog: prog.to_string(), about: about.to_string(), positional_usage: None, defs: Vec::new() }
    }

    /// Declares a value flag with a default (like `flag.String`/`Int`/`Duration`).
    pub fn flag(mut self, name: &str, default: &str, help: &str) -> Cli {
        self.defs.push(Def { name: name.into(), default: default.into(), help: help.into(), is_switch: false });
        self
    }

    /// Declares a boolean switch, false unless present (like `flag.Bool`).
    pub fn switch(mut self, name: &str, help: &str) -> Cli {
        self.defs.push(Def { name: name.into(), default: "false".into(), help: help.into(), is_switch: true });
        self
    }

    /// Names the positional arguments in the usage line, e.g. `"<PLC_IP>"`.
    pub fn positional(mut self, usage: &str) -> Cli {
        self.positional_usage = Some(usage.to_string());
        self
    }

    /// Renders usage text (like `flag.PrintDefaults`).
    pub fn usage(&self) -> String {
        let mut s = String::new();
        let _ = write!(s, "Usage: {} [flags]", self.prog);
        if let Some(p) = &self.positional_usage {
            let _ = write!(s, " {p}");
        }
        s.push('\n');
        if !self.about.is_empty() {
            let _ = writeln!(s, "{}", self.about);
        }
        s.push_str("\nFlags:\n");
        for d in &self.defs {
            if d.is_switch {
                let _ = writeln!(s, "  -{}\n        {}", d.name, d.help);
            } else {
                let _ = writeln!(s, "  -{} <value>\n        {} (default {:?})", d.name, d.help, d.default);
            }
        }
        s
    }

    /// Parses an explicit argument list (without the program name).
    pub fn parse<I: IntoIterator<Item = String>>(&self, args: I) -> Result<Parsed, CliError> {
        let mut values: HashMap<String, String> =
            self.defs.iter().map(|d| (d.name.clone(), d.default.clone())).collect();
        let mut positional = Vec::new();
        let mut it = args.into_iter();
        let mut only_positional = false;
        while let Some(a) = it.next() {
            if only_positional || a == "-" || !a.starts_with('-') {
                positional.push(a);
                continue;
            }
            if a == "--" {
                only_positional = true;
                continue;
            }
            let body = a.trim_start_matches('-');
            if body == "h" || body == "help" {
                return Err(CliError::Help(self.usage()));
            }
            let (name, inline) = match body.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (body, None),
            };
            let def = self
                .defs
                .iter()
                .find(|d| d.name == name)
                .ok_or_else(|| CliError::Invalid(format!("flag provided but not defined: -{name}")))?;
            let value = if def.is_switch {
                inline.unwrap_or_else(|| "true".to_string())
            } else {
                match inline {
                    Some(v) => v,
                    None => it
                        .next()
                        .ok_or_else(|| CliError::Invalid(format!("flag needs an argument: -{name}")))?,
                }
            };
            values.insert(def.name.clone(), value);
        }
        Ok(Parsed { prog: self.prog.clone(), values, positional })
    }

    /// Parses `std::env::args()`; prints usage and exits (status 0 for help, 2 for errors).
    pub fn parse_env(&self) -> Parsed {
        match self.parse(std::env::args().skip(1)) {
            Ok(p) => p,
            Err(CliError::Help(u)) => {
                print!("{u}");
                std::process::exit(0);
            }
            Err(CliError::Invalid(m)) => {
                eprintln!("{m}");
                eprint!("{}", self.usage());
                std::process::exit(2);
            }
        }
    }
}

impl Parsed {
    /// The string value of flag `name` (its default if absent). Undeclared names give "".
    pub fn str(&self, name: &str) -> &str {
        self.values.get(name).map(String::as_str).unwrap_or("")
    }

    /// The value of a switch.
    pub fn on(&self, name: &str) -> bool {
        matches!(self.str(name), "true" | "1" | "t" | "T" | "TRUE" | "True")
    }

    /// Parses flag `name` into `T`, or reports a Go-flag-style error.
    pub fn try_get<T: FromStr>(&self, name: &str) -> Result<T, String> {
        let v = self.str(name);
        v.parse::<T>().map_err(|_| format!("invalid value {v:?} for flag -{name}"))
    }

    /// Parses flag `name` into `T`; prints the error and exits with status 2 on failure.
    pub fn get<T: FromStr>(&self, name: &str) -> T {
        match self.try_get(name) {
            Ok(v) => v,
            Err(m) => {
                eprintln!("{}: {m}", self.prog);
                std::process::exit(2);
            }
        }
    }

    /// Parses flag `name` as a Go-style duration (`500ms`, `2s`, `1m30s`); exits on failure.
    pub fn duration(&self, name: &str) -> Duration {
        match parse_duration(self.str(name)) {
            Ok(d) => d,
            Err(m) => {
                eprintln!("{}: invalid value {:?} for flag -{name}: {m}", self.prog, self.str(name));
                std::process::exit(2);
            }
        }
    }

    /// Positional arguments in order.
    pub fn positional(&self) -> &[String] {
        &self.positional
    }
}

// Implementation detail (not spec): Go `time.ParseDuration` subset (ns, us, ms, s, m, h; fractional ok).
/// Parses a Go-style duration string such as `500ms`, `2s`, `1.5s` or `1m30s`.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    if s == "0" {
        return Ok(Duration::ZERO);
    }
    if s.is_empty() {
        return Err("empty duration".into());
    }
    let mut rest = s;
    let mut total = 0f64;
    while !rest.is_empty() {
        let num_end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
        if num_end == 0 {
            return Err(format!("invalid duration {s:?}"));
        }
        let n: f64 = rest[..num_end].parse().map_err(|_| format!("invalid duration {s:?}"))?;
        rest = &rest[num_end..];
        let unit_end = rest.find(|c: char| c.is_ascii_digit() || c == '.').unwrap_or(rest.len());
        let unit = &rest[..unit_end];
        rest = &rest[unit_end..];
        let secs = match unit {
            "ns" => 1e-9,
            "us" | "\u{b5}s" => 1e-6,
            "ms" => 1e-3,
            "s" => 1.0,
            "m" => 60.0,
            "h" => 3600.0,
            "" => return Err(format!("missing unit in duration {s:?}")),
            _ => return Err(format!("unknown unit {unit:?} in duration {s:?}")),
        };
        total += n * secs;
    }
    if !total.is_finite() || total < 0.0 {
        return Err(format!("invalid duration {s:?}"));
    }
    Ok(Duration::from_secs_f64(total))
}

// Implementation detail (not spec): Go `log.Fatalf` - message to stderr, exit status 1.
/// Prints `msg` to stderr and exits with status 1.
pub fn fatal(msg: impl AsRef<str>) -> ! {
    eprintln!("{}", msg.as_ref());
    std::process::exit(1)
}

// Implementation detail (not spec): Go `fmt` verb `% X` - uppercase hex, space separated.
/// Formats bytes like Go's `% X` (e.g. `DE AD BE EF`).
pub fn hex_spaced(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 3);
    for (i, b) in data.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{b:02X}");
    }
    s
}

// Implementation detail (not spec): Go `fmt` verb `%X` on a byte slice - uppercase hex, no separator.
/// Formats bytes like Go's `%X` (e.g. `DEADBEEF`).
pub fn hex_plain(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        let _ = write!(s, "{b:02X}");
    }
    s
}

// Implementation detail (not spec): Go `%v` on a slice - `[a b c]`.
/// Formats a slice like Go's `%v` (space separated inside brackets).
pub fn fmt_list<T: std::fmt::Display>(items: &[T]) -> String {
    let mut s = String::from("[");
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{it}");
    }
    s.push(']');
    s
}

// Implementation detail (not spec): `net.SplitHostPort` check used by examples that accept a bare host.
/// Appends `:port` to `addr` unless it already carries a port (IPv6 literals need brackets).
pub fn with_default_port(addr: &str, port: u16) -> String {
    let has_port = if let Some(rest) = addr.strip_prefix('[') {
        rest.contains("]:")
    } else {
        addr.matches(':').count() == 1
    };
    if has_port {
        addr.to_string()
    } else if addr.contains(':') && !addr.starts_with('[') {
        format!("[{addr}]:{port}")
    } else {
        format!("{addr}:{port}")
    }
}

// Implementation detail (not spec): "HH:MM:SS" in UTC, used for log-style output (std has no local time).
/// Current wall-clock time of day as `HH:MM:SS` (UTC).
pub fn clock_hms() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let t = secs % 86_400;
    format!("{:02}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
}

// ---------------- interrupt handling ----------------

// Implementation detail (not spec): set by the SIGINT/SIGTERM handler, polled by the examples.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
mod sys {
    use super::INTERRUPTED;
    use std::sync::atomic::Ordering;

    // Implementation detail (not spec): minimal libc binding. `signal` is already linked by std
    // on Unix, so this adds no crate dependency. The handler only stores to an atomic, which is
    // async-signal-safe.
    unsafe extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }

    extern "C" fn on_signal(_sig: i32) {
        INTERRUPTED.store(true, Ordering::SeqCst);
    }

    pub fn install() {
        const SIGINT: i32 = 2;
        const SIGTERM: i32 = 15;
        // SAFETY: registering an async-signal-safe handler with the C library.
        unsafe {
            signal(SIGINT, on_signal as extern "C" fn(i32) as usize);
            signal(SIGTERM, on_signal as extern "C" fn(i32) as usize);
        }
    }
}

#[cfg(not(unix))]
mod sys {
    pub fn install() {}
}

/// Installs SIGINT/SIGTERM handlers (Unix) so [`interrupted`] turns true on Ctrl+C
/// (Go `signal.Notify`). A no-op on other platforms.
pub fn install_interrupt_handler() {
    sys::install();
}

/// Installs the interrupt handler and spawns a watcher thread that runs `f` once on the first
/// Ctrl+C / SIGTERM (the Go `signal.Notify` + goroutine + `cancel()` idiom).
pub fn on_interrupt(f: impl FnOnce() + Send + 'static) {
    install_interrupt_handler();
    std::thread::spawn(move || {
        while !interrupted() {
            std::thread::sleep(Duration::from_millis(20));
        }
        f();
    });
}

/// True once Ctrl+C / SIGTERM has been received.
pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

/// Sleeps up to `d` in small slices, returning true early if interrupted.
pub fn sleep_interruptible(d: Duration) -> bool {
    let end = std::time::Instant::now() + d;
    loop {
        if interrupted() {
            return true;
        }
        let now = std::time::Instant::now();
        if now >= end {
            return false;
        }
        std::thread::sleep((end - now).min(Duration::from_millis(20)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli() -> Cli {
        Cli::new("t", "test").flag("addr", "a:1", "address").flag("count", "1", "n").switch("v", "verbose")
    }

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults_and_overrides() {
        let p = cli().parse(args(&[])).unwrap();
        assert_eq!(p.str("addr"), "a:1");
        assert!(!p.on("v"));
        let p = cli().parse(args(&["-addr", "x:2", "--count=7", "-v", "pos"])).unwrap();
        assert_eq!(p.str("addr"), "x:2");
        assert_eq!(p.get::<u32>("count"), 7);
        assert!(p.on("v"));
        assert_eq!(p.positional(), &["pos".to_string()]);
    }

    #[test]
    fn errors() {
        assert!(matches!(cli().parse(args(&["-nope"])), Err(CliError::Invalid(_))));
        assert!(matches!(cli().parse(args(&["-addr"])), Err(CliError::Invalid(_))));
        assert!(matches!(cli().parse(args(&["-h"])), Err(CliError::Help(_))));
        let p = cli().parse(args(&["-count", "zz"])).unwrap();
        assert!(p.try_get::<u32>("count").is_err());
    }

    #[test]
    fn double_dash_stops_flags() {
        let p = cli().parse(args(&["--", "-v"])).unwrap();
        assert!(!p.on("v"));
        assert_eq!(p.positional(), &["-v".to_string()]);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_duration("1m30s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("1.5s").unwrap(), Duration::from_millis(1500));
        assert!(parse_duration("5").is_err());
        assert!(parse_duration("").is_err());
        assert!(parse_duration("3x").is_err());
    }

    #[test]
    fn hex_and_ports() {
        assert_eq!(hex_spaced(&[0xDE, 0xAD, 1]), "DE AD 01");
        assert_eq!(hex_plain(&[0xDE, 1]), "DE01");
        assert_eq!(hex_spaced(&[]), "");
        assert_eq!(fmt_list(&[1, 2, 3]), "[1 2 3]");
        assert_eq!(fmt_list::<u8>(&[]), "[]");
        assert_eq!(with_default_port("1.2.3.4", 44818), "1.2.3.4:44818");
        assert_eq!(with_default_port("1.2.3.4:5", 44818), "1.2.3.4:5");
        assert_eq!(with_default_port("::1", 44818), "[::1]:44818");
        assert_eq!(with_default_port("[::1]:9", 44818), "[::1]:9");
    }
}
