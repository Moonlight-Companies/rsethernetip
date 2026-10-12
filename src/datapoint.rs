//! The EtherNet/IP [`Tag`] data point.
//!
//! Port of Go `datapoint.go`.

use crate::plc::DataPoint;

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - tags are named data points addressed by
// symbolic segment (ANSI Extended Symbol, CIP Vol. 1 Appendix C segment 0x91, section unverified);
// the element count maps to the Number of Elements field of Read Tag (0x4C) / Write Tag (0x4D).
/// Identifies a readable/writable tag on an EtherNet/IP controller, e.g.
/// `"Motor_Speed"`. Implements [`DataPoint`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tag {
    /// Symbolic tag name as configured in the PLC program (e.g. `"Line1_Count"`).
    pub name: String,

    /// Number of array elements to read or write. For scalar tags use 1; a zero value is
    /// treated as 1.
    pub elements: u16,
}

impl Tag {
    /// Creates a scalar tag (elements = 1).
    pub fn new(name: impl Into<String>) -> Tag {
        Tag { name: name.into(), elements: 1 }
    }

    /// Creates a tag with an explicit element count.
    pub fn with_elements(name: impl Into<String>, elements: u16) -> Tag {
        Tag { name: name.into(), elements }
    }
}

// Implementation detail (not spec): Go `Tag.String()` returns the tag name.
impl DataPoint for Tag {
    fn describe(&self) -> String {
        self.name.clone()
    }

    // Implementation detail: lets `Client` recover the concrete `Tag` (Go `dp.(Tag)`).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_describe_is_name() {
        let t = Tag::new("Motor_Speed");
        assert_eq!(t.describe(), "Motor_Speed");
        assert_eq!(t.elements, 1);
        let dp: &dyn DataPoint = &Tag { name: "A".into(), elements: 0 };
        assert_eq!(dp.as_any().unwrap().downcast_ref::<Tag>().unwrap().name, "A");
    }
}
