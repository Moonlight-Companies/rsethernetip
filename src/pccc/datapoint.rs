//! PCCC data point. Port of Go `pccc/datapoint.go`.

use super::address::{Address, parse_address};
use super::{PcccError, Result};
use crate::plc::DataPoint;

// Ref: Allen-Bradley PCCC / DF1 Protocol manual 1770-6.5.16 (SLC logical
// addressing, e.g. N7:0, B3:0/2, T4:0.ACC); the data-point wrapper itself is a
// library abstraction, not spec.
/// A [`DataPoint`] naming a PCCC data-table location by its textual SLC
/// address (e.g. "N7:0", "B3:0/2", "T4:0.ACC").
///
/// `count` selects how many consecutive elements to read/write starting at
/// the addressed element. A value of 0 is treated as 1. Count > 1 is
/// meaningful only for word-sized files; bit suffixes and timer/counter
/// sub-fields are scalar and ignore it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct File {
    /// SLC address text.
    pub address: String,
    /// Element count (<= 0 means 1).
    pub count: i32,
}

impl File {
    // Implementation detail (not spec): constructor.
    /// Creates a single-element File data point.
    pub fn new(address: impl Into<String>) -> File {
        File { address: address.into(), count: 0 }
    }

    // Implementation detail (not spec): Go `String()` text.
    /// Canonical "File(address)" / "File(address, count=N)" form.
    pub fn string(&self) -> String {
        if self.count > 1 {
            return format!("File({}, count={})", self.address, self.count);
        }
        if self.address.is_empty() {
            return "File()".to_string();
        }
        format!("File({})", self.address)
    }

    // Implementation detail (not spec): count clamp.
    /// Count clamped to a minimum of 1.
    pub fn effective_count(&self) -> i32 {
        if self.count <= 0 { 1 } else { self.count }
    }

    // Implementation detail (not spec): wraps the parse error like Go.
    /// Parses `address`, wrapping an error with the File description.
    pub fn parse(&self) -> Result<Address> {
        parse_address(&self.address)
            .map_err(|e| PcccError::msg(format!("pccc: {}: {}", self.string(), e)))
    }
}

impl DataPoint for File {
    fn describe(&self) -> String {
        self.string()
    }

    // Implementation detail (not spec): lets the client recover a File from
    // an `Arc<dyn DataPoint>` (Go type assertion `dp.(File)`).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_implements_data_point() {
        let f = File::new("N7:0");
        let _: &dyn DataPoint = &f;
    }

    #[test]
    fn file_string() {
        let tests = [
            (File { address: "N7:0".into(), count: 0 }, "File(N7:0)"),
            (File { address: "N7:0".into(), count: 1 }, "File(N7:0)"),
            (File { address: "N7:0".into(), count: 5 }, "File(N7:0, count=5)"),
            (File { address: "T4:0.ACC".into(), count: 0 }, "File(T4:0.ACC)"),
            (File::default(), "File()"),
        ];
        for (f, want) in tests {
            assert_eq!(f.string(), want);
            assert_eq!(f.describe(), want);
        }
    }

    #[test]
    fn file_effective_count() {
        for (i, want) in [(0, 1), (-1, 1), (1, 1), (2, 2), (10, 10)] {
            let f = File { address: String::new(), count: i };
            assert_eq!(f.effective_count(), want, "effectiveCount({i})");
        }
    }
}
