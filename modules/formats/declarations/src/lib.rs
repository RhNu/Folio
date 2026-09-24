//! Versioned, portable external API declarations. Names and types here are
//! unresolved spellings; the analysis layer owns their semantic meaning.

use std::collections::BTreeSet;
use std::io::Read;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 2;
pub const BUILTIN_PACKAGES: &[&str] = &["ck-1.6.1170", "skse-2.2.8"];

/// Decode a compiled-in SDK snapshot selected by its exact package identity.
pub fn builtin(name: &str) -> Result<Option<DeclarationBundle>, String> {
    let bytes: &[u8] = match name {
        "ck-1.6.1170" => include_bytes!("../builtin/ck-1.6.1170.json.gz"),
        "skse-2.2.8" => include_bytes!("../builtin/skse-2.2.8.json.gz"),
        _ => return Ok(None),
    };
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut raw = Vec::new();
    decoder
        .read_to_end(&mut raw)
        .map_err(|cause| format!("decompress built-in {name}: {cause}"))?;
    let bundle = decode(&raw).map_err(|cause| format!("decode built-in {name}: {cause}"))?;
    if bundle.package.name != name {
        return Err(format!(
            "built-in {name} has mismatched package identity {}",
            bundle.package.name
        ));
    }
    Ok(Some(bundle))
}

/// A complete snapshot of the public API supplied by an external package.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclarationBundle {
    pub schema: u32,
    pub package: Package,
    pub compatibility: Compatibility,
    pub naming: Naming,
    pub scripts: Vec<Script>,
}

/// Portable origin metadata, without a machine-specific absolute path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub source: String,
    pub generator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_digest: Option<String>,
}

/// Target and ABI are separate because API availability does not imply VM support.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compatibility {
    pub target: String,
    pub abi: String,
}

/// Rules required to compare exported script names without resolving types.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Naming {
    pub language: String,
    pub case_sensitive: bool,
}

/// A whole-script API selected atomically by dependency precedence.
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

/// A member's type spellings are carried unchanged into semantic analysis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub name: String,
    pub kind: MemberKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
    #[serde(default)]
    pub is_global: bool,
    #[serde(default)]
    pub is_native: bool,
    #[serde(default)]
    pub is_auto: bool,
    #[serde(default)]
    pub is_read_only: bool,
    #[serde(default)]
    pub is_readable: bool,
    #[serde(default)]
    pub is_writable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_literal: Option<String>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    /// Runtime-only marker for PEX signatures, which cannot retain parameter defaults.
    #[serde(skip)]
    pub unknown_defaults: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemberKind {
    Function,
    Property,
    Event,
    Variable,
    /// A PEX callable whose original event/function distinction is unavailable.
    UnknownCallable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    pub ty: String,
    /// Literal spelling, if a default is declared. Analysis interprets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_literal: Option<String>,
}

/// A relative source reference for diagnostics; never interpreted as a local path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    pub path: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug)]
pub enum DecodeError {
    Syntax(serde_json::Error),
    UnsupportedSchema(u32),
    Invalid { field: String, reason: &'static str },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Syntax(error) => write!(f, "invalid declaration JSON: {error}"),
            Self::UnsupportedSchema(version) => {
                write!(f, "unsupported declaration schema {version}")
            }
            Self::Invalid { field, reason } => write!(f, "invalid {field}: {reason}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decode and validate a declaration snapshot in memory.
pub fn decode(input: &[u8]) -> Result<DeclarationBundle, DecodeError> {
    let bundle: DeclarationBundle = serde_json::from_slice(input).map_err(DecodeError::Syntax)?;
    validate(&bundle)?;
    Ok(bundle)
}

/// Encode a validated snapshot using the current carrier version.
pub fn encode(bundle: &DeclarationBundle) -> Result<Vec<u8>, DecodeError> {
    validate(bundle)?;
    serde_json::to_vec_pretty(bundle).map_err(DecodeError::Syntax)
}

/// Check portable carrier invariants, leaving semantic type resolution to analysis.
pub fn validate(bundle: &DeclarationBundle) -> Result<(), DecodeError> {
    if bundle.schema != 1 && bundle.schema != SCHEMA_VERSION {
        return Err(DecodeError::UnsupportedSchema(bundle.schema));
    }
    for (field, value) in [
        ("package.name", &bundle.package.name),
        ("package.version", &bundle.package.version),
        ("package.source", &bundle.package.source),
        ("package.generator", &bundle.package.generator),
        ("compatibility.target", &bundle.compatibility.target),
        ("compatibility.abi", &bundle.compatibility.abi),
        ("naming.language", &bundle.naming.language),
    ] {
        nonempty(field, value)?;
    }
    // This label describes provenance; physical local paths belong to the loader.
    if bundle.package.source.contains(['/', '\\', ':'])
        || bundle.package.source == "."
        || bundle.package.source == ".."
    {
        return Err(DecodeError::Invalid {
            field: "package.source".into(),
            reason: "must be a portable provenance label",
        });
    }
    if let Some(digest) = &bundle.package.source_digest {
        nonempty("package.source_digest", digest)?;
    }
    let mut names = BTreeSet::new();
    for (index, script) in bundle.scripts.iter().enumerate() {
        nonempty(&format!("scripts[{index}].name"), &script.name)?;
        if bundle.schema == 1
            && (!script.flags.is_empty()
                || !script.imports.is_empty()
                || !script.states.is_empty()
                || script.members.iter().any(|member| {
                    member.kind == MemberKind::Variable
                        || member.is_read_only
                        || member.is_readable
                        || member.is_writable
                        || !member.flags.is_empty()
                        || member.initial_literal.is_some()
                }))
        {
            return Err(DecodeError::Invalid {
                field: format!("scripts[{index}]"),
                reason: "schema 2 facts require schema 2",
            });
        }
        if let Some(parent) = &script.parent {
            nonempty(&format!("scripts[{index}].parent"), parent)?;
        }
        for (flag_index, flag) in script.flags.iter().enumerate() {
            nonempty(&format!("scripts[{index}].flags[{flag_index}]"), flag)?;
        }
        for (import_index, import) in script.imports.iter().enumerate() {
            nonempty(&format!("scripts[{index}].imports[{import_index}]"), import)?;
        }
        let key = if bundle.naming.case_sensitive {
            script.name.clone()
        } else {
            script.name.to_lowercase()
        };
        if !names.insert(key) {
            return Err(DecodeError::Invalid {
                field: format!("scripts[{index}].name"),
                reason: "duplicate script identity",
            });
        }
        if let Some(source) = &script.source {
            nonempty(&format!("scripts[{index}].source.path"), &source.path)?;
            let path = std::path::Path::new(&source.path);
            if path.is_absolute()
                || source.path.starts_with('/')
                || source.path.starts_with('\\')
                || source.path.get(1..2) == Some(":")
                || path
                    .components()
                    .any(|component| component == std::path::Component::ParentDir)
            {
                return Err(DecodeError::Invalid {
                    field: format!("scripts[{index}].source.path"),
                    reason: "must stay relative to the declaration source",
                });
            }
            if source.line == 0 || source.column == 0 {
                return Err(DecodeError::Invalid {
                    field: format!("scripts[{index}].source"),
                    reason: "line and column are one-based",
                });
            }
        }
        validate_members(index, "members", &script.members)?;
        let mut states = BTreeSet::new();
        for (state_index, state) in script.states.iter().enumerate() {
            nonempty(
                &format!("scripts[{index}].states[{state_index}].name"),
                &state.name,
            )?;
            if !states.insert(state.name.to_ascii_lowercase()) {
                return Err(DecodeError::Invalid {
                    field: format!("scripts[{index}].states[{state_index}].name"),
                    reason: "duplicate state identity",
                });
            }
            validate_members(
                index,
                &format!("states[{state_index}].members"),
                &state.members,
            )?;
        }
    }
    Ok(())
}

fn validate_members(
    script_index: usize,
    context: &str,
    members: &[Member],
) -> Result<(), DecodeError> {
    let mut names = BTreeSet::new();
    for (member_index, member) in members.iter().enumerate() {
        let index = script_index;
        nonempty(
            &format!("scripts[{index}].{context}[{member_index}].name"),
            &member.name,
        )?;
        let field = format!("scripts[{index}].{context}[{member_index}]");
        if !names.insert((member.name.to_ascii_lowercase(), member.kind)) {
            return Err(DecodeError::Invalid {
                field,
                reason: "duplicate member identity",
            });
        }
        if member.kind == MemberKind::UnknownCallable && !member.unknown_defaults {
            return Err(DecodeError::Invalid {
                field,
                reason: "unknown callable kind is reserved for in-memory PEX extraction",
            });
        }
        if member.is_global
            && !matches!(
                member.kind,
                MemberKind::Function | MemberKind::UnknownCallable
            )
        {
            return Err(DecodeError::Invalid {
                field,
                reason: "only a function can be global",
            });
        }
        if member.is_auto && member.kind != MemberKind::Property {
            return Err(DecodeError::Invalid {
                field,
                reason: "only a property can be auto",
            });
        }
        if member.is_read_only && member.kind != MemberKind::Property {
            return Err(DecodeError::Invalid {
                field,
                reason: "only a property can be read-only",
            });
        }
        if !member.parameters.is_empty() && member.kind == MemberKind::Property {
            return Err(DecodeError::Invalid {
                field,
                reason: "a property has no parameters",
            });
        }
        if let Some(ty) = &member.ty {
            nonempty(
                &format!("scripts[{index}].{context}[{member_index}].ty"),
                ty,
            )?;
        }
        for (parameter_index, parameter) in member.parameters.iter().enumerate() {
            nonempty(
                &format!(
                    "scripts[{index}].{context}[{member_index}].parameters[{parameter_index}].name"
                ),
                &parameter.name,
            )?;
            nonempty(
                &format!(
                    "scripts[{index}].{context}[{member_index}].parameters[{parameter_index}].ty"
                ),
                &parameter.ty,
            )?;
            if let Some(default) = &parameter.default_literal {
                nonempty(
                    &format!(
                        "scripts[{index}].{context}[{member_index}].parameters[{parameter_index}].default_literal"
                    ),
                    default,
                )?;
            }
        }
    }
    Ok(())
}

fn nonempty(field: &str, value: &str) -> Result<(), DecodeError> {
    if value.trim().is_empty() {
        Err(DecodeError::Invalid {
            field: field.to_owned(),
            reason: "must not be empty",
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_fields_and_duplicate_script_identities() {
        let bytes = br#"{"schema":1,"package":{"name":"sdk","version":"1","source":"local","generator":"fixture"},"compatibility":{"target":"skyrim-se","abi":"papyrus"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Actor"},{"name":"actor"}]}"#;
        assert!(matches!(decode(bytes), Err(DecodeError::Invalid { .. })));
        let unknown = br#"{"schema":1,"package":{"name":"sdk","version":"1","source":"local","generator":"fixture","secret":1},"compatibility":{"target":"skyrim-se","abi":"papyrus"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[]}"#;
        assert!(matches!(decode(unknown), Err(DecodeError::Syntax(_))));
    }

    #[test]
    fn rejects_unsupported_schema() {
        let bytes = br#"{"schema":3,"package":{"name":"sdk","version":"1","source":"local","generator":"fixture"},"compatibility":{"target":"skyrim-se","abi":"papyrus"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[]}"#;
        assert!(matches!(
            decode(bytes),
            Err(DecodeError::UnsupportedSchema(3))
        ));
    }

    #[test]
    fn preserves_public_signature_and_relative_origin() {
        let bytes = br#"{"schema":1,"package":{"name":"ck","version":"1","source":"self-authored","generator":"fixture"},"compatibility":{"target":"skyrim-se","abi":"papyrus-skyrim"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Actor","parent":"Form","is_native":true,"source":{"path":"api/Actor.psc","line":2,"column":1},"members":[{"name":"GetName","kind":"function","ty":"String","is_global":false,"is_native":true,"parameters":[{"name":"prefix","ty":"String","default_literal":"\"\""}]}]}]}"#;
        let bundle = decode(bytes).unwrap();
        assert_eq!(bundle.scripts[0].parent.as_deref(), Some("Form"));
        assert!(bundle.scripts[0].members[0].is_native);
        assert_eq!(
            bundle.scripts[0].members[0].parameters[0]
                .default_literal
                .as_deref(),
            Some("\"\"")
        );
        assert!(decode(&encode(&bundle).unwrap()).is_ok());
    }

    #[test]
    fn rejects_invalid_member_flags_and_absolute_source_location() {
        let bytes = br#"{"schema":1,"package":{"name":"ck","version":"1","source":"local","generator":"fixture"},"compatibility":{"target":"skyrim-se","abi":"papyrus-skyrim"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Actor","members":[{"name":"OnInit","kind":"event","is_global":true}]}]}"#;
        assert!(matches!(decode(bytes), Err(DecodeError::Invalid { .. })));
        let bytes = br#"{"schema":1,"package":{"name":"ck","version":"1","source":"local","generator":"fixture"},"compatibility":{"target":"skyrim-se","abi":"papyrus-skyrim"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Actor","source":{"path":"C:/private/Actor.psc","line":1,"column":1}}]}"#;
        assert!(matches!(decode(bytes), Err(DecodeError::Invalid { .. })));
    }

    #[test]
    fn builtins_decode_with_expected_versions_and_schema() {
        let ck = builtin("ck-1.6.1170").unwrap().unwrap();
        let skse = builtin("skse-2.2.8").unwrap().unwrap();
        assert_eq!(ck.schema, SCHEMA_VERSION);
        assert_eq!(ck.package.version, "1.6.1170");
        assert_eq!(skse.package.version, "2.2.8");
        assert!(ck.scripts.iter().any(|script| script.name == "Actor"));
        assert!(skse.scripts.iter().any(|script| script.name == "Actor"));
        assert!(builtin("missing").unwrap().is_none());
    }
}
