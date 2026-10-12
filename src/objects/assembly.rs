//! CIP Assembly Object (Class 0x04): I/O data buffers for implicit messaging.
//!
//! Port of Go `objects/assembly/assembly.go`.
//!
//! In CIP an Assembly Object groups attributes from other objects into one contiguous block
//! of data. A typical device exposes an Input assembly (produced data), an Output assembly
//! (consumed data) and a Configuration assembly (static parameters sent at connection time).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use crate::cip::{
    CipError, Error, Object, Path, Result, STATUS_ATTRIBUTE_NOT_SUPPORTED,
    STATUS_INVALID_ATTRIBUTE_VALUE, STATUS_OBJECT_DOES_NOT_EXIST, STATUS_PATH_SEGMENT_ERROR,
    STATUS_SERVICE_NOT_SUPPORTED, SERVICE_GET_ATTRIBUTE_SINGLE, SERVICE_SET_ATTRIBUTE_SINGLE,
    USINT,
};

// Ref: ODVA CIP Vol. 1, Chapter 5 Object Library, Assembly Object class code 0x04 (section
// unverified).
// Attribute 3 (Data) of an Assembly instance: the assembly's data block.
const ATTR_DATA: u16 = 3;

// Ref: ODVA CIP Vol. 1, Appendix C Segment Encoding - Logical Segment, Instance ID 8-bit
// format (0x24) (section unverified).
const SEG_INSTANCE_8: u8 = 0x24;
// Ref: ODVA CIP Vol. 1, Appendix C Logical Segment, Instance ID 16-bit format (0x25) (section
// unverified).
const SEG_INSTANCE_16: u8 = 0x25;
// Ref: ODVA CIP Vol. 1, Appendix C Logical Segment, Attribute ID 8-bit format (0x30) (section
// unverified).
const SEG_ATTRIBUTE_8: u8 = 0x30;
// Ref: ODVA CIP Vol. 1, Appendix C Logical Segment, Attribute ID 16-bit format (0x31) (section
// unverified).
const SEG_ATTRIBUTE_16: u8 = 0x31;

// Ref: ODVA CIP Vol. 1, Assembly Object (class 0x04) instance (section unverified).
// A single assembly instance such as an Input, Output or Configuration assembly. `id` is the
// CIP instance number; the data buffer's size is fixed at registration time.
//
// Implementation detail (not spec): the buffer sits behind a Mutex so instances handed out by
// `get_instance` can be shared across threads (Go returns a raw pointer).
#[derive(Debug)]
pub struct AssemblyInstance {
    pub id: u32,
    data: Mutex<Vec<u8>>,
}

impl AssemblyInstance {
    // Implementation detail: returns a copy of the current buffer.
    pub fn data(&self) -> Vec<u8> {
        self.data.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    // Implementation detail: current buffer length in bytes.
    pub fn len(&self) -> usize {
        self.data.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    // Implementation detail: true when the buffer is zero-length.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// Ref: ODVA CIP Vol. 1, Chapter 5 Object Library, Assembly Object (class 0x04) (section
// unverified).
// Holds a set of assembly instances, each backed by a contiguous byte buffer, and supports
// Get_Attribute_Single and Set_Attribute_Single as needed for implicit I/O messaging. All
// methods are safe for concurrent use.
#[derive(Debug, Default)]
pub struct AssemblyObject {
    instances: RwLock<HashMap<u32, Arc<AssemblyInstance>>>,
}

// Implementation detail: builds a CIP status error.
fn status(code: USINT) -> CipError {
    CipError::Status(Error::new(code))
}

impl AssemblyObject {
    // Creates a new, empty AssemblyObject with no registered instances.
    pub fn new() -> AssemblyObject {
        AssemblyObject { instances: RwLock::new(HashMap::new()) }
    }

    // Returns the instance for the given ID, or None if not found.
    pub fn get_instance(&self, instance_id: u32) -> Option<Arc<AssemblyInstance>> {
        let g = self.instances.read().unwrap_or_else(|e| e.into_inner());
        g.get(&instance_id).cloned()
    }

    // Registers a new assembly instance with the given ID and initial buffer. The length of
    // `data` defines the fixed size of the instance; later Set_Attribute_Single calls must
    // supply exactly this many bytes. Replaces an existing instance with the same ID.
    pub fn register_assembly(&self, instance_id: u32, data: Vec<u8>) {
        let mut g = self.instances.write().unwrap_or_else(|e| e.into_inner());
        g.insert(instance_id, Arc::new(AssemblyInstance { id: instance_id, data: Mutex::new(data) }));
    }

    // Ref: ODVA CIP Vol. 1, Common Services - Get_Attribute_Single (0x0E); Assembly Object
    // attribute 3 "Data" (section unverified).
    // Attribute 3 (Data) returns a copy of the instance buffer. All other attribute IDs return
    // StatusAttributeNotSupported; an unknown instance returns StatusObjectDoesNotExist.
    pub fn get_attribute_single(&self, instance_id: u32, attr_id: u16) -> Result<Vec<u8>> {
        let g = self.instances.read().unwrap_or_else(|e| e.into_inner());
        let instance = g.get(&instance_id).ok_or_else(|| status(STATUS_OBJECT_DOES_NOT_EXIST))?;

        if attr_id == ATTR_DATA {
            return Ok(instance.data());
        }
        // Attribute 4 (Size) and everything else is not implemented (as in Go).
        Err(status(STATUS_ATTRIBUTE_NOT_SUPPORTED))
    }

    // Ref: ODVA CIP Vol. 1, Common Services - Set_Attribute_Single (0x10); Assembly Object
    // attribute 3 "Data" (section unverified).
    // Attribute 3 (Data) copies the provided bytes into the instance buffer. The length must
    // match the registered buffer size exactly, else StatusInvalidAttributeValue.
    pub fn set_attribute_single(&self, instance_id: u32, attr_id: u16, data: &[u8]) -> Result<()> {
        // Implementation detail: Go takes the object-wide write lock here.
        let g = self.instances.write().unwrap_or_else(|e| e.into_inner());
        let instance = g.get(&instance_id).ok_or_else(|| status(STATUS_OBJECT_DOES_NOT_EXIST))?;

        if attr_id == ATTR_DATA {
            let mut buf = instance.data.lock().unwrap_or_else(|e| e.into_inner());
            if data.len() != buf.len() {
                // Assembly size is fixed; strict size match enforced (as in Go).
                return Err(status(STATUS_INVALID_ATTRIBUTE_VALUE));
            }
            buf.copy_from_slice(data);
            return Ok(());
        }
        Err(status(STATUS_ATTRIBUTE_NOT_SUPPORTED))
    }
}

// Ref: ODVA CIP Vol. 1, Message Router request path: class, instance, attribute logical
// segments (Appendix C, section unverified). The router has already consumed the class
// segment, so `path` holds instance [+ attribute].
// Decodes the instance (0x24/0x25) and optional attribute (0x30/0x31) segments, then
// dispatches to get/set_attribute_single.
impl Object for AssemblyObject {
    fn handle_request(&self, service: USINT, path: &Path, data: &[u8]) -> Result<Vec<u8>> {
        let path_bytes = path.bytes();
        if path_bytes.is_empty() {
            return Err(status(STATUS_PATH_SEGMENT_ERROR));
        }

        let (instance_id, remaining): (u32, &[u8]) = match path_bytes[0] {
            SEG_INSTANCE_8 => {
                if path_bytes.len() < 2 {
                    return Err(status(STATUS_PATH_SEGMENT_ERROR));
                }
                (path_bytes[1] as u32, &path_bytes[2..])
            }
            SEG_INSTANCE_16 => {
                if path_bytes.len() < 4 {
                    return Err(status(STATUS_PATH_SEGMENT_ERROR));
                }
                (u16::from_le_bytes([path_bytes[2], path_bytes[3]]) as u32, &path_bytes[4..])
            }
            _ => return Err(status(STATUS_PATH_SEGMENT_ERROR)),
        };

        // Attribute ID, if present; 0 means "not present".
        let mut attr_id: u16 = 0;
        if let Some(&seg) = remaining.first() {
            if seg == SEG_ATTRIBUTE_8 {
                if remaining.len() < 2 {
                    return Err(status(STATUS_PATH_SEGMENT_ERROR));
                }
                attr_id = remaining[1] as u16;
            } else if seg == SEG_ATTRIBUTE_16 {
                if remaining.len() < 4 {
                    return Err(status(STATUS_PATH_SEGMENT_ERROR));
                }
                attr_id = u16::from_le_bytes([remaining[2], remaining[3]]);
            }
        }

        match service {
            SERVICE_GET_ATTRIBUTE_SINGLE => {
                if attr_id == 0 {
                    return Err(status(STATUS_PATH_SEGMENT_ERROR)); // attribute required
                }
                self.get_attribute_single(instance_id, attr_id)
            }
            SERVICE_SET_ATTRIBUTE_SINGLE => {
                if attr_id == 0 {
                    return Err(status(STATUS_PATH_SEGMENT_ERROR));
                }
                self.set_attribute_single(instance_id, attr_id, data).map(|_| Vec::new())
            }
            _ => Err(status(STATUS_SERVICE_NOT_SUPPORTED)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    // Extracts the CIP general status from an error.
    fn st(e: CipError) -> u8 {
        match e {
            CipError::Status(e) => e.status,
            other => panic!("expected cip status error, got {:?}", other),
        }
    }

    #[test]
    fn register_and_get_instance() {
        let ao = AssemblyObject::new();
        ao.register_assembly(100, vec![0; 8]);
        ao.register_assembly(200, vec![0; 16]);

        let inst = ao.get_instance(100).expect("GetInstance(100) returned nil");
        assert_eq!(inst.id, 100);
        assert_eq!(inst.len(), 8);

        let inst2 = ao.get_instance(200).expect("GetInstance(200) returned nil");
        assert_eq!(inst2.len(), 16);
    }

    #[test]
    fn get_instance_nonexistent() {
        let ao = AssemblyObject::new();
        assert!(ao.get_instance(999).is_none());
    }

    #[test]
    fn get_set_attribute_single() {
        let ao = AssemblyObject::new();
        ao.register_assembly(10, vec![0; 4]);

        let write = [0xDE, 0xAD, 0xBE, 0xEF];
        ao.set_attribute_single(10, 3, &write).unwrap();

        let mut read = ao.get_attribute_single(10, 3).unwrap();
        assert_eq!(read, write);

        // The returned buffer must be a copy.
        read[0] = 0;
        let read2 = ao.get_attribute_single(10, 3).unwrap();
        assert_eq!(read2[0], 0xDE, "returned a reference to internal data instead of a copy");
    }

    #[test]
    fn set_attribute_single_size_mismatch() {
        let ao = AssemblyObject::new();
        ao.register_assembly(10, vec![0; 4]);
        let err = ao.set_attribute_single(10, 3, &[1, 2]).unwrap_err();
        assert_eq!(st(err), STATUS_INVALID_ATTRIBUTE_VALUE);
    }

    #[test]
    fn get_attribute_single_invalid_instance() {
        let ao = AssemblyObject::new();
        let err = ao.get_attribute_single(999, 3).unwrap_err();
        assert_eq!(st(err), STATUS_OBJECT_DOES_NOT_EXIST);
    }

    #[test]
    fn set_attribute_single_invalid_instance() {
        let ao = AssemblyObject::new();
        assert!(ao.set_attribute_single(999, 3, &[1, 2, 3, 4]).is_err());
    }

    #[test]
    fn unsupported_attribute() {
        let ao = AssemblyObject::new();
        ao.register_assembly(10, vec![0; 4]);
        // Attribute 4 (Size) is not implemented.
        assert!(ao.get_attribute_single(10, 4).is_err());
        assert!(ao.set_attribute_single(10, 99, &[1, 2, 3, 4]).is_err());
    }

    #[test]
    fn concurrent_access() {
        let ao = Arc::new(AssemblyObject::new());
        ao.register_assembly(1, vec![0; 4]);

        let mut handles = Vec::new();
        for i in 0..20 {
            let ao = Arc::clone(&ao);
            handles.push(thread::spawn(move || {
                for j in 0..100u8 {
                    if i % 2 == 0 {
                        let _ = ao.set_attribute_single(1, 3, &[j, j, j, j]);
                    } else {
                        let _ = ao.get_attribute_single(1, 3);
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let data = ao.get_attribute_single(1, 3).unwrap();
        assert_eq!(data.len(), 4);
    }

    #[test]
    fn handle_request_get_attribute() {
        let ao = AssemblyObject::new();
        ao.register_assembly(5, vec![0xAA, 0xBB, 0xCC, 0xDD]);

        // Path: Instance 5 (8-bit), Attribute 3 (8-bit)
        let path = Path(vec![0x24, 0x05, 0x30, 0x03]);
        let data = ao.handle_request(SERVICE_GET_ATTRIBUTE_SINGLE, &path, &[]).unwrap();
        assert_eq!(data, vec![0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn handle_request_set_attribute() {
        let ao = AssemblyObject::new();
        ao.register_assembly(5, vec![0; 4]);

        let path = Path(vec![0x24, 0x05, 0x30, 0x03]);
        let write = [0x01, 0x02, 0x03, 0x04];
        ao.handle_request(SERVICE_SET_ATTRIBUTE_SINGLE, &path, &write).unwrap();

        let read = ao.handle_request(SERVICE_GET_ATTRIBUTE_SINGLE, &path, &[]).unwrap();
        assert_eq!(read, write);
    }

    // Added: path edge cases (16-bit segments, malformed paths) must error, not panic.
    #[test]
    fn handle_request_path_variants_and_errors() {
        let ao = AssemblyObject::new();
        ao.register_assembly(0x0102, vec![9, 8]);

        let p16 = Path(vec![0x25, 0x00, 0x02, 0x01, 0x31, 0x00, 0x03, 0x00]);
        assert_eq!(ao.handle_request(SERVICE_GET_ATTRIBUTE_SINGLE, &p16, &[]).unwrap(), vec![9, 8]);

        for bad in [vec![], vec![0x24], vec![0x25, 0, 1], vec![0x20, 4], vec![0x24, 1, 0x30], vec![0x24, 1, 0x31, 0, 3]] {
            let e = ao.handle_request(SERVICE_GET_ATTRIBUTE_SINGLE, &Path(bad), &[]).unwrap_err();
            assert_eq!(st(e), STATUS_PATH_SEGMENT_ERROR);
        }
        // No attribute segment -> attribute required.
        let e = ao.handle_request(SERVICE_GET_ATTRIBUTE_SINGLE, &Path(vec![0x24, 1]), &[]).unwrap_err();
        assert_eq!(st(e), STATUS_PATH_SEGMENT_ERROR);
        // Unsupported service.
        let e = ao.handle_request(0x01, &Path(vec![0x24, 1, 0x30, 3]), &[]).unwrap_err();
        assert_eq!(st(e), STATUS_SERVICE_NOT_SUPPORTED);
    }
}
