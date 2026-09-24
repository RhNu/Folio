//! Typed identifiers for source languages and compilation targets.

/// A source language selected for analysis.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LanguageId(String);

impl LanguageId {
    /// Creates an identifier when its textual name is nonempty.
    pub fn new(name: impl Into<String>) -> Option<Self> {
        let name = name.into();
        (!name.trim().is_empty()).then_some(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A target platform selected for compilation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TargetId(String);

impl TargetId {
    /// Creates an identifier when its textual name is nonempty.
    pub fn new(name: impl Into<String>) -> Option<Self> {
        let name = name.into();
        (!name.trim().is_empty()).then_some(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A VM and ABI contract independent of installed SDK declarations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetProfile {
    pub id: &'static str,
    pub pex_major: u8,
    pub pex_minor: u8,
    pub max_array_length: u32,
    pub max_user_flag_bit: u8,
    pub supports_structs: bool,
    pub supports_guards: bool,
}

impl TargetProfile {
    /// Skyrim Special Edition uses the Skyrim Papyrus ABI and PEX 3.2 layout.
    pub const fn skyrim_se() -> Self {
        Self {
            id: "skyrim-se",
            pex_major: 3,
            pex_minor: 2,
            max_array_length: 128,
            max_user_flag_bit: 31,
            supports_structs: false,
            supports_guards: false,
        }
    }

    /// Resolve only targets with an implemented code generator.
    pub fn implemented(id: &TargetId) -> Option<Self> {
        id.as_str()
            .eq_ignore_ascii_case("skyrim-se")
            .then_some(Self::skyrim_se())
    }
}
