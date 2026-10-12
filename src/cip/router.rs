//! CIP Message Router: dispatches requests to registered objects by class ID.
//!
//! Port of Go `cip/router.go`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::message::{MessageRouterRequest, MessageRouterResponse};
use super::path::Path;
use super::types::*;

// Ref: ODVA CIP Vol. 1, Message Router Object (class 0x02) - objects receive explicit service
// requests routed by class (section unverified).
// Implemented by CIP objects to receive service requests dispatched by the [`MessageRouter`].
pub trait Object: Send + Sync {
    // Processes a CIP service request addressed to this object. `path` contains the remaining
    // EPATH segments after the class segment (typically instance and attribute). Returns the
    // response data or an error. Returning `CipError::Status` yields that status in the reply;
    // any other error yields General Status 0x08 (Service not supported), as in Go.
    fn handle_request(&self, service: USINT, path: &Path, data: &[u8]) -> Result<Vec<u8>>;
}

// Ref: ODVA CIP Vol. 1, Object Library - Message Router Object (class 0x02; section unverified).
// Maintains a registry of [`Object`] implementations keyed by class ID and dispatches incoming
// requests to the appropriate object. Registrations are protected by an RwLock (implementation
// detail).
pub struct MessageRouter {
    // Map of Class ID -> Object
    objects: RwLock<HashMap<UINT, Arc<dyn Object>>>,
    // Handler for symbolic segment (0x91) requests
    symbolic_handler: RwLock<Option<Arc<dyn Object>>>,
}

impl Default for MessageRouter {
    fn default() -> Self {
        MessageRouter::new()
    }
}

impl MessageRouter {
    // Creates a new empty MessageRouter with no registered objects.
    pub fn new() -> MessageRouter {
        MessageRouter { objects: RwLock::new(HashMap::new()), symbolic_handler: RwLock::new(None) }
    }

    // Ref: CIP Vol. 1, Message Router Object (section unverified).
    // Registers a CIP [`Object`] under the given class ID (replaces any previous registration).
    pub fn register_object(&self, class_id: UINT, obj: Arc<dyn Object>) {
        self.objects.write().unwrap_or_else(|e| e.into_inner()).insert(class_id, obj);
    }

    // Ref: CIP Vol. 1, Appendix C ANSI Extended Symbol segment (0x91; section unverified);
    // Rockwell 1756-PM020 Logix 5000 Data Access symbolic tag addressing.
    // Registers an [`Object`] to handle requests whose path starts with an ANSI Extended Symbol
    // segment (0x91), used for Logix-style tag access.
    pub fn register_symbolic_handler(&self, obj: Arc<dyn Object>) {
        *self.symbolic_handler.write().unwrap_or_else(|e| e.into_inner()) = Some(obj);
    }

    // Ref: CIP Vol. 1, Message Router Object - routing by the class segment of the request path
    // (section unverified); logical class segment 0x20 (8-bit) / 0x21 (16-bit), CIP Vol. 1
    // Appendix C.
    // Routes a CIP request to the registered object that owns the target class ID. The class ID
    // is extracted from the first path segment (8-bit 0x20 and 16-bit 0x21 formats). The
    // remaining path (instance, attribute, ...) is forwarded to the object's `handle_request`.
    // Path errors are returned as `Err(Error)` (like Go); object errors become status replies.
    pub fn dispatch(&self, req: &MessageRouterRequest) -> std::result::Result<MessageRouterResponse, Error> {
        let path_bytes = req.request_path.bytes();
        if path_bytes.is_empty() {
            return Err(Error::new(STATUS_PATH_SEGMENT_ERROR));
        }

        let class_id: UINT;
        let remaining_path: Path;

        match path_bytes[0] {
            0x20 => {
                // 8-bit class ID
                if path_bytes.len() < 2 {
                    return Err(Error::new(STATUS_PATH_SEGMENT_ERROR));
                }
                class_id = path_bytes[1] as UINT;
                remaining_path = Path::from(&path_bytes[2..]);
            }
            0x21 => {
                // 16-bit class ID: [0x21] [pad] [ID_LO] [ID_HI]
                if path_bytes.len() < 4 {
                    return Err(Error::new(STATUS_PATH_SEGMENT_ERROR));
                }
                class_id = u16::from_le_bytes([path_bytes[2], path_bytes[3]]);
                remaining_path = Path::from(&path_bytes[4..]);
            }
            0x91 => {
                // ANSI Extended Symbol segment - route to symbolic handler
                let handler = self.symbolic_handler.read().unwrap_or_else(|e| e.into_inner()).clone();
                let handler = match handler {
                    Some(h) => h,
                    None => return Err(Error::new(STATUS_PATH_SEGMENT_ERROR)),
                };
                let result = handler.handle_request(req.service, &req.request_path, &req.request_data);
                return Ok(Self::reply(req.service, result));
            }
            _ => return Err(Error::new(STATUS_PATH_SEGMENT_ERROR)),
        }

        let obj = self.objects.read().unwrap_or_else(|e| e.into_inner()).get(&class_id).cloned();
        let obj = match obj {
            Some(o) => o,
            None => {
                return Ok(MessageRouterResponse {
                    service: req.service | 0x80,
                    general_status: STATUS_OBJECT_DOES_NOT_EXIST,
                    ..Default::default()
                });
            }
        };

        let result = obj.handle_request(req.service, &remaining_path, &req.request_data);
        Ok(Self::reply(req.service, result))
    }

    // Ref: CIP Vol. 1, Message Router Response format - reply service = request | 0x80, general
    // status and additional status (section unverified).
    // Builds the reply for an object result (shared by both dispatch paths in Go).
    fn reply(service: USINT, result: Result<Vec<u8>>) -> MessageRouterResponse {
        match result {
            Ok(resp_data) => MessageRouterResponse {
                service: service | 0x80,
                general_status: STATUS_SUCCESS,
                response_data: resp_data,
                ..Default::default()
            },
            Err(CipError::Status(cip_err)) => MessageRouterResponse {
                service: service | 0x80,
                general_status: cip_err.status,
                ext_status_size: cip_err.ext_status.len() as USINT,
                ext_status: cip_err.ext_status,
                ..Default::default()
            },
            Err(_) => MessageRouterResponse {
                service: service | 0x80,
                general_status: STATUS_SERVICE_NOT_SUPPORTED,
                ..Default::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    type Handler = dyn Fn(USINT, &Path, &[u8]) -> Result<Vec<u8>> + Send + Sync;

    // Test helper object wrapping a closure (Go `testObject`).
    struct TestObject {
        f: Box<Handler>,
    }

    impl Object for TestObject {
        fn handle_request(&self, service: USINT, path: &Path, data: &[u8]) -> Result<Vec<u8>> {
            (self.f)(service, path, data)
        }
    }

    // ===== MessageRouter Dispatch =====
    // Ref: CIP Vol. 1, Message Router Object routing by class segment.

    #[test]
    fn dispatch_8bit_class() {
        let router = MessageRouter::new();
        let called = Arc::new(AtomicBool::new(false));
        let c = called.clone();
        router.register_object(
            0x04,
            Arc::new(TestObject {
                f: Box::new(move |_s, _p, _d| {
                    c.store(true, Ordering::SeqCst);
                    Ok(vec![0x42])
                }),
            }),
        );

        let req = MessageRouterRequest {
            service: SERVICE_GET_ATTRIBUTE_SINGLE,
            request_path: Path::from(vec![0x20, 0x04, 0x24, 0x01]),
            request_data: Vec::new(),
        };
        let resp = router.dispatch(&req).unwrap();
        assert!(called.load(Ordering::SeqCst), "object not called");
        assert!(resp.is_success(), "dispatch failed: status 0x{:02X}", resp.general_status);
        assert_eq!(resp.response_data, vec![0x42]);
    }

    #[test]
    fn dispatch_16bit_class() {
        let router = MessageRouter::new();
        let called = Arc::new(AtomicBool::new(false));
        let c = called.clone();
        router.register_object(
            0x0100,
            Arc::new(TestObject {
                f: Box::new(move |_s, _p, _d| {
                    c.store(true, Ordering::SeqCst);
                    Ok(Vec::new())
                }),
            }),
        );

        let req = MessageRouterRequest {
            service: SERVICE_GET_ATTRIBUTE_SINGLE,
            request_path: Path::from(vec![0x21, 0x00, 0x00, 0x01, 0x24, 0x01]),
            request_data: Vec::new(),
        };
        let resp = router.dispatch(&req).unwrap();
        assert!(called.load(Ordering::SeqCst), "16-bit class object not called");
        assert!(resp.is_success(), "dispatch failed: status 0x{:02X}", resp.general_status);
    }

    #[test]
    fn dispatch_unknown_class() {
        let router = MessageRouter::new();
        let req = MessageRouterRequest {
            service: SERVICE_GET_ATTRIBUTE_SINGLE,
            request_path: Path::from(vec![0x20, 0xFF, 0x24, 0x01]),
            request_data: Vec::new(),
        };
        let resp = router.dispatch(&req).unwrap();
        assert!(!resp.is_success(), "expected failure for unknown class");
        assert_eq!(resp.general_status, STATUS_OBJECT_DOES_NOT_EXIST);
    }

    #[test]
    fn dispatch_empty_path() {
        let router = MessageRouter::new();
        let req = MessageRouterRequest {
            service: SERVICE_GET_ATTRIBUTE_SINGLE,
            request_path: Path::default(),
            request_data: Vec::new(),
        };
        assert!(router.dispatch(&req).is_err(), "expected error for empty path");
    }

    #[test]
    fn dispatch_cip_error() {
        let router = MessageRouter::new();
        router.register_object(
            0x04,
            Arc::new(TestObject { f: Box::new(|_s, _p, _d| Err(Error::new(STATUS_SERVICE_NOT_SUPPORTED).into())) }),
        );
        let req = MessageRouterRequest {
            service: 0xFF, // Unsupported
            request_path: Path::from(vec![0x20, 0x04, 0x24, 0x01]),
            request_data: Vec::new(),
        };
        let resp = router.dispatch(&req).unwrap();
        assert_eq!(resp.general_status, STATUS_SERVICE_NOT_SUPPORTED);
    }

    // Added coverage of Go behavior not exercised by the Go tests: the remaining path passed to
    // the object, truncated class segments, non-CIP errors, ext status and the symbolic handler.
    #[test]
    fn dispatch_extra_behavior() {
        let router = MessageRouter::new();
        let seen = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let s = seen.clone();
        router.register_object(
            0x04,
            Arc::new(TestObject {
                f: Box::new(move |_svc, p, _d| {
                    *s.lock().unwrap() = p.bytes().to_vec();
                    Err(CipError::Other("boom".into()))
                }),
            }),
        );
        let req = MessageRouterRequest {
            service: 0x0E,
            request_path: Path::from(vec![0x20, 0x04, 0x24, 0x01]),
            request_data: Vec::new(),
        };
        let resp = router.dispatch(&req).unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![0x24, 0x01]);
        // Non-CIP errors map to ServiceNotSupported; reply service has bit 7 set.
        assert_eq!(resp.general_status, STATUS_SERVICE_NOT_SUPPORTED);
        assert_eq!(resp.service, 0x8E);

        // Truncated class segments and unknown first segment types are path errors.
        for p in [vec![0x20], vec![0x21, 0x00, 0x01], vec![0x24, 0x01]] {
            let r = MessageRouterRequest { service: 1, request_path: Path(p), request_data: vec![] };
            assert_eq!(router.dispatch(&r).unwrap_err().status, STATUS_PATH_SEGMENT_ERROR);
        }

        // Ext status is carried through with its size.
        router.register_object(
            0x05,
            Arc::new(TestObject {
                f: Box::new(|_s, _p, _d| Err(Error::with_ext(STATUS_CONNECTION_FAILURE, vec![0x0100, 0x0200]).into())),
            }),
        );
        let r = MessageRouterRequest { service: 1, request_path: Path(vec![0x20, 0x05]), request_data: vec![] };
        let resp = router.dispatch(&r).unwrap();
        assert_eq!(resp.ext_status_size, 2);
        assert_eq!(resp.ext_status, vec![0x0100, 0x0200]);

        // Symbolic handler: absent -> path error; present -> receives the whole path.
        let sym = MessageRouterRequest { service: 0x4C, request_path: Path(vec![0x91, 0x02, b'A', b'B']), request_data: vec![1, 0] };
        assert_eq!(router.dispatch(&sym).unwrap_err().status, STATUS_PATH_SEGMENT_ERROR);
        router.register_symbolic_handler(Arc::new(TestObject {
            f: Box::new(|svc, p, d| {
                assert_eq!(svc, 0x4C);
                assert_eq!(p.bytes().len(), 4);
                Ok(d.to_vec())
            }),
        }));
        let resp = router.dispatch(&sym).unwrap();
        assert!(resp.is_success());
        assert_eq!(resp.service, 0xCC);
        assert_eq!(resp.response_data, vec![1, 0]);
    }
}
