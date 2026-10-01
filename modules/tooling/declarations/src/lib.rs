//! Deterministic declaration extraction from supplied Papyrus source text.

use std::collections::{BTreeMap, BTreeSet};

use folio_format_declarations::{
    DeclarationBundle, FORMAT, Member, MemberData, MemberKind, Origin, PROFILE, Parameter,
    ParameterDefault, PropertyAccess, SCHEMA_VERSION, Script, SourceLocation, State, validate,
};
use folio_papyrus::{Declaration, PapyrusDialect, SyntaxKind, SyntaxNode, declarations, parse};

mod pex;
pub use pex::{ExtractedPex, PexInput, extract_pex};

/// A source snapshot whose path is relative to the supplied source root.
#[derive(Clone, Copy)]
pub struct SourceInput<'a> {
    pub path: &'a str,
    pub text: &'a str,
}

/// Reproducible provenance label; output location belongs to the caller.
pub struct GenerationOptions<'a> {
    pub source: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationError {
    pub path: String,
    pub reason: String,
}

impl std::fmt::Display for GenerationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.reason)
    }
}

impl std::error::Error for GenerationError {}

/// Produce a portable snapshot without reading the filesystem or resolving types.
pub fn generate(
    options: GenerationOptions<'_>,
    sources: &[SourceInput<'_>],
) -> Result<DeclarationBundle, GenerationError> {
    let mut sorted = sources.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| a.path.cmp(b.path));
    let mut digest = blake3::Hasher::new();
    let mut scripts = Vec::with_capacity(sorted.len());
    let mut paths = BTreeSet::new();
    for input in sorted {
        if input.path.is_empty()
            || input.path.starts_with('/')
            || input.path.starts_with('\\')
            || input.path.contains('\\')
            || input
                .path
                .split('/')
                .any(|part| part.is_empty() || part == ".." || part == ".")
        {
            return Err(error(
                input.path,
                "source path must be a portable relative path",
            ));
        }
        if !paths.insert(input.path.to_ascii_lowercase()) {
            return Err(error(input.path, "duplicate source path"));
        }
        digest.update(&(input.path.len() as u64).to_le_bytes());
        digest.update(input.path.as_bytes());
        digest.update(&(input.text.len() as u64).to_le_bytes());
        digest.update(input.text.as_bytes());
        scripts.push(extract(input)?);
    }
    scripts.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    let bundle = DeclarationBundle {
        format: FORMAT.into(),
        schema: SCHEMA_VERSION,
        profile: PROFILE.into(),
        origin: Origin {
            source: options.source.to_owned(),
            input_digest: Some(digest.finalize().to_hex().to_string()),
        },
        scripts,
    };
    validate(&bundle).map_err(|cause| error("<declarations>", cause.to_string()))?;
    tracing::info!(source = %bundle.origin.source, scripts = bundle.scripts.len(), "generated declarations");
    Ok(bundle)
}

fn error(path: &str, reason: impl Into<String>) -> GenerationError {
    GenerationError {
        path: path.to_owned(),
        reason: reason.into(),
    }
}

fn extract(input: &SourceInput<'_>) -> Result<Script, GenerationError> {
    let parsed = parse(input.text, PapyrusDialect::Skyrim);
    let summaries = declarations(&parsed)
        .into_iter()
        .map(|item| (item.range.start, item.declaration))
        .collect::<BTreeMap<_, _>>();
    let root = parsed.syntax();
    let mut script = None;
    let mut imports = Vec::new();
    let mut members = Vec::new();
    let mut states = Vec::new();
    for node in root.children() {
        match node.kind() {
            SyntaxKind::ScriptDecl => {
                if script.is_some() {
                    return Err(error(input.path, "multiple ScriptName declarations"));
                }
                let Declaration::Script {
                    name,
                    parent,
                    flags,
                } = declaration(&summaries, &node, input.path)?
                else {
                    return Err(error(input.path, "invalid ScriptName declaration"));
                };
                let basename = input
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(input.path)
                    .rsplit_once('.')
                    .filter(|(_, extension)| extension.eq_ignore_ascii_case("psc"))
                    .map(|(basename, _)| basename)
                    .ok_or_else(|| error(input.path, "expected .psc source file"))?;
                if !name.eq_ignore_ascii_case(basename) {
                    return Err(error(
                        input.path,
                        format!("ScriptName {name} differs from file name {basename}"),
                    ));
                }
                let (line, column) =
                    line_column(input.text, usize::from(node.text_range().start()));
                script = Some((
                    name.clone(),
                    parent.clone(),
                    flags.clone(),
                    SourceLocation {
                        path: input.path.to_owned(),
                        line,
                        column,
                    },
                ));
            }
            SyntaxKind::ImportDecl => {
                if let Declaration::Import { name } = declaration(&summaries, &node, input.path)? {
                    imports.push(name.clone());
                }
            }
            SyntaxKind::FunctionDecl
            | SyntaxKind::EventDecl
            | SyntaxKind::PropertyDecl
            | SyntaxKind::VariableDecl => {
                members.push(member(&summaries, &node, input.path)?);
            }
            SyntaxKind::StateDecl => {
                let Declaration::State { name, flags } =
                    declaration(&summaries, &node, input.path)?
                else {
                    return Err(error(input.path, "invalid State declaration"));
                };
                let mut state_members = Vec::new();
                for child in node
                    .children()
                    .filter(|child| child.kind() == SyntaxKind::Block)
                    .flat_map(|block| block.children())
                {
                    match child.kind() {
                        SyntaxKind::FunctionDecl | SyntaxKind::EventDecl => {
                            state_members.push(member(&summaries, &child, input.path)?)
                        }
                        SyntaxKind::Error => {
                            return Err(error(
                                input.path,
                                format!("unsupported declaration in state {name}"),
                            ));
                        }
                        _ => {}
                    }
                }
                if let Some(existing) = states
                    .iter_mut()
                    .find(|state: &&mut State| state.name.eq_ignore_ascii_case(name))
                {
                    existing.auto |= flags.iter().any(|flag| flag == "auto");
                    existing.members.extend(state_members);
                } else {
                    states.push(State {
                        name: name.clone(),
                        auto: flags.iter().any(|flag| flag == "auto"),
                        members: state_members,
                    });
                }
            }
            SyntaxKind::Error => {
                return Err(error(input.path, "unsupported top-level declaration"));
            }
            _ => {}
        }
    }
    let (name, parent, flags, source) =
        script.ok_or_else(|| error(input.path, "missing ScriptName declaration"))?;
    let mut member_names = BTreeSet::new();
    for member in &members {
        if !member_names.insert((member.name.to_ascii_lowercase(), member.kind())) {
            return Err(error(
                input.path,
                format!("duplicate root member {}", member.name),
            ));
        }
    }
    for state in &states {
        let mut state_names = BTreeSet::new();
        for member in &state.members {
            if !state_names.insert((member.name.to_ascii_lowercase(), member.kind())) {
                return Err(error(
                    input.path,
                    format!("duplicate member {} in state {}", member.name, state.name),
                ));
            }
        }
    }
    tracing::debug!(
        path = input.path,
        members = members.len(),
        states = states.len(),
        parse_errors = parsed.errors.len(),
        "extracted script declarations"
    );
    Ok(Script {
        name,
        parent,
        is_native: false,
        flags,
        imports,
        members,
        states,
        source: Some(source),
    })
}

fn declaration<'a>(
    summaries: &'a BTreeMap<usize, Declaration>,
    node: &SyntaxNode,
    path: &str,
) -> Result<&'a Declaration, GenerationError> {
    summaries
        .get(&usize::from(node.text_range().start()))
        .ok_or_else(|| error(path, format!("incomplete {:?} signature", node.kind())))
}

fn member(
    summaries: &BTreeMap<usize, Declaration>,
    node: &SyntaxNode,
    path: &str,
) -> Result<Member, GenerationError> {
    let signature = declaration(summaries, node, path)?;
    let (name, kind, ty, flags, parameters) = match signature {
        Declaration::Function {
            name,
            return_type,
            parameters,
            modifiers,
        } => (
            name.clone(),
            MemberKind::Function,
            return_type.clone(),
            modifiers.clone(),
            parameters.iter().map(parameter).collect(),
        ),
        Declaration::Event {
            name,
            parameters,
            modifiers,
        } => (
            name.clone(),
            MemberKind::Event,
            None,
            modifiers.clone(),
            parameters.iter().map(parameter).collect(),
        ),
        Declaration::Property { name, ty, flags } => (
            name.clone(),
            MemberKind::Property,
            Some(ty.clone()),
            flags.clone(),
            Vec::new(),
        ),
        Declaration::Variable { name, ty, flags } => (
            name.clone(),
            MemberKind::Variable,
            Some(ty.clone()),
            flags.clone(),
            Vec::new(),
        ),
        _ => return Err(error(path, "unexpected declaration kind")),
    };
    let is_auto = flags
        .iter()
        .any(|flag| flag == "auto" || flag == "autoreadonly");
    let is_read_only = flags.iter().any(|flag| flag == "autoreadonly");
    let accessors = node
        .children()
        .filter(|child| child.kind() == SyntaxKind::Block)
        .flat_map(|block| block.children())
        .filter(|child| child.kind() == SyntaxKind::FunctionDecl)
        .filter_map(|child| declaration(summaries, &child, path).ok())
        .filter_map(|declaration| match declaration {
            Declaration::Function { name, .. } => Some(name.to_ascii_lowercase()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    // A nested literal in a binary expression is not the declaration's value.
    let initial_literal = node
        .children()
        .find(|child| is_expression(child.kind()))
        .filter(is_literal_expression)
        .map(|child| child.text().to_string());
    let global = flags.iter().any(|flag| flag == "global");
    let native = flags.iter().any(|flag| flag == "native");
    if global && kind != MemberKind::Function {
        return Err(error(path, "only functions can be global"));
    }
    let data = match kind {
        MemberKind::Function => MemberData::Function {
            return_type: ty,
            global,
            native,
            parameters,
        },
        MemberKind::Event => MemberData::Event { native, parameters },
        MemberKind::Property => {
            let access = if is_read_only {
                PropertyAccess::AutoReadOnly
            } else if is_auto {
                PropertyAccess::Auto
            } else {
                PropertyAccess::Manual {
                    readable: accessors.contains("get"),
                    writable: accessors.contains("set"),
                }
            };
            MemberData::Property {
                ty: ty.expect("property type extracted"),
                access,
                initial_literal,
            }
        }
        MemberKind::Variable => MemberData::Variable {
            ty: ty.expect("variable type extracted"),
            initial_literal,
        },
        MemberKind::UnknownCallable => unreachable!("PSC retains callable kinds"),
    };
    Ok(Member { name, flags, data })
}

fn is_expression(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::LiteralExpr
            | SyntaxKind::NameExpr
            | SyntaxKind::UnaryExpr
            | SyntaxKind::BinaryExpr
            | SyntaxKind::MemberExpr
            | SyntaxKind::IndexExpr
            | SyntaxKind::CallExpr
            | SyntaxKind::ParenExpr
            | SyntaxKind::NewArrayExpr
    )
}

fn is_literal_expression(node: &SyntaxNode) -> bool {
    match node.kind() {
        SyntaxKind::LiteralExpr => true,
        SyntaxKind::UnaryExpr => {
            let mut expressions = node.children().filter(|child| is_expression(child.kind()));
            expressions
                .next()
                .is_some_and(|child| is_literal_expression(&child))
                && expressions.next().is_none()
        }
        _ => false,
    }
}

fn parameter(source: &folio_papyrus::Parameter) -> Parameter {
    Parameter {
        name: source.name.clone(),
        ty: source.ty.clone(),
        default: source
            .default
            .clone()
            .map(ParameterDefault::Literal)
            .unwrap_or(ParameterDefault::Required),
    }
}

fn line_column(text: &str, byte: usize) -> (u32, u32) {
    let prefix = &text[..byte];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32 + 1;
    let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() as u32 + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_state_variable_and_property_access() {
        let text = "ScriptName Sample Extends Quest Hidden\nImport Utility\nInt count = 3\nInt Property Value AutoReadOnly\nAuto State Busy\nEvent OnUpdate(Int ticks = 1)\nEndEvent\nEndState\n";
        let bundle = generate(
            GenerationOptions { source: "fixture" },
            &[SourceInput {
                path: "Sample.psc",
                text,
            }],
        )
        .unwrap();
        let script = &bundle.scripts[0];
        assert_eq!(script.parent.as_deref(), Some("Quest"));
        assert_eq!(script.imports, ["Utility"]);
        assert!(script.states[0].auto);
        assert_eq!(
            script.states[0].members[0].parameters()[0]
                .default
                .literal(),
            Some("1")
        );
        assert!(
            script
                .members
                .iter()
                .any(|member| member.kind() == MemberKind::Variable && member.name == "count")
        );
        let property = script
            .members
            .iter()
            .find(|member| member.name == "Value")
            .unwrap();
        assert!(property.is_read_only() && property.is_readable() && !property.is_writable());
    }

    #[test]
    fn content_order_does_not_change_output_and_bad_names_fail() {
        let first = SourceInput {
            path: "A.psc",
            text: "ScriptName A\n",
        };
        let second = SourceInput {
            path: "B.psc",
            text: "ScriptName B\n",
        };
        let options = || GenerationOptions { source: "fixture" };
        assert_eq!(
            generate(options(), &[first, second]).unwrap(),
            generate(options(), &[second, first]).unwrap()
        );
        assert!(
            generate(
                options(),
                &[SourceInput {
                    path: "A.psc",
                    text: "ScriptName Other\n"
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn merges_reopened_states_and_keeps_variable_function_namespaces() {
        let text = "ScriptName Sample\nBool busy\nState Ready\nEndState\nState Ready\nFunction Wait()\nEndFunction\nEndState\nFunction Busy()\nEndFunction\n";
        let bundle = generate(
            GenerationOptions { source: "fixture" },
            &[SourceInput {
                path: "Sample.psc",
                text,
            }],
        )
        .unwrap();
        let script = &bundle.scripts[0];
        assert_eq!(script.states.len(), 1);
        assert_eq!(script.states[0].members[0].name, "Wait");
        assert_eq!(
            script
                .members
                .iter()
                .filter(|member| member.name.eq_ignore_ascii_case("busy"))
                .count(),
            2
        );
    }

    #[test]
    fn records_only_complete_literal_initializers() {
        let text = "ScriptName Sample\nInt plain = 7\nInt negative = -2\nInt calculated = 1 + 2\n";
        let bundle = generate(
            GenerationOptions { source: "fixture" },
            &[SourceInput {
                path: "Sample.psc",
                text,
            }],
        )
        .unwrap();
        let members = &bundle.scripts[0].members;
        assert_eq!(members[0].initial_literal(), Some("7"));
        assert_eq!(members[1].initial_literal(), Some("-2"));
        assert_eq!(members[2].initial_literal(), None);
    }
}
