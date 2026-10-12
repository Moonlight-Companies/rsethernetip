//! Test-support helpers (port of Go `internal/testutil`): a recording
//! [`MockLogger`] and free-port discovery. Implementation detail, not spec.

use crate::logging::{Fields, Level, Logger};
use std::fmt;
use std::net::{TcpListener};
use std::sync::{Arc, Mutex};

// Implementation detail (not spec): a recorded log entry.
/// A recorded log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogMessage {
    /// Severity.
    pub level: Level,
    /// Formatted message text.
    pub message: String,
}

// Implementation detail (not spec): recording logger for assertions.
/// A [`Logger`] that records every message at or above its level. Cloning
/// shares the recorded messages.
#[derive(Debug, Clone)]
pub struct MockLogger {
    inner: Arc<Mutex<(Level, Vec<LogMessage>)>>,
}

impl Default for MockLogger {
    fn default() -> Self {
        MockLogger::new()
    }
}

impl MockLogger {
    /// Captures everything (level Trace).
    pub fn new() -> MockLogger {
        MockLogger { inner: Arc::new(Mutex::new((Level::Trace, Vec::new()))) }
    }

    /// Snapshot of recorded messages.
    pub fn messages(&self) -> Vec<LogMessage> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).1.clone()
    }

    /// True if any recorded message contains `needle`.
    pub fn contains(&self, needle: &str) -> bool {
        self.messages().iter().any(|m| m.message.contains(needle))
    }

    fn record(&self, level: Level, args: fmt::Arguments<'_>) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if level < g.0 {
            return;
        }
        g.1.push(LogMessage { level, message: args.to_string() });
    }
}

impl Logger for MockLogger {
    fn trace(&self, a: fmt::Arguments<'_>) {
        self.record(Level::Trace, a)
    }
    fn debug(&self, a: fmt::Arguments<'_>) {
        self.record(Level::Debug, a)
    }
    fn info(&self, a: fmt::Arguments<'_>) {
        self.record(Level::Info, a)
    }
    fn warn(&self, a: fmt::Arguments<'_>) {
        self.record(Level::Warn, a)
    }
    fn error(&self, a: fmt::Arguments<'_>) {
        self.record(Level::Error, a)
    }
    // Fields are ignored in tests; same recorder is returned.
    fn with_fields(&self, _: &Fields) -> Arc<dyn Logger> {
        Arc::new(self.clone())
    }
    fn level(&self) -> Level {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).0
    }
    fn set_level(&self, level: Level) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).0 = level;
    }
}

/// Returns a free localhost TCP port (binds :0 then releases).
pub fn find_free_port() -> std::io::Result<u16> {
    let l = TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

/// Binds a TCP listener on a random free localhost port (no TOCTOU race).
pub fn listen_free() -> std::io::Result<TcpListener> {
    TcpListener::bind("127.0.0.1:0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_logger_records_and_filters() {
        let l = MockLogger::new();
        l.info(format_args!("a {}", 1));
        l.set_level(Level::Warn);
        l.debug(format_args!("dropped"));
        l.error(format_args!("e"));
        let m = l.messages();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].message, "a 1");
        assert!(l.contains("e"));
    }

    #[test]
    fn free_port() {
        assert!(find_free_port().unwrap() > 0);
        assert!(listen_free().is_ok());
    }
}
