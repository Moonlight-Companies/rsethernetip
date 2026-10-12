//! Registry of custom CIP struct types (type code -> codec factory).
//!
//! Port of Go `cip/registry.go`.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use super::decode::Unmarshaler;
use super::encode::Marshaler;
use super::types::*;

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - structured (UDT / predefined) tag types are
// identified on the wire by a structure type code and are encoded as raw structure bytes; ODVA
// CIP Vol. 1, Appendix C data type reporting for the code space (section unverified).
// Implemented by types that can be registered in the CIP type registry for automatic encoding,
// decoding and name resolution (Go `TypeCodec`). Combines [`Marshaler`] and [`Unmarshaler`]
// with a type code accessor. A type may also supply a display name via `display_name` (Go's
// optional `fmt.Stringer`), used by `DataType`'s Display instead of "UNKNOWN(0x...)".
//
// Registration is intended for start-up time only.
pub trait TypeCodec: Marshaler + Unmarshaler + Send {
    // Returns the CIP DataType code for this struct (without the array flag). Must match the
    // code used with [`register_type`].
    fn cip_type(&self) -> DataType;

    // Implementation detail: optional human-readable name (Go `fmt.Stringer`); None if the type
    // has no display name.
    fn display_name(&self) -> Option<String> {
        None
    }
}

// Implementation detail (not spec): a registered factory.
type Factory = Box<dyn Fn() -> Box<dyn TypeCodec> + Send + Sync>;

// Implementation detail (not spec): a registered type's factory and optional display name.
struct TypeEntry {
    factory: Factory,
    // from display_name() if provided, else ""
    name: String,
}

// Implementation detail (not spec): global registry keyed by base DataType code. Go populates
// it at init() time without synchronisation; an RwLock keeps the Rust version safe.
fn registry() -> &'static RwLock<HashMap<u16, TypeEntry>> {
    static REGISTRY: OnceLock<RwLock<HashMap<u16, TypeEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - custom struct type codes (section
// unverified).
// Registers a factory for a custom CIP struct type identified by `dt` (the base type code,
// without the array flag). The factory must return a new zero-value codec. If the returned
// codec supplies `display_name`, it is used by DataType's Display.
//
// Panics if `dt` has already been registered (same as Go's `RegisterType`; programmer error).
pub fn register_type<F>(dt: DataType, factory: F)
where
    F: Fn() -> Box<dyn TypeCodec> + Send + Sync + 'static,
{
    let base = dt.base();
    let mut reg = registry().write().unwrap_or_else(|e| e.into_inner());
    if reg.contains_key(&base.0) {
        panic!("cip: duplicate RegisterType for DataType 0x{:04X}", base.0);
    }
    // Probe the factory once to extract a display name.
    let probe = factory();
    let name = probe.display_name().unwrap_or_default();
    reg.insert(base.0, TypeEntry { factory: Box::new(factory), name });
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access (section unverified).
// Returns a new zero-value codec for the given DataType, or None if no type has been registered
// for that code. The array flag is masked off before lookup.
pub fn lookup_type(dt: DataType) -> Option<Box<dyn TypeCodec>> {
    let reg = registry().read().unwrap_or_else(|e| e.into_inner());
    reg.get(&dt.base().0).map(|e| (e.factory)())
}

// Implementation detail: returns the registered display name for `dt`, or "" if none.
pub(crate) fn lookup_type_name(dt: DataType) -> String {
    let reg = registry().read().unwrap_or_else(|e| e.into_inner());
    reg.get(&dt.base().0).map(|e| e.name.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A test codec for a made-up struct type (Go has no registry test; added for coverage).
    #[derive(Default)]
    struct Sample {
        v: u16,
    }

    impl Marshaler for Sample {
        fn marshal_cip(&self) -> Result<Vec<u8>> {
            Ok(self.v.to_le_bytes().to_vec())
        }
    }
    impl Unmarshaler for Sample {
        fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
            self.v.unmarshal_cip(data)
        }
    }
    impl TypeCodec for Sample {
        fn cip_type(&self) -> DataType {
            DataType(0x2F83)
        }
        fn display_name(&self) -> Option<String> {
            Some("Sample".to_string())
        }
    }

    #[test]
    fn register_and_lookup() {
        register_type(DataType(0x2F83), || Box::new(Sample::default()));
        let mut c = lookup_type(DataType(0x2F83 | 0x8000)).expect("registered");
        c.unmarshal_cip(&[0x34, 0x12]).unwrap();
        assert_eq!(c.marshal_cip().unwrap(), vec![0x34, 0x12]);
        assert_eq!(DataType(0x2F83).to_string(), "Sample");
        assert_eq!(DataType(0xAF83).to_string(), "Sample[]");
        assert!(lookup_type(DataType(0x2F84)).is_none());
        assert_eq!(crate::cip::encode::go_type_to_cip_type(&Sample::default()).unwrap(), DataType(0x2F83));
    }

    #[test]
    #[should_panic(expected = "duplicate RegisterType")]
    fn duplicate_registration_panics() {
        register_type(DataType(0x2F90), || Box::new(Sample::default()));
        register_type(DataType(0x2F90), || Box::new(Sample::default()));
    }
}
