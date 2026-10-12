//! Service request builders (Get/Set_Attribute_Single, Read/Write Tag, Execute_PCCC).
//!
//! Port of Go `cip/services.go`.

use super::message::MessageRouterRequest;
use super::path::{build_path, Path};
use super::types::*;

// Ref: ODVA CIP Vol. 1, Appendix A "Explicit Messaging Services" - Get_Attribute_Single (0x0E);
// request format per CIP Vol. 1 Message Router Request (section unverified).
// Creates a CIP Get_Attribute_Single (0x0E) request targeting the object addressed by `path`.
pub fn new_get_attribute_single_request(path: Path) -> MessageRouterRequest {
    MessageRouterRequest {
        service: SERVICE_GET_ATTRIBUTE_SINGLE,
        request_path: path,
        request_data: Vec::new(),
    }
}

// Ref: ODVA CIP Vol. 1, Appendix A "Explicit Messaging Services" - Set_Attribute_Single (0x10)
// (section unverified). Creates a Set_Attribute_Single request with the given data payload.
pub fn new_set_attribute_single_request(path: Path, data: &[u8]) -> MessageRouterRequest {
    MessageRouterRequest {
        service: SERVICE_SET_ATTRIBUTE_SINGLE,
        request_path: path,
        request_data: data.to_vec(),
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Read Tag service (0x4C), used instead of the
// generic CIP Get/Set_Attribute services when talking to Logix controllers.
pub const SERVICE_READ_TAG: USINT = 0x4C;
// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Write Tag service (0x4D).
pub const SERVICE_WRITE_TAG: USINT = 0x4D;
// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Read Tag Fragmented service (0x52).
pub const SERVICE_READ_TAG_FRAGMENTED: USINT = 0x52;
// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Write Tag Fragmented service (0x53).
pub const SERVICE_WRITE_TAG_FRAGMENTED: USINT = 0x53;

// Ref: Allen-Bradley PCCC Object (class 0x67) Execute_PCCC service 0x4B; Allen-Bradley PCCC /
// DF1 protocol manual 1770-6.5.16 (section unverified). Tunnels a PCCC command inside a CIP
// message router request addressed to the PCCC object ([`CLASS_PCCC`]).
pub const SERVICE_EXECUTE_PCCC: USINT = 0x4B;

// Ref: Allen-Bradley PCCC Object (class 0x67, instance 1), Execute_PCCC (0x4B); requestor ID
// header per the PCCC-over-CIP encapsulation: Length:1 (always 7), Vendor:UINT, Serial:UDINT
// (Allen-Bradley PCCC / DF1 protocol manual 1770-6.5.16, section unverified).
// Builds an Execute_PCCC request. `pccc_cmd` is the raw PCCC command bytes (built with the pccc
// module). Request data is the requestor-ID header followed by the PCCC command.
pub fn new_execute_pccc_request(vendor_id: UINT, serial_number: UDINT, pccc_cmd: &[u8]) -> MessageRouterRequest {
    let mut req_data = Vec::with_capacity(7 + pccc_cmd.len());
    req_data.push(0x07); // Requestor ID length, including this byte
    req_data.extend_from_slice(&vendor_id.to_le_bytes());
    req_data.extend_from_slice(&serial_number.to_le_bytes());
    req_data.extend_from_slice(pccc_cmd);
    MessageRouterRequest {
        service: SERVICE_EXECUTE_PCCC,
        request_path: build_path(CLASS_PCCC, 1, 0),
        request_data: req_data,
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Read Tag (0x4C) request: path is the symbolic
// tag EPATH, request data is the element count (UINT).
// Creates a Read Tag request. `tag_path` should contain a symbolic segment addressing the tag by
// name; `elements` is how many array elements to read (1 for scalar tags).
pub fn new_read_tag_request(tag_path: Path, elements: u16) -> MessageRouterRequest {
    MessageRouterRequest {
        service: SERVICE_READ_TAG,
        request_path: tag_path,
        request_data: elements.to_le_bytes().to_vec(),
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - Write Tag (0x4D) request data: CIP data type
// (UINT), element count (UINT), then the raw data.
// Creates a Write Tag request. `data_type` is the CIP type code, `elements` the number of array
// elements, `data` the raw payload bytes.
pub fn new_write_tag_request(tag_path: Path, data_type: DataType, elements: u16, data: &[u8]) -> MessageRouterRequest {
    // Write Tag Request Data:
    // Data Type (UINT)
    // Number of Elements (UINT)
    // Data (...)
    let mut req_data = Vec::with_capacity(4 + data.len());
    req_data.extend_from_slice(&data_type.0.to_le_bytes());
    req_data.extend_from_slice(&elements.to_le_bytes());
    req_data.extend_from_slice(data);
    MessageRouterRequest {
        service: SERVICE_WRITE_TAG,
        request_path: tag_path,
        request_data: req_data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ref: Rockwell 1756-PM020 Read Tag request layout (Go behavior; no dedicated Go test).
    #[test]
    fn read_tag_request_layout() {
        let r = new_read_tag_request(Path::from(vec![0x91, 0x01, b'A', 0x00]), 0x0102);
        assert_eq!(r.service, 0x4C);
        assert_eq!(r.request_data, vec![0x02, 0x01]);
    }

    // Ref: Rockwell 1756-PM020 Write Tag request layout (Go behavior; no dedicated Go test).
    #[test]
    fn write_tag_request_layout() {
        let r = new_write_tag_request(Path::default(), TYPE_DINT, 1, &[1, 2, 3, 4]);
        assert_eq!(r.service, 0x4D);
        assert_eq!(r.request_data, vec![0xC4, 0x00, 0x01, 0x00, 1, 2, 3, 4]);
    }

    // Ref: Allen-Bradley PCCC Execute_PCCC requestor ID header (Go behavior).
    #[test]
    fn execute_pccc_request_layout() {
        let r = new_execute_pccc_request(0x0001, 0x04030201, &[0xAA, 0xBB]);
        assert_eq!(r.service, 0x4B);
        assert_eq!(r.request_path.bytes(), &[0x20, 0x67, 0x24, 0x01]);
        assert_eq!(r.request_data, vec![0x07, 0x01, 0x00, 0x01, 0x02, 0x03, 0x04, 0xAA, 0xBB]);
    }
}
