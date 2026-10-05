use super::{Error, PexDebugFunctionType, PexOpcode, PexStringId, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PexWriteError {
    UnsupportedTarget {
        target: &'static str,
    },
    UnsupportedVersion {
        major: u8,
        minor: u8,
    },
    CountTooLarge {
        what: &'static str,
        len: usize,
    },
    StringTooLong {
        what: &'static str,
        len: usize,
    },
    StringIdOutOfRange {
        id: PexStringId,
        table_len: usize,
    },
    ObjectTooLarge {
        len: usize,
    },
    InvalidInstructionArity {
        opcode: PexOpcode,
        expected: usize,
        actual: usize,
    },
    UnexpectedVariadicArguments {
        opcode: PexOpcode,
    },
    VariadicArgumentCountTooLarge {
        len: usize,
    },
    InvalidUserFlagBit {
        bit_index: u8,
    },
    InvalidPropertyModel {
        property: PexStringId,
        reason: &'static str,
    },
    InvalidAccessorSignature {
        property: PexStringId,
        accessor: &'static str,
        reason: &'static str,
    },
    InvalidDebugFunctionReference {
        object_name: PexStringId,
        state_name: PexStringId,
        function_name: PexStringId,
        function_type: PexDebugFunctionType,
    },
    DebugLineMapLengthMismatch {
        function_name: PexStringId,
        expected: usize,
        actual: usize,
    },
    AutoPropertyMissingAutoVar,
    ReadablePropertyMissingGetter,
    WritablePropertyMissingSetter,
}

impl fmt::Display for PexWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTarget { target } => write!(f, "unsupported PEX target {target}"),
            Self::UnsupportedVersion { major, minor } => {
                write!(f, "unsupported Skyrim PEX version {major}.{minor}")
            },
            Self::CountTooLarge { what, len } => {
                write!(f, "{what} count {len} exceeds u16::MAX")
            },
            Self::StringTooLong { what, len } => {
                write!(f, "{what} length {len} exceeds u16::MAX")
            },
            Self::StringIdOutOfRange { id, table_len } => write!(
                f,
                "string id {} is outside string table length {table_len}",
                id.index()
            ),
            Self::ObjectTooLarge { len } => {
                write!(f, "object body length {len} exceeds u32::MAX")
            },
            Self::InvalidInstructionArity {
                opcode,
                expected,
                actual,
            } => write!(
                f,
                "opcode {opcode:?} expects {expected} arguments but received {actual}"
            ),
            Self::UnexpectedVariadicArguments { opcode } => {
                write!(f, "opcode {opcode:?} does not accept variadic arguments")
            },
            Self::VariadicArgumentCountTooLarge { len } => {
                write!(f, "variadic argument count {len} exceeds i32::MAX")
            },
            Self::InvalidUserFlagBit { bit_index } => {
                write!(f, "user flag bit {bit_index} is outside a 32-bit mask")
            },
            Self::InvalidPropertyModel { property, reason } => write!(
                f,
                "property {} has an invalid PEX model: {reason}",
                property.index()
            ),
            Self::InvalidAccessorSignature {
                property,
                accessor,
                reason,
            } => write!(
                f,
                "property {} {accessor} has an invalid signature: {reason}",
                property.index()
            ),
            Self::InvalidDebugFunctionReference {
                object_name,
                state_name,
                function_name,
                function_type,
            } => write!(
                f,
                "debug function reference {function_type:?} object={} state={} function={} does not resolve",
                object_name.index(),
                state_name.index(),
                function_name.index()
            ),
            Self::DebugLineMapLengthMismatch {
                function_name,
                expected,
                actual,
            } => write!(
                f,
                "debug line map for function {} has {actual} entries but function has {expected} instructions",
                function_name.index()
            ),
            Self::AutoPropertyMissingAutoVar => {
                f.write_str("auto property has no backing variable")
            },
            Self::ReadablePropertyMissingGetter => {
                f.write_str("property is readable but has no getter function")
            },
            Self::WritablePropertyMissingSetter => {
                f.write_str("property is writable but has no setter function")
            },
        }
    }
}

impl Error for PexWriteError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PexReadError {
    InvalidMagic {
        value: u32,
    },
    UnsupportedVersion {
        major: u8,
        minor: u8,
    },
    UnsupportedGame {
        game_id: u16,
    },
    Truncated {
        offset: usize,
        needed: usize,
        remaining: usize,
        what: &'static str,
    },
    InvalidUtf8 {
        offset: usize,
        what: &'static str,
    },
    StringIdOutOfRange {
        id: PexStringId,
        table_len: usize,
        what: &'static str,
    },
    UnknownOpcode {
        offset: usize,
        opcode: u8,
    },
    UnknownValueType {
        offset: usize,
        tag: u8,
    },
    InvalidDebugFunctionType {
        offset: usize,
        tag: u8,
    },
    InvalidField {
        offset: usize,
        what: &'static str,
        value: u8,
    },
    InvalidStructure {
        reason: String,
    },
    MalformedVariadicCount {
        offset: usize,
        opcode: PexOpcode,
    },
    ObjectSizeMismatch {
        offset: usize,
        expected_end: usize,
        actual_end: usize,
    },
    TrailingBytes {
        offset: usize,
        len: usize,
    },
}

impl fmt::Display for PexReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic { value } => write!(f, "invalid PEX magic 0x{value:08x}"),
            Self::UnsupportedVersion { major, minor } => {
                write!(f, "unsupported Skyrim PEX version {major}.{minor}")
            },
            Self::UnsupportedGame { game_id } => {
                write!(f, "unsupported PEX game id {game_id}")
            },
            Self::Truncated {
                offset,
                needed,
                remaining,
                what,
            } => write!(
                f,
                "truncated PEX while reading {what} at offset {offset}: needed {needed} bytes, remaining {remaining}"
            ),
            Self::InvalidUtf8 { offset, what } => {
                write!(f, "invalid UTF-8 in {what} at offset {offset}")
            },
            Self::StringIdOutOfRange {
                id,
                table_len,
                what,
            } => write!(
                f,
                "{what} string id {} is outside string table length {table_len}",
                id.index()
            ),
            Self::UnknownOpcode { offset, opcode } => {
                write!(f, "unknown PEX opcode {opcode} at offset {offset}")
            },
            Self::UnknownValueType { offset, tag } => {
                write!(f, "unknown PEX value type {tag} at offset {offset}")
            },
            Self::InvalidDebugFunctionType { offset, tag } => {
                write!(f, "invalid debug function type {tag} at offset {offset}")
            },
            Self::InvalidField {
                offset,
                what,
                value,
            } => {
                write!(f, "invalid {what} value {value} at offset {offset}")
            },
            Self::InvalidStructure { reason } => write!(f, "invalid PEX structure: {reason}"),
            Self::MalformedVariadicCount { offset, opcode } => write!(
                f,
                "malformed variadic count for opcode {opcode:?} at offset {offset}"
            ),
            Self::ObjectSizeMismatch {
                offset,
                expected_end,
                actual_end,
            } => write!(
                f,
                "object body size mismatch at offset {offset}: expected end {expected_end}, actual end {actual_end}"
            ),
            Self::TrailingBytes { offset, len } => {
                write!(
                    f,
                    "trailing bytes after PEX body at offset {offset} of {len}"
                )
            },
        }
    }
}

impl Error for PexReadError {}
