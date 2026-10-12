use std::fmt;

// Ref: CIP Vol. 2 (EtherNet/IP Adaptation of CIP), Chapter 2 "Encapsulation", encapsulation
// command field of the header (command codes table; table number unverified).
/// An EtherNet/IP encapsulation command code. Unknown codes are representable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Command(pub u16);

// Ref: CIP Vol. 2, Chapter 2 encapsulation command codes (table number unverified).
impl Command {
    // Ref: CIP Vol. 2, Ch. 2 "NOP" command (0x0000).
    pub const NOP: Command = Command(0x0000);
    // Ref: CIP Vol. 2, Ch. 2 "ListServices" command (0x0004).
    pub const LIST_SERVICES: Command = Command(0x0004);
    // Ref: CIP Vol. 2, Ch. 2 "ListIdentity" command (0x0063).
    pub const LIST_IDENTITY: Command = Command(0x0063);
    // Ref: CIP Vol. 2, Ch. 2 "ListInterfaces" command (0x0064).
    pub const LIST_INTERFACES: Command = Command(0x0064);
    // Ref: CIP Vol. 2, Ch. 2 "RegisterSession" command (0x0065).
    pub const REGISTER_SESSION: Command = Command(0x0065);
    // Ref: CIP Vol. 2, Ch. 2 "UnRegisterSession" command (0x0066).
    pub const UNREGISTER_SESSION: Command = Command(0x0066);
    // Ref: CIP Vol. 2, Ch. 2 "SendRRData" command (0x006F).
    pub const SEND_RR_DATA: Command = Command(0x006F);
    // Ref: CIP Vol. 2, Ch. 2 "SendUnitData" command (0x0070).
    pub const SEND_UNIT_DATA: Command = Command(0x0070);
    // Ref: CIP Vol. 2, Ch. 2 "IndicateStatus" command (0x0072) (section unverified).
    pub const INDICATE_STATUS: Command = Command(0x0072);
    // Ref: CIP Vol. 2, Ch. 2 "Cancel" command (0x0073) (section unverified).
    pub const CANCEL: Command = Command(0x0073);
}

// Go-style aliases (CommandNop etc.) for readers porting from goindustrial.
// Ref: CIP Vol. 2, Ch. 2 command codes (as above).
pub const COMMAND_NOP: Command = Command::NOP;
pub const COMMAND_LIST_SERVICES: Command = Command::LIST_SERVICES;
pub const COMMAND_LIST_IDENTITY: Command = Command::LIST_IDENTITY;
pub const COMMAND_LIST_INTERFACES: Command = Command::LIST_INTERFACES;
pub const COMMAND_REGISTER_SESSION: Command = Command::REGISTER_SESSION;
pub const COMMAND_UNREGISTER_SESSION: Command = Command::UNREGISTER_SESSION;
pub const COMMAND_SEND_RR_DATA: Command = Command::SEND_RR_DATA;
pub const COMMAND_SEND_UNIT_DATA: Command = Command::SEND_UNIT_DATA;
pub const COMMAND_INDICATE_STATUS: Command = Command::INDICATE_STATUS;
pub const COMMAND_CANCEL: Command = Command::CANCEL;

// Implementation detail (not spec): human-readable command names, same text as the Go String().
impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            Command::NOP => "Nop",
            Command::LIST_SERVICES => "ListServices",
            Command::LIST_IDENTITY => "ListIdentity",
            Command::LIST_INTERFACES => "ListInterfaces",
            Command::REGISTER_SESSION => "RegisterSession",
            Command::UNREGISTER_SESSION => "UnregisterSession",
            Command::SEND_RR_DATA => "SendRRData",
            Command::SEND_UNIT_DATA => "SendUnitData",
            Command::INDICATE_STATUS => "IndicateStatus",
            Command::CANCEL => "Cancel",
            Command(c) => return write!(f, "UnknownCommand(0x{c:04X})"),
        };
        f.write_str(s)
    }
}

// Ref: CIP Vol. 2, Ch. 2 encapsulation status (error) codes returned in the header Status
// field (table number unverified).
// Ref: CIP Vol. 2, Ch. 2: 0x0000 success.
pub const STATUS_SUCCESS: u32 = 0x0000_0000;
// Ref: CIP Vol. 2, Ch. 2: 0x0001 invalid or unsupported encapsulation command.
pub const STATUS_INVALID_COMMAND: u32 = 0x0000_0001;
// Ref: CIP Vol. 2, Ch. 2: 0x0002 insufficient memory in receiver.
pub const STATUS_INSUFFICIENT_MEMORY: u32 = 0x0000_0002;
// Ref: CIP Vol. 2, Ch. 2: 0x0003 poorly formed or incorrect data in the data portion.
pub const STATUS_INCORRECT_DATA: u32 = 0x0000_0003;
// Ref: CIP Vol. 2, Ch. 2: 0x0064 invalid session handle.
pub const STATUS_INVALID_SESSION_HANDLE: u32 = 0x0000_0064;
// Ref: CIP Vol. 2, Ch. 2: 0x0065 invalid length in header.
pub const STATUS_INVALID_LENGTH: u32 = 0x0000_0065;
// Ref: CIP Vol. 2, Ch. 2: 0x0069 unsupported encapsulation protocol revision.
pub const STATUS_UNSUPPORTED_PROTOCOL: u32 = 0x0000_0069;

// Ref: CIP Vol. 2, Ch. 2 "RegisterSession" command-specific data: Protocol Version (UINT) +
// Options Flags (UINT) (section unverified).
/// Payload of a RegisterSession command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterSessionData {
    pub protocol_version: u16,
    pub options_flags: u16,
}

impl Default for RegisterSessionData {
    // Ref: CIP Vol. 2, Ch. 2 RegisterSession: protocol version 1, options 0.
    fn default() -> Self {
        Self { protocol_version: 1, options_flags: 0 }
    }
}

impl RegisterSessionData {
    /// Protocol version 1, no option flags (Go NewRegisterSessionData).
    pub fn new() -> Self {
        Self::default()
    }

    // Ref: CIP Vol. 2, Ch. 2 RegisterSession data layout, little-endian (4 bytes).
    /// Serializes to the 4-byte wire format.
    pub fn encode(&self) -> super::Result<Vec<u8>> {
        let mut v = Vec::with_capacity(4);
        v.extend_from_slice(&self.protocol_version.to_le_bytes());
        v.extend_from_slice(&self.options_flags.to_le_bytes());
        Ok(v)
    }
}
