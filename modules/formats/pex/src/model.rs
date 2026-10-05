use super::{HashMap, PexInstruction, PexTarget, PexValue, PexVersion, PexWriteError, ensure_u16};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PexStringId(u16);

impl PexStringId {
    pub const fn new(index: u16) -> Self { Self(index) }

    pub const fn index(self) -> u16 { self.0 }
}

/// Header metadata written before the PEX string table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PexHeader {
    pub(crate) target: PexTarget,
    pub(crate) pex_version: PexVersion,
    pub(crate) compilation_time: u64,
    pub(crate) source_file_name: String,
    pub(crate) user_name: String,
    pub(crate) computer_name: String,
}

impl PexHeader {
    /// Creates a deterministic Skyrim header when the caller supplies fixed metadata.
    pub fn skyrim(
        compilation_time: u64,
        source_file_name: impl Into<String>,
        user_name: impl Into<String>,
        computer_name: impl Into<String>,
    ) -> Self {
        Self::new(
            PexTarget::Skyrim,
            compilation_time,
            source_file_name,
            user_name,
            computer_name,
        )
    }

    pub fn new(
        target: PexTarget,
        compilation_time: u64,
        source_file_name: impl Into<String>,
        user_name: impl Into<String>,
        computer_name: impl Into<String>,
    ) -> Self {
        Self {
            target,
            pex_version: target.pex_version(),
            compilation_time,
            source_file_name: source_file_name.into(),
            user_name: user_name.into(),
            computer_name: computer_name.into(),
        }
    }

    pub(crate) fn read(
        target: PexTarget,
        pex_version: PexVersion,
        compilation_time: u64,
        source_file_name: String,
        user_name: String,
        computer_name: String,
    ) -> Self {
        Self {
            target,
            pex_version,
            compilation_time,
            source_file_name,
            user_name,
            computer_name,
        }
    }

    pub const fn target(&self) -> PexTarget { self.target }

    pub const fn pex_version(&self) -> PexVersion { self.pex_version }

    pub const fn compilation_time(&self) -> u64 { self.compilation_time }

    pub fn source_file_name(&self) -> &str { &self.source_file_name }

    pub fn user_name(&self) -> &str { &self.user_name }

    pub fn computer_name(&self) -> &str { &self.computer_name }
}

/// User flag table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PexUserFlag {
    pub name: PexStringId,
    pub bit_index: u8,
}

/// Optional PEX debug information table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PexDebugInfo {
    pub modification_time: u64,
    pub functions: Vec<PexDebugFunctionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PexDebugFunctionInfo {
    pub object_name: PexStringId,
    pub state_name: PexStringId,
    pub function_name: PexStringId,
    pub function_type: PexDebugFunctionType,
    pub instruction_line_map: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PexDebugFunctionType {
    Normal = 0,
    Getter = 1,
    Setter = 2,
}

/// In-memory PEX file model for Skyrim-family PEX files.
#[derive(Debug, Clone)]
pub struct PexFile {
    pub(crate) header: PexHeader,
    pub(crate) strings: Vec<String>,
    pub(crate) string_lookup: HashMap<String, PexStringId>,
    pub debug_info: Option<PexDebugInfo>,
    pub user_flags: Vec<PexUserFlag>,
    pub objects: Vec<PexObject>,
}

impl PartialEq for PexFile {
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header
            && self.strings == other.strings
            && self.debug_info == other.debug_info
            && self.user_flags == other.user_flags
            && self.objects == other.objects
    }
}

impl PexFile {
    pub fn new(header: PexHeader) -> Self {
        tracing::trace!(target = %header.target().id(), "creating PEX model");
        Self {
            header,
            strings: Vec::new(),
            string_lookup: HashMap::new(),
            debug_info: None,
            user_flags: Vec::new(),
            objects: Vec::new(),
        }
    }

    pub const fn target(&self) -> PexTarget { self.header.target() }

    pub const fn header(&self) -> &PexHeader { &self.header }

    /// # Errors
    /// Returns an error if the string or the resulting string table exceeds PEX limits.
    pub fn intern(&mut self, text: impl AsRef<str>) -> Result<PexStringId, PexWriteError> {
        let text = text.as_ref();
        if let Some(id) = self.string_lookup.get(text) {
            tracing::trace!(id = id.index(), "reusing PEX string");
            return Ok(*id);
        }

        if self.strings.len() >= u16::MAX as usize {
            return Err(PexWriteError::CountTooLarge {
                what: "string table",
                len: self.strings.len() + 1,
            });
        }
        ensure_u16("string length", text.len())?;

        let id = PexStringId::new(ensure_u16("string table", self.strings.len())?);
        self.strings.push(text.to_owned());
        self.string_lookup.insert(text.to_owned(), id);
        tracing::trace!(id = id.index(), len = text.len(), "interned PEX string");
        Ok(id)
    }

    pub fn string_table(&self) -> &[String] { &self.strings }

    /// Resolve a checked string-table identifier for inspect clients.
    pub fn resolve_string(&self, id: PexStringId) -> Option<&str> {
        self.strings
            .get(usize::from(id.index()))
            .map(String::as_str)
    }
}

impl Default for PexFile {
    fn default() -> Self { Self::new(PexHeader::skyrim(0, "", "", "")) }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PexObject {
    pub name: PexStringId,
    pub parent_class_name: PexStringId,
    pub documentation_string: PexStringId,
    pub user_flags: u32,
    pub auto_state_name: PexStringId,
    pub variables: Vec<PexVariable>,
    pub properties: Vec<PexProperty>,
    pub states: Vec<PexState>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PexVariable {
    pub name: PexStringId,
    pub type_name: PexStringId,
    pub user_flags: u32,
    pub default_value: PexValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PexProperty {
    pub name: PexStringId,
    pub type_name: PexStringId,
    pub documentation_string: PexStringId,
    pub user_flags: u32,
    pub is_readable: bool,
    pub is_writable: bool,
    pub is_auto: bool,
    pub auto_var: Option<PexStringId>,
    pub read_function: Option<PexFunction>,
    pub write_function: Option<PexFunction>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PexState {
    pub name: PexStringId,
    pub functions: Vec<PexFunction>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PexFunction {
    pub name: PexStringId,
    pub return_type_name: PexStringId,
    pub documentation_string: PexStringId,
    pub user_flags: u32,
    pub is_global: bool,
    pub is_native: bool,
    pub parameters: Vec<PexParameter>,
    pub locals: Vec<PexLocal>,
    pub instructions: Vec<PexInstruction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PexParameter {
    pub name: PexStringId,
    pub type_name: PexStringId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PexLocal {
    pub name: PexStringId,
    pub type_name: PexStringId,
}
