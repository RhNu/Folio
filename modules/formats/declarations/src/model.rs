use serde::{Deserialize, Serialize};

/// A complete snapshot of the public API supplied by a declaration source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclarationBundle {
    pub format: String,
    pub schema: u32,
    pub profile: String,
    pub origin: Origin,
    pub scripts: Vec<Script>,
}

/// Generation provenance is descriptive and never grants runtime capabilities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_digest: Option<String>,
}

/// Whole-script API selected atomically by dependency precedence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default)]
    pub is_native: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<String>,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceLocation>,
}

/// Named runtime state and its public callable declarations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub name: String,
    #[serde(default)]
    pub auto: bool,
    #[serde(default)]
    pub members: Vec<Member>,
}

/// Each member kind carries only the facts meaningful for that kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
    #[serde(flatten)]
    pub data: MemberData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum MemberData {
    Function {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        return_type: Option<String>,
        #[serde(default)]
        global: bool,
        #[serde(default)]
        native: bool,
        #[serde(default)]
        parameters: Vec<Parameter>,
    },
    Event {
        #[serde(default)]
        native: bool,
        #[serde(default)]
        parameters: Vec<Parameter>,
    },
    UnknownCallable {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        return_type: Option<String>,
        #[serde(default)]
        global: bool,
        #[serde(default)]
        native: bool,
        #[serde(default)]
        parameters: Vec<Parameter>,
    },
    Property {
        ty: String,
        access: PropertyAccess,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        initial_literal: Option<String>,
    },
    Variable {
        ty: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        initial_literal: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PropertyAccess {
    Auto,
    AutoReadOnly,
    Manual { readable: bool, writable: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemberKind {
    Function,
    Property,
    Event,
    Variable,
    UnknownCallable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    pub ty: String,
    #[serde(default)]
    pub default: ParameterDefault,
}

/// Binary extraction cannot recover a default; unknown must survive persistence.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum ParameterDefault {
    #[default]
    Required,
    Literal(String),
    Unknown,
}

impl ParameterDefault {
    pub fn literal(&self) -> Option<&str> {
        if let Self::Literal(value) = self {
            Some(value)
        } else {
            None
        }
    }
}

/// Historical relative origin, not a navigable path on the consuming machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    pub path: String,
    pub line: u32,
    pub column: u32,
}

impl Member {
    pub fn kind(&self) -> MemberKind {
        match self.data {
            MemberData::Function { .. } => MemberKind::Function,
            MemberData::Event { .. } => MemberKind::Event,
            MemberData::UnknownCallable { .. } => MemberKind::UnknownCallable,
            MemberData::Property { .. } => MemberKind::Property,
            MemberData::Variable { .. } => MemberKind::Variable,
        }
    }
    pub fn ty(&self) -> Option<&str> {
        match &self.data {
            MemberData::Function { return_type, .. }
            | MemberData::UnknownCallable { return_type, .. } => return_type.as_deref(),
            MemberData::Property { ty, .. } | MemberData::Variable { ty, .. } => Some(ty),
            MemberData::Event { .. } => None,
        }
    }
    pub fn parameters(&self) -> &[Parameter] {
        match &self.data {
            MemberData::Function { parameters, .. }
            | MemberData::Event { parameters, .. }
            | MemberData::UnknownCallable { parameters, .. } => parameters,
            _ => &[],
        }
    }
    pub fn is_global(&self) -> bool {
        matches!(
            self.data,
            MemberData::Function { global: true, .. }
                | MemberData::UnknownCallable { global: true, .. }
        )
    }
    pub fn is_native(&self) -> bool {
        matches!(
            self.data,
            MemberData::Function { native: true, .. }
                | MemberData::Event { native: true, .. }
                | MemberData::UnknownCallable { native: true, .. }
        )
    }
    pub fn is_auto(&self) -> bool {
        matches!(
            self.data,
            MemberData::Property {
                access: PropertyAccess::Auto | PropertyAccess::AutoReadOnly,
                ..
            }
        )
    }
    pub fn is_read_only(&self) -> bool {
        matches!(
            self.data,
            MemberData::Property {
                access: PropertyAccess::AutoReadOnly
                    | PropertyAccess::Manual {
                        writable: false,
                        ..
                    },
                ..
            }
        )
    }
    pub fn is_readable(&self) -> bool {
        matches!(
            self.data,
            MemberData::Property {
                access: PropertyAccess::Auto
                    | PropertyAccess::AutoReadOnly
                    | PropertyAccess::Manual { readable: true, .. },
                ..
            }
        )
    }
    pub fn is_writable(&self) -> bool {
        matches!(
            self.data,
            MemberData::Property {
                access: PropertyAccess::Auto | PropertyAccess::Manual { writable: true, .. },
                ..
            }
        )
    }
    pub fn initial_literal(&self) -> Option<&str> {
        match &self.data {
            MemberData::Property {
                initial_literal, ..
            }
            | MemberData::Variable {
                initial_literal, ..
            } => initial_literal.as_deref(),
            _ => None,
        }
    }
}
