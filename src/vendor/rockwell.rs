//! Rockwell Automation (Allen-Bradley) Logix structured types as [`TypeCodec`]s: Timer
//! (TON/TOF/RTO), Counter (CTU/CTD/CTUD), PID and CONTROL.
//!
//! Port of Go `vendor/rockwell/rockwell.go`.
//!
//! Rockwell struct types are identified by vendor-specific CIP DataType codes which can vary
//! between controller models and firmware, so nothing is auto-registered. Call the
//! `register_*` functions with the codes discovered from your controller (e.g. by listing
//! tags):
//!
//! ```ignore
//! rockwell::register_timer(DataType(0x0F83));
//! rockwell::register_counter(DataType(0x0F84));
//! ```
//!
//! This registers the types in the global [`register_type`] registry, enabling automatic name
//! resolution in `DataType`'s Display and codec lookup via [`crate::cip::lookup_type`].

use std::ops::{Deref, DerefMut};

use crate::cip::{
    register_type, CipError, Counter as CipCounter, DataType, Error, Marshaler, Result,
    Timer as CipTimer, TypeCodec, Unmarshaler, STATUS_NOT_ENOUGH_DATA,
};

// Implementation detail: builds the Go `cip.Error{Status: StatusNotEnoughData}`.
fn not_enough_data() -> CipError {
    CipError::Status(Error::new(STATUS_NOT_ENOUGH_DATA))
}

// ---------- Timer ----------

// Ref: Rockwell 1756-RM003 Logix 5000 General Instructions Reference - TIMER structure
// (TON/TOF/RTO); wire layout per Rockwell 1756-PM020 Logix 5000 Data Access.
// Wraps [`cip::Timer`](CipTimer) as a [`TypeCodec`] with a controller-specific CIP DataType
// code (Go embeds the struct; here it derefs to it). Use [`register_timer`] to register it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Timer {
    pub timer: CipTimer,
    dt: DataType,
}

impl Timer {
    // Implementation detail: creates a zero-value Timer carrying the type code `dt`.
    pub fn new(dt: DataType) -> Timer {
        Timer { timer: CipTimer::default(), dt }
    }
}

impl Deref for Timer {
    type Target = CipTimer;
    fn deref(&self) -> &CipTimer {
        &self.timer
    }
}

impl DerefMut for Timer {
    fn deref_mut(&mut self) -> &mut CipTimer {
        &mut self.timer
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - TIMER memory layout (see cip::Timer).
impl Marshaler for Timer {
    fn marshal_cip(&self) -> Result<Vec<u8>> {
        self.timer.marshal_cip()
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - TIMER memory layout (12- or 14-byte).
impl Unmarshaler for Timer {
    fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
        self.timer.unmarshal_cip(data)
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - structure type code (controller-specific).
impl TypeCodec for Timer {
    fn cip_type(&self) -> DataType {
        self.dt
    }
    // Go `String()` returns "TIMER".
    fn display_name(&self) -> Option<String> {
        Some("TIMER".to_string())
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - structure type codes are
// controller-specific and discovered from the Symbol/Template objects.
// Registers [`Timer`] as a [`TypeCodec`] for the given DataType code. Panics if the code is
// already registered (as Go).
pub fn register_timer(dt: DataType) {
    register_type(dt, move || Box::new(Timer::new(dt)));
}

// ---------- Counter ----------

// Ref: Rockwell 1756-RM003 - COUNTER structure (CTU/CTD/CTUD); wire layout per 1756-PM020.
// Wraps [`cip::Counter`](CipCounter) as a [`TypeCodec`] with a controller-specific DataType
// code. Use [`register_counter`] to register it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counter {
    pub counter: CipCounter,
    dt: DataType,
}

impl Counter {
    // Implementation detail: creates a zero-value Counter carrying the type code `dt`.
    pub fn new(dt: DataType) -> Counter {
        Counter { counter: CipCounter::default(), dt }
    }
}

impl Deref for Counter {
    type Target = CipCounter;
    fn deref(&self) -> &CipCounter {
        &self.counter
    }
}

impl DerefMut for Counter {
    fn deref_mut(&mut self) -> &mut CipCounter {
        &mut self.counter
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - COUNTER memory layout.
impl Marshaler for Counter {
    fn marshal_cip(&self) -> Result<Vec<u8>> {
        self.counter.marshal_cip()
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - COUNTER memory layout.
impl Unmarshaler for Counter {
    fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
        self.counter.unmarshal_cip(data)
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - structure type code (controller-specific).
impl TypeCodec for Counter {
    fn cip_type(&self) -> DataType {
        self.dt
    }
    // Go `String()` returns "COUNTER".
    fn display_name(&self) -> Option<String> {
        Some("COUNTER".to_string())
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access (controller-specific type codes).
// Registers [`Counter`] as a [`TypeCodec`] for the given DataType code. Panics on duplicate.
pub fn register_counter(dt: DataType) {
    register_type(dt, move || Box::new(Counter::new(dt)));
}

// ---------- PID ----------

// Ref: Rockwell 1756-RM006 Logix 5000 Advanced Process Control and Drives Instruction Set
// Reference Manual - PID instruction structure (document number and field layout beyond the
// first 28 bytes unverified); Go source describes a 116-byte vendor-specific struct.
// Minimal representation of a Logix PID structure exposing the most commonly accessed fields.
// For complete PID configuration read the raw bytes instead.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pid {
    // Setpoint
    pub sp: f32,
    // Process Variable
    pub pv: f32,
    // Output (%)
    pub out: f32,
    // Error (SP - PV)
    pub err: f32,
    // Proportional gain
    pub kp: f32,
    // Integral gain
    pub ki: f32,
    // Derivative gain
    pub kd: f32,

    dt: DataType,
}

impl Pid {
    // Implementation detail: creates a zero-value Pid carrying the type code `dt`.
    pub fn new(dt: DataType) -> Pid {
        Pid { dt, ..Default::default() }
    }
}

// Ref: Rockwell PID structure (layout unverified) - decodes the first 28 bytes (seven
// little-endian REALs) into the commonly used fields. The full 116-byte layout contains tuning
// limits, scaling and mode bits not captured here. Fewer than 28 bytes yields
// StatusNotEnoughData.
impl Unmarshaler for Pid {
    fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < 28 {
            return Err(not_enough_data());
        }
        let f = |i: usize| f32::from_le_bytes([data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]]);
        self.sp = f(0);
        self.pv = f(1);
        self.out = f(2);
        self.err = f(3);
        self.kp = f(4);
        self.ki = f(5);
        self.kd = f(6);
        Ok(())
    }
}

// Ref: Rockwell PID structure (layout unverified) - encodes the 28-byte prefix. Writing
// individual PID members (e.g. "MyPID.SP") is usually preferred over the whole struct.
impl Marshaler for Pid {
    fn marshal_cip(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(28);
        for v in [self.sp, self.pv, self.out, self.err, self.kp, self.ki, self.kd] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        Ok(buf)
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - structure type code (controller-specific).
impl TypeCodec for Pid {
    fn cip_type(&self) -> DataType {
        self.dt
    }
    // Go `String()` returns "PID".
    fn display_name(&self) -> Option<String> {
        Some("PID".to_string())
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access (controller-specific type codes).
// Registers [`Pid`] for the given DataType code. Panics on duplicate.
pub fn register_pid(dt: DataType) {
    register_type(dt, move || Box::new(Pid::new(dt)));
}

// ---------- CONTROL ----------

// Ref: Rockwell 1756-RM003 - CONTROL structure used by FAL, FSC, COP etc.; layout per
// 1756-PM020 Logix 5000 Data Access (bit positions per Go source).
// 14-byte wire layout, same size as Timer and Counter:
//   Offset 0-1:   Reserved (INT)
//   Offset 2-5:   Status bits (DINT) - EN, EU, DN, EM, ER, UL, IN, FD
//   Offset 6-9:   LEN (DINT) - array length
//   Offset 10-13: POS (DINT) - current position
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Control {
    // Array length
    pub len: i32,
    // Current position
    pub pos: i32,
    // Enable
    pub en: bool,
    // Enable Unload
    pub eu: bool,
    // Done
    pub dn: bool,
    // Empty
    pub em: bool,
    // Error
    pub er: bool,
    // Unload
    pub ul: bool,
    // Inhibit
    pub inhibit: bool,
    // Found
    pub fd: bool,

    dt: DataType,
}

impl Control {
    // Implementation detail: creates a zero-value Control carrying the type code `dt`.
    pub fn new(dt: DataType) -> Control {
        Control { dt, ..Default::default() }
    }
}

// Ref: Rockwell 1756-RM003 CONTROL structure status bits within the 32-bit status DINT (bit
// positions per Go source). EN (Enable), bit 31.
pub const CONTROL_STATUS_EN: u32 = 31;
// Ref: Rockwell 1756-RM003 CONTROL structure - EU (Enable Unload), bit 30.
pub const CONTROL_STATUS_EU: u32 = 30;
// Ref: Rockwell 1756-RM003 CONTROL structure - DN (Done), bit 29.
pub const CONTROL_STATUS_DN: u32 = 29;
// Ref: Rockwell 1756-RM003 CONTROL structure - EM (Empty), bit 28.
pub const CONTROL_STATUS_EM: u32 = 28;
// Ref: Rockwell 1756-RM003 CONTROL structure - ER (Error), bit 27.
pub const CONTROL_STATUS_ER: u32 = 27;
// Ref: Rockwell 1756-RM003 CONTROL structure - UL (Unload), bit 26.
pub const CONTROL_STATUS_UL: u32 = 26;
// Ref: Rockwell 1756-RM003 CONTROL structure - IN (Inhibit), bit 25.
pub const CONTROL_STATUS_IN: u32 = 25;
// Ref: Rockwell 1756-RM003 CONTROL structure - FD (Found), bit 24.
pub const CONTROL_STATUS_FD: u32 = 24;

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - CONTROL 14-byte layout (see Control docs).
// Decodes the 14-byte layout; fewer than 14 bytes yields StatusNotEnoughData.
impl Unmarshaler for Control {
    fn unmarshal_cip(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < 14 {
            return Err(not_enough_data());
        }
        let w = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);

        let status = w(2);
        self.len = w(6) as i32;
        self.pos = w(10) as i32;

        self.en = status & (1 << CONTROL_STATUS_EN) != 0;
        self.eu = status & (1 << CONTROL_STATUS_EU) != 0;
        self.dn = status & (1 << CONTROL_STATUS_DN) != 0;
        self.em = status & (1 << CONTROL_STATUS_EM) != 0;
        self.er = status & (1 << CONTROL_STATUS_ER) != 0;
        self.ul = status & (1 << CONTROL_STATUS_UL) != 0;
        self.inhibit = status & (1 << CONTROL_STATUS_IN) != 0;
        self.fd = status & (1 << CONTROL_STATUS_FD) != 0;
        Ok(())
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - encodes the 14-byte CONTROL layout
// (reserved INT = 0, status DINT, LEN DINT, POS DINT).
impl Marshaler for Control {
    fn marshal_cip(&self) -> Result<Vec<u8>> {
        let mut buf = [0u8; 14];
        // Offset 0-1: Reserved

        let mut status: u32 = 0;
        for (set, bit) in [
            (self.en, CONTROL_STATUS_EN),
            (self.eu, CONTROL_STATUS_EU),
            (self.dn, CONTROL_STATUS_DN),
            (self.em, CONTROL_STATUS_EM),
            (self.er, CONTROL_STATUS_ER),
            (self.ul, CONTROL_STATUS_UL),
            (self.inhibit, CONTROL_STATUS_IN),
            (self.fd, CONTROL_STATUS_FD),
        ] {
            if set {
                status |= 1 << bit;
            }
        }
        buf[2..6].copy_from_slice(&status.to_le_bytes());
        buf[6..10].copy_from_slice(&self.len.to_le_bytes());
        buf[10..14].copy_from_slice(&self.pos.to_le_bytes());
        Ok(buf.to_vec())
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access - structure type code (controller-specific).
impl TypeCodec for Control {
    fn cip_type(&self) -> DataType {
        self.dt
    }
    // Go `String()` returns "CONTROL".
    fn display_name(&self) -> Option<String> {
        Some("CONTROL".to_string())
    }
}

// Ref: Rockwell 1756-PM020 Logix 5000 Data Access (controller-specific type codes).
// Registers [`Control`] for the given DataType code. Panics on duplicate.
pub fn register_control(dt: DataType) {
    register_type(dt, move || Box::new(Control::new(dt)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cip::{go_type_to_cip_type, lookup_type};

    // Rust has no downcast on Box<dyn TypeCodec>, so the codec returned by the registry is
    // exercised through the Marshaler / Unmarshaler / TypeCodec traits and the concrete type
    // is exercised directly.

    #[test]
    fn timer_round_trip() {
        // Register with a fake type code for testing.
        ensure_timer_registered();

        let mut codec = lookup_type(DataType(0x1001)).expect("LookupType(0x1001) returned nil");

        // 14-byte timer payload: reserved(2) + status(4) + PRE(4) + ACC(4)
        // EN=true (bit 31), DN=true (bit 29), PRE=5000, ACC=3000
        let data = [
            0x00, 0x00, // reserved
            0x00, 0x00, 0x00, 0xA0, // status: bits 31+29 set = 0xA0000000
            0x88, 0x13, 0x00, 0x00, // PRE = 5000
            0xB8, 0x0B, 0x00, 0x00, // ACC = 3000
        ];
        codec.unmarshal_cip(&data).unwrap();
        let out = codec.marshal_cip().unwrap();
        assert_eq!(out.len(), 14);
        assert_eq!(out, data.to_vec());
        assert_eq!(codec.cip_type(), DataType(0x1001));
        assert_eq!(codec.display_name().as_deref(), Some("TIMER"));

        let mut tmr = Timer::new(DataType(0x1001));
        tmr.unmarshal_cip(&data).unwrap();
        assert_eq!(tmr.pre, 5000);
        assert_eq!(tmr.acc, 3000);
        assert!(tmr.en);
        assert!(!tmr.tt);
        assert!(tmr.dn);
        assert_eq!(tmr.marshal_cip().unwrap().len(), 14);
        assert_eq!(tmr.cip_type(), DataType(0x1001));
        assert_eq!(tmr.display_name().unwrap(), "TIMER");
    }

    #[test]
    fn counter_round_trip() {
        register_counter(DataType(0x1002));
        let mut codec = lookup_type(DataType(0x1002)).expect("LookupType(0x1002) returned nil");

        // CU=true (bit 31), DN=true (bit 29), PRE=100, ACC=42
        let data = [
            0x00, 0x00, // reserved
            0x00, 0x00, 0x00, 0xA0, // status: bits 31+29 set
            0x64, 0x00, 0x00, 0x00, // PRE = 100
            0x2A, 0x00, 0x00, 0x00, // ACC = 42
        ];
        codec.unmarshal_cip(&data).unwrap();
        assert_eq!(codec.marshal_cip().unwrap(), data.to_vec());
        assert_eq!(codec.display_name().as_deref(), Some("COUNTER"));

        let mut ctr = Counter::new(DataType(0x1002));
        ctr.unmarshal_cip(&data).unwrap();
        assert_eq!(ctr.pre, 100);
        assert_eq!(ctr.acc, 42);
        assert!(ctr.cu);
        assert!(ctr.dn);
    }

    #[test]
    fn control_round_trip() {
        register_control(DataType(0x1003));
        let mut codec = lookup_type(DataType(0x1003)).expect("LookupType(0x1003) returned nil");

        // EN=true (bit 31), DN=true (bit 29), LEN=50, POS=25
        let data = [
            0x00, 0x00, // reserved
            0x00, 0x00, 0x00, 0xA0, // status: bits 31+29 set
            0x32, 0x00, 0x00, 0x00, // LEN = 50
            0x19, 0x00, 0x00, 0x00, // POS = 25
        ];
        codec.unmarshal_cip(&data).unwrap();
        assert_eq!(codec.marshal_cip().unwrap(), data.to_vec());

        let mut ctrl = Control::new(DataType(0x1003));
        ctrl.unmarshal_cip(&data).unwrap();
        assert_eq!(ctrl.len, 50);
        assert_eq!(ctrl.pos, 25);
        assert!(ctrl.en);
        assert!(ctrl.dn);
        assert!(!ctrl.em);

        // Marshal round-trip.
        let out = ctrl.marshal_cip().unwrap();
        assert_eq!(out.len(), 14);

        // Unmarshal the round-tripped bytes into a fresh Control.
        let mut ctrl2 = Control::new(DataType(0x1003));
        ctrl2.unmarshal_cip(&out).unwrap();
        assert!(ctrl2.len == 50 && ctrl2.pos == 25 && ctrl2.en && ctrl2.dn, "round-trip mismatch: {:?}", ctrl2);
    }

    #[test]
    fn data_type_string_resolves_registered() {
        // 0x1001 is registered as Timer by timer_round_trip; register here too if this test
        // runs first (tests run in parallel and the registry is global).
        ensure_timer_registered();
        assert_eq!(DataType(0x1001).to_string(), "TIMER");
        // Array variant.
        assert_eq!(DataType(0x1001 | 0x8000).to_string(), "TIMER[]");
    }

    #[test]
    fn go_type_to_cip_type_with_codec() {
        let tmr = Timer::new(DataType(0x1001));
        assert_eq!(go_type_to_cip_type(&tmr).unwrap(), DataType(0x1001));
    }

    #[test]
    #[should_panic(expected = "duplicate RegisterType")]
    fn duplicate_register_panics() {
        ensure_timer_registered();
        // 0x1001 already registered.
        register_timer(DataType(0x1001));
    }

    // Added: PID codec and short-input errors (Go has no PID test).
    #[test]
    fn pid_and_short_input() {
        let mut pid = Pid::new(DataType(0x1004));
        let mut data = Vec::new();
        for v in [1.5f32, 2.5, 3.5, -1.0, 0.25, 0.5, 0.75] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        // Longer input (full 116-byte struct) is accepted; only the first 28 bytes are used.
        let mut full = data.clone();
        full.resize(116, 0xFF);
        pid.unmarshal_cip(&full).unwrap();
        assert_eq!((pid.sp, pid.pv, pid.out, pid.err, pid.kp, pid.ki, pid.kd), (1.5, 2.5, 3.5, -1.0, 0.25, 0.5, 0.75));
        assert_eq!(pid.marshal_cip().unwrap(), data);
        assert_eq!(pid.display_name().unwrap(), "PID");

        for r in [pid.unmarshal_cip(&data[..27]), Control::new(DataType(1)).unmarshal_cip(&[0u8; 13])] {
            match r.unwrap_err() {
                CipError::Status(e) => assert_eq!(e.status, STATUS_NOT_ENOUGH_DATA),
                other => panic!("unexpected {:?}", other),
            }
        }
    }

    // Registers the 0x1001 timer once across parallel tests (implementation detail of the
    // test suite; the Go tests rely on sequential execution order).
    fn ensure_timer_registered() {
        use std::sync::Once;
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            if lookup_type(DataType(0x1001)).is_none() {
                register_timer(DataType(0x1001));
            }
        });
    }
}
