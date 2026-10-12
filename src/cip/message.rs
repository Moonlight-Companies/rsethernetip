//! CIP Message Router request/response PDUs.
//!
//! Port of Go `cip/message.go`.

use super::path::Path;
use super::types::*;

// Ref: ODVA CIP Vol. 1, Message Router Request format (Service, Request_Path_Size, Request_Path,
// Request_Data; section unverified). Carries a service code, the EPATH to the target object and
// optional request data. The Message Router (class 0x02) dispatches the request to the
// addressed object.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MessageRouterRequest {
    pub service: USINT,
    pub request_path: Path,
    pub request_data: Vec<u8>,
}

impl MessageRouterRequest {
    // Ref: CIP Vol. 1, Message Router Request format (section unverified).
    // Serializes the request: [Service:1][PathSizeWords:1][Path...][RequestData...].
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(2 + self.request_path.len() + self.request_data.len());
        buf.push(self.service);
        buf.push(self.request_path.len_words());
        buf.extend_from_slice(self.request_path.bytes());
        if !self.request_data.is_empty() {
            buf.extend_from_slice(&self.request_data);
        }
        Ok(buf)
    }
}

// Ref: CIP Vol. 1, Message Router Request format (section unverified).
// Decodes a byte slice into a MessageRouterRequest. Wire format:
// [Service:1][PathSizeWords:1][Path...][RequestData...].
//
// Go behavior retained: like Go's bytes.Reader.Read, when the data after the 2-byte header is
// completely exhausted this returns EOF (even if the path size is 0), and when the data is
// shorter than the declared path size the path is zero-padded to the declared length instead
// of failing (noted as a Go quirk; no panic possible here).
pub fn decode_message_router_request(data: &[u8]) -> Result<MessageRouterRequest> {
    let mut r = MessageRouterRequest::default();
    let (&service, rest) = data.split_first().ok_or(CipError::Eof)?;
    r.service = service;
    let (&path_size_words, rest) = rest.split_first().ok_or(CipError::Eof)?;

    let want = path_size_words as usize * 2;
    if rest.is_empty() {
        return Err(CipError::Eof);
    }
    let n = want.min(rest.len());
    let mut path_bytes = vec![0u8; want];
    path_bytes[..n].copy_from_slice(&rest[..n]);
    r.request_path = Path(path_bytes);

    let remaining = &rest[n..];
    if !remaining.is_empty() {
        r.request_data = remaining.to_vec();
    }
    Ok(r)
}

// Ref: ODVA CIP Vol. 1, Message Router Response format (Reply Service, Reserved, General Status,
// Size of Additional Status, Additional Status, Response Data; section unverified). The Service
// field echoes the request service code with bit 7 set (OR 0x80). GeneralStatus is success
// (0x00) or an error code; ExtStatus carries optional additional detail.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MessageRouterResponse {
    // Reply Service (Request Service | 0x80)
    pub service: USINT,
    pub reserved: USINT,
    pub general_status: USINT,
    pub ext_status_size: USINT,
    pub ext_status: Vec<UINT>,
    pub response_data: Vec<u8>,
}

impl MessageRouterResponse {
    // Ref: CIP Vol. 1, Message Router Response format (section unverified).
    // Serializes the response into the CIP wire format. Note (Go behavior): ext_status_size is
    // written as stored, not derived from ext_status.len().
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(4 + self.ext_status.len() * 2 + self.response_data.len());
        buf.push(self.service);
        buf.push(self.reserved);
        buf.push(self.general_status);
        buf.push(self.ext_status_size);
        for ext in &self.ext_status {
            buf.extend_from_slice(&ext.to_le_bytes());
        }
        buf.extend_from_slice(&self.response_data);
        Ok(buf)
    }

    // Ref: CIP Vol. 1, Appendix B General Status Codes: 0x00 = Success (section unverified).
    // Returns true if the general status is [`STATUS_SUCCESS`].
    pub fn is_success(&self) -> bool {
        self.general_status == STATUS_SUCCESS
    }

    // Ref: CIP Vol. 1, Appendix B General Status Codes (section unverified).
    // Returns a structured [`Error`] if the response indicates failure, or None on success
    // (Go `MessageRouterResponse.Error`).
    pub fn error(&self) -> Option<Error> {
        if self.is_success() {
            return None;
        }
        Some(Error { status: self.general_status, ext_status: self.ext_status.clone() })
    }
}

// Ref: CIP Vol. 1, Message Router Response format (section unverified).
// Decodes a byte slice into a MessageRouterResponse.
pub fn decode_message_router_response(data: &[u8]) -> Result<MessageRouterResponse> {
    let mut r = MessageRouterResponse::default();
    let mut pos = 0usize;

    // Reads one byte, mirroring binary.Read on a uint8 (EOF when nothing is left).
    fn take_u8(data: &[u8], pos: &mut usize) -> Result<u8> {
        let b = *data.get(*pos).ok_or(CipError::Eof)?;
        *pos += 1;
        Ok(b)
    }

    r.service = take_u8(data, &mut pos)?;
    r.reserved = take_u8(data, &mut pos)?;
    r.general_status = take_u8(data, &mut pos)?;
    r.ext_status_size = take_u8(data, &mut pos)?;

    if r.ext_status_size > 0 {
        // Bounded: at most 255 words.
        r.ext_status = Vec::with_capacity(r.ext_status_size as usize);
        for _ in 0..r.ext_status_size {
            let rest = &data[pos..];
            if rest.is_empty() {
                return Err(CipError::Eof);
            }
            if rest.len() < 2 {
                return Err(CipError::UnexpectedEof);
            }
            r.ext_status.push(u16::from_le_bytes([rest[0], rest[1]]));
            pos += 2;
        }
    }

    // The rest is response data
    let remaining = &data[pos..];
    if !remaining.is_empty() {
        r.response_data = remaining.to_vec();
    }

    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cip::services::*;

    // ===== MessageRouterRequest Encode =====
    // Ref: CIP Vol. 1, Message Router Request format.
    #[test]
    fn message_router_request_encode() {
        let req = MessageRouterRequest {
            service: SERVICE_READ_TAG,
            request_path: Path::from(vec![0x20, 0x04, 0x24, 0x01]),
            request_data: vec![0x01, 0x00],
        };
        let data = req.encode().unwrap();
        // Service(1) + PathLenWords(1) + Path(4) + Data(2) = 8
        assert_eq!(data.len(), 8);
        assert_eq!(data[0], SERVICE_READ_TAG);
        assert_eq!(data[1], 2); // 4 bytes / 2 = 2 words
        assert_eq!(&data[2..6], &[0x20, 0x04, 0x24, 0x01]);
    }

    #[test]
    fn message_router_request_encode_empty() {
        let req = MessageRouterRequest {
            service: SERVICE_GET_ATTRIBUTE_SINGLE,
            request_path: Path::from(vec![0x20, 0x01, 0x24, 0x01]),
            request_data: Vec::new(),
        };
        let data = req.encode().unwrap();
        assert_eq!(data.len(), 6);
    }

    // Round trip of the request decoder (added; Go has no decode-request test).
    #[test]
    fn message_router_request_round_trip() {
        let req = MessageRouterRequest {
            service: SERVICE_READ_TAG,
            request_path: Path::from(vec![0x20, 0x04, 0x24, 0x01]),
            request_data: vec![0x01, 0x00],
        };
        let data = req.encode().unwrap();
        let got = decode_message_router_request(&data).unwrap();
        assert_eq!(got, req);
        // Truncated / empty input never panics.
        assert_eq!(decode_message_router_request(&[]), Err(CipError::Eof));
        assert_eq!(decode_message_router_request(&[0x4C]), Err(CipError::Eof));
        assert_eq!(decode_message_router_request(&[0x4C, 0x02]), Err(CipError::Eof));
    }

    // ===== MessageRouterResponse Decode =====
    // Ref: CIP Vol. 1, Message Router Response format.
    #[test]
    fn message_router_response_decode_success() {
        // Build a known-good response: Service|0x80, Reserved, Status=0, ExtSize=0, Data
        let resp_bytes = [
            SERVICE_READ_TAG | 0x80, // Reply service
            0x00,                    // Reserved
            STATUS_SUCCESS,          // General status
            0x00,                    // Ext status size
            0xC4, 0x00, 0x2A, 0x00, 0x00, 0x00, // Response data (TypeDINT + 42)
        ];
        let resp = decode_message_router_response(&resp_bytes).unwrap();
        assert!(resp.is_success(), "expected success, got status 0x{:02X}", resp.general_status);
        assert!(resp.error().is_none(), "expected nil error");
        assert_eq!(resp.response_data.len(), 6);
        assert_eq!(resp.service, SERVICE_READ_TAG | 0x80);
    }

    #[test]
    fn message_router_response_decode_error() {
        let resp_bytes = [SERVICE_READ_TAG | 0x80, 0x00, STATUS_OBJECT_DOES_NOT_EXIST, 0x00];
        let resp = decode_message_router_response(&resp_bytes).unwrap();
        assert!(!resp.is_success(), "expected failure");
        assert!(resp.error().is_some(), "expected non-nil error");
    }

    #[test]
    fn message_router_response_decode_ext_status() {
        // Response with 1 extended status word
        let resp_bytes = [
            SERVICE_READ_TAG | 0x80,
            0x00,
            STATUS_CONNECTION_FAILURE,
            0x01, // 1 word of ext status
            0x00, 0x01, // Extended status 0x0100
        ];
        let resp = decode_message_router_response(&resp_bytes).unwrap();
        assert_eq!(resp.ext_status.len(), 1);
        assert_eq!(resp.ext_status[0], 0x0100);
    }

    // Truncated responses error instead of panicking (added).
    #[test]
    fn message_router_response_decode_truncated() {
        assert_eq!(decode_message_router_response(&[]), Err(CipError::Eof));
        assert_eq!(decode_message_router_response(&[0xCC, 0, 0]), Err(CipError::Eof));
        assert_eq!(decode_message_router_response(&[0xCC, 0, 4, 1]), Err(CipError::Eof));
        assert_eq!(decode_message_router_response(&[0xCC, 0, 4, 1, 0x00]), Err(CipError::UnexpectedEof));
    }

    // Response encode/decode round trip (added; Go encode has no direct test).
    #[test]
    fn message_router_response_round_trip() {
        let resp = MessageRouterResponse {
            service: 0xCC,
            reserved: 0,
            general_status: STATUS_CONNECTION_FAILURE,
            ext_status_size: 1,
            ext_status: vec![0x0100],
            response_data: vec![1, 2, 3],
        };
        let b = resp.encode().unwrap();
        assert_eq!(decode_message_router_response(&b).unwrap(), resp);
    }

    // --- MessageRouter response decoding from wire captures (cpppo) ---

    // cpppo unk_014_reply CPF data inner CIP response from a real Logix PLC.
    #[test]
    fn message_router_response_decode_tag_reply() {
        let data = [
            0xD2, // Service = 0x52 | 0x80 (reply)
            0x00, // Reserved
            0x00, // Status = success
            0x00, // Extended status size = 0
            0xC3, 0x00, // Response data: type code 0x00C3 (INT)
            0x27, 0x80, // INT value = 0x8027
        ];
        let resp = decode_message_router_response(&data).unwrap();
        assert_eq!(resp.service, 0xD2);
        assert_eq!(resp.general_status, 0x00);
        assert_eq!(resp.ext_status.len(), 0);
        // Response data should be the type code + value
        assert_eq!(resp.response_data.len(), 4);
    }

    // cpppo rfg_002_reply - CIP error 0x04 with extended status.
    #[test]
    fn message_router_response_decode_path_error() {
        let data = [
            0xD2, // Service
            0x00, // Reserved
            0x04, // Status = 0x04 (path segment error)
            0x01, // Extended status size = 1 word
            0x00, 0x00, // Extended status word = 0x0000
        ];
        let resp = decode_message_router_response(&data).unwrap();
        assert_eq!(resp.general_status, 0x04);
        assert_eq!(resp.ext_status.len(), 1);
        assert_eq!(resp.ext_status[0], 0x0000);
    }

    // cpppo rfg_001_reply - CIP error 0x05.
    #[test]
    fn message_router_response_decode_general_error() {
        let data = [0xD2, 0x00, 0x05, 0x00];
        let resp = decode_message_router_response(&data).unwrap();
        assert_eq!(resp.general_status, 0x05);
        assert_eq!(resp.ext_status.len(), 0);
    }
}
