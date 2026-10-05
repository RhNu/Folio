//! Independent Papyrus PEX format model and in-memory codec.
//!
//! The current layout implementation is for Skyrim PEX 3.1/3.2. PEX from
//! later games has additional structures and opcodes, so its game ID is
//! rejected explicitly rather than decoded using Skyrim's layout.

use std::{collections::HashMap, error::Error, fmt};

const PEX_MAGIC: u32 = 0xFA57_C0DE;

/// Skyrim PEX game identifier in the binary header.
pub const SKYRIM_GAME_ID: u16 = 1;
/// Version emitted for Skyrim PEX files.
pub const SKYRIM_WRITE_VERSION: PexVersion = PexVersion::new(3, 2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PexVersion {
    major: u8,
    minor: u8,
}

impl PexVersion {
    pub const fn new(major: u8, minor: u8) -> Self { Self { major, minor } }

    pub const fn major(self) -> u8 { self.major }

    pub const fn minor(self) -> u8 { self.minor }
}

/// PEX binary layout family; this does not describe language or SDK support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PexTarget {
    Skyrim,
}

impl PexTarget {
    pub const fn skyrim() -> Self { Self::Skyrim }

    pub const fn skyrim_se() -> Self { Self::Skyrim }

    pub const fn id(self) -> &'static str { "skyrim" }

    pub const fn display_name(self) -> &'static str { "Skyrim" }

    pub const fn pex_game_id(self) -> u16 { SKYRIM_GAME_ID }

    pub const fn pex_version(self) -> PexVersion { SKYRIM_WRITE_VERSION }

    pub const fn endianness(self) -> Endianness { Endianness::Big }

    pub const fn supports_pex_version(self, version: PexVersion) -> bool {
        version.major == 3 && (version.minor == 1 || version.minor == 2)
    }

    pub const fn from_pex_game_id(id: u16) -> Option<Self> {
        if id == SKYRIM_GAME_ID {
            Some(Self::Skyrim)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endianness {
    Little,
    Big,
}

mod binary;
mod dump;
mod errors;
mod model;
mod opcode;
mod reader;
mod validation;
mod writer;

pub(crate) use binary::{BinaryReader, BinaryWriter};
pub use errors::{PexReadError, PexWriteError};
pub use model::{
    PexDebugFunctionInfo, PexDebugFunctionType, PexDebugInfo, PexFile, PexFunction, PexHeader,
    PexLocal, PexObject, PexParameter, PexProperty, PexState, PexStringId, PexUserFlag,
    PexVariable,
};
pub use opcode::{PexInstruction, PexOpcode, PexValue};
pub(crate) use validation::{ensure_u16, escape_dump_text, string_id_eq, validate_for_write};
pub(crate) use writer::validate_property_model;
