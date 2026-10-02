//! Pure, recoverable symbol and type analysis over one coherent input view.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use folio_diagnostics::{Diagnostic, RelatedLocation, Severity};
use folio_format_declarations::{MemberKind, ParameterDefault, Script as ExternalScript};
use folio_hir::{
    Binding, Body, CallFact, DeclarationFact, ExpressionFact, ExpressionKind, MemberFact,
    MemberKind as HirMemberKind, NameRef, ParameterFact, Script, StateFact, Statement, Symbol,
    Type,
};
use folio_papyrus::{Declaration, SyntaxKind, SyntaxNode, SyntaxToken};
use folio_source::{FileId, SourceSpan, TextRange};

use super::{AnalysisCancelled, AnalysisView};

pub(super) struct FileAnalysis {
    pub script: Script,
    pub diagnostics: Vec<Diagnostic>,
}

pub(super) struct Analysis {
    files: BTreeMap<FileId, FileAnalysis>,
    pub project_diagnostics: Vec<Diagnostic>,
    world: World,
}

impl Analysis {
    pub fn file(&self, file: FileId) -> Option<&FileAnalysis> {
        self.files.get(&file)
    }
}

#[derive(Clone)]
struct MemberInfo {
    name: String,
    ty: Type,
    kind: MemberKind,
    parameters: Vec<(String, Type, ParameterDefault)>,
    global: bool,
    auto: bool,
    read_only: bool,
    readable: bool,
    writable: bool,
    definition: Option<SourceSpan>,
}

#[derive(Clone)]
struct ScriptInfo {
    name: String,
    parent: Option<String>,
    definition: Option<SourceSpan>,
    members: BTreeMap<String, MemberInfo>,
    variables: BTreeMap<String, MemberInfo>,
    callable_overloads: BTreeMap<String, MemberInfo>,
    states: BTreeMap<String, BTreeMap<String, MemberInfo>>,
}

struct World {
    scripts: BTreeMap<String, ScriptInfo>,
}

fn key(name: &str) -> String {
    name.to_lowercase()
}

fn state_runtime_intrinsic_type(name: &str) -> Option<Type> {
    crate::intrinsic_signature(name, None).map(|signature| signature.result)
}

fn span(file: FileId, node: &SyntaxNode) -> SourceSpan {
    let range = node.text_range();
    SourceSpan {
        file,
        range: TextRange {
            start: usize::from(range.start()),
            end: usize::from(range.end()),
        },
    }
}

fn token_span(file: FileId, token: &SyntaxToken) -> SourceSpan {
    let range = token.text_range();
    SourceSpan {
        file,
        range: TextRange {
            start: usize::from(range.start()),
            end: usize::from(range.end()),
        },
    }
}

fn name_token(node: &SyntaxNode, skip: &[&str]) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|item| item.into_token())
        .filter(|token| token.kind() == SyntaxKind::Ident)
        .find(|token| {
            !skip
                .iter()
                .any(|word| token.text().eq_ignore_ascii_case(word))
        })
}

fn type_text(node: &SyntaxNode) -> String {
    node.children_with_tokens()
        .filter_map(|item| item.into_token())
        .filter(|token| {
            matches!(
                token.kind(),
                SyntaxKind::Ident | SyntaxKind::LBracket | SyntaxKind::RBracket
            )
        })
        .map(|token| token.text().to_string())
        .collect()
}

fn enclosing_name(node: &SyntaxNode, kind: SyntaxKind, keyword: &str) -> Option<String> {
    let parent = node
        .ancestors()
        .skip(1)
        .find(|ancestor| ancestor.kind() == kind)?;
    let mut saw_keyword = false;
    for token in parent
        .children_with_tokens()
        .take_while(|item| item.kind() != SyntaxKind::Block)
        .filter_map(|item| item.into_token())
    {
        if token.kind() != SyntaxKind::Ident {
            continue;
        }
        if saw_keyword {
            return Some(token.text().to_string());
        }
        saw_keyword = token.text().eq_ignore_ascii_case(keyword);
    }
    None
}

fn diagnostic(code: &str, message: impl Into<String>, at: SourceSpan) -> Diagnostic {
    Diagnostic::new(code, Severity::Error, message).at(at)
}

/// Skyrim-compatible call padding uses the VM default for each concrete type.
fn missing_argument_literal(ty: &Type) -> Option<&'static str> {
    match ty {
        Type::Bool => Some("false"),
        Type::Int => Some("0"),
        Type::Float => Some("0.0"),
        Type::String => Some("\"\""),
        Type::Script(_) | Type::Array(_) => Some("None"),
        Type::Void | Type::None | Type::Error => None,
    }
}

fn source_declaration_node(
    view: &AnalysisView,
    file: FileId,
    range: TextRange,
) -> Option<SyntaxNode> {
    let parse = view.parse(file)?;
    let root = parse.syntax();
    let range = rowan::TextRange::new(
        u32::try_from(range.start).ok()?.into(),
        u32::try_from(range.end).ok()?.into(),
    );
    if !root.text_range().contains_range(range) {
        return None;
    }
    let element = root.covering_element(range);
    let node = match element {
        rowan::NodeOrToken::Node(node) => node,
        rowan::NodeOrToken::Token(token) => token.parent()?,
    };
    // Equal-range wrappers must retain the outermost match used by the tree walk.
    node.ancestors()
        .filter(|node| node.text_range() == range)
        .last()
}

/// Builds the selected source and external declaration symbol environment, then analyzes each source independently.
pub(super) fn analyze(view: &AnalysisView) -> Analysis {
    analyze_with_cancel(view, &|| false).expect("non-cancellable analysis cannot be cancelled")
}

#[tracing::instrument(skip(view, cancelled), fields(generation = view.generation(), phase = "analysis.semantic"))]
pub(super) fn analyze_with_cancel(
    view: &AnalysisView,
    cancelled: &dyn Fn() -> bool,
) -> Result<Analysis, AnalysisCancelled> {
    let started = std::time::Instant::now();
    if cancelled() {
        return Err(AnalysisCancelled);
    }
    let mut analysis = Analysis {
        files: BTreeMap::new(),
        project_diagnostics: Vec::new(),
        world: World {
            scripts: BTreeMap::new(),
        },
    };
    let mut world = World {
        scripts: BTreeMap::new(),
    };
    let mut file_scripts = BTreeMap::new();
    for file in view.file_ids() {
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        let mut file_analysis = FileAnalysis {
            script: Script::default(),
            diagnostics: Vec::new(),
        };
        if let Some(parse) = view.parse(file) {
            for error in &parse.errors {
                file_analysis.diagnostics.push(diagnostic(
                    "papyrus.syntax",
                    error.message.clone(),
                    SourceSpan {
                        file,
                        range: error.range,
                    },
                ));
            }
        }
        if let Some(located) = view.located_declarations(file) {
            for item in located.iter() {
                if let Declaration::Script {
                    name,
                    parent,
                    flags,
                } = &item.declaration
                {
                    file_analysis.script.flags = flags.clone();
                    let node = source_declaration_node(view, file, item.range);
                    let name_span = node
                        .as_ref()
                        .and_then(|node| name_token(node, &["scriptname"]))
                        .map(|token| token_span(file, &token))
                        .unwrap_or(SourceSpan {
                            file,
                            range: item.range,
                        });
                    let name_ref = NameRef {
                        text: name.clone(),
                        span: name_span,
                    };
                    if let Some(parent) = parent {
                        let parent_span = node
                            .as_ref()
                            .and_then(|node| {
                                node.children_with_tokens()
                                    .filter_map(|item| item.into_token())
                                    .filter(|token| token.kind() == SyntaxKind::Ident)
                                    .find(|token| token.text().eq_ignore_ascii_case(parent))
                            })
                            .map(|token| token_span(file, &token))
                            .unwrap_or(name_span);
                        file_analysis.script.parent = Some(NameRef {
                            text: parent.clone(),
                            span: parent_span,
                        });
                    }
                    file_analysis.script.name = Some(name_ref);
                    if let Some(previous) = world.scripts.get(&key(name)) {
                        let mut error = diagnostic(
                            "semantic.duplicate-script",
                            format!("duplicate script {name}"),
                            name_span,
                        );
                        if let Some(location) = previous.definition {
                            error.related.push(RelatedLocation {
                                span: location,
                                message: "previous script".into(),
                            });
                        }
                        file_analysis.diagnostics.push(error);
                    } else {
                        file_scripts.insert(file, key(name));
                        world.scripts.insert(
                            key(name),
                            ScriptInfo {
                                name: name.clone(),
                                parent: parent.clone(),
                                definition: Some(name_span),
                                members: BTreeMap::new(),
                                variables: BTreeMap::new(),
                                callable_overloads: BTreeMap::new(),
                                states: BTreeMap::new(),
                            },
                        );
                    }
                    break;
                }
            }
        }
        analysis.files.insert(file, file_analysis);
    }
    for bundle in view.external_declarations.iter() {
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        for external in &bundle.scripts {
            if cancelled() {
                return Err(AnalysisCancelled);
            }
            let k = key(&external.name);
            if world.scripts.contains_key(&k) {
                analysis.project_diagnostics.push(Diagnostic::new(
                    "semantic.duplicate-script",
                    Severity::Error,
                    format!(
                        "external script {} conflicts with selected source",
                        external.name
                    ),
                ));
                continue;
            }
            validate_external_initializers(external, &mut analysis.project_diagnostics);
            world.scripts.insert(k, script_from_external(external));
        }
    }
    // Source member signatures are collected before bodies to support mutual calls.
    for (&file, script_key) in &file_scripts {
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        let Some(located) = view.located_declarations(file) else {
            continue;
        };
        for item in located.iter() {
            if cancelled() {
                return Err(AnalysisCancelled);
            }
            if matches!(
                item.declaration,
                Declaration::Script { .. } | Declaration::Import { .. } | Declaration::State { .. }
            ) {
                if let Declaration::State { name, flags } = &item.declaration {
                    analysis
                        .files
                        .get_mut(&file)
                        .unwrap()
                        .script
                        .states
                        .push(StateFact {
                            name: name.clone(),
                            auto: flags.iter().any(|flag| flag.eq_ignore_ascii_case("auto")),
                            span: SourceSpan {
                                file,
                                range: item.range,
                            },
                        });
                }
                continue;
            }
            let Some(node) = source_declaration_node(view, file, item.range) else {
                continue;
            };
            if enclosing_name(&node, SyntaxKind::PropertyDecl, "property").is_some() {
                continue;
            }
            if let Some(member) = member_from_source(file, &node, &item.declaration) {
                let script = world
                    .scripts
                    .get_mut(script_key)
                    .expect("source script exists");
                let member_key = key(&member.name);
                let state = enclosing_name(&node, SyntaxKind::StateDecl, "state");
                let table = if let Some(state) = &state {
                    script.states.entry(key(state)).or_default()
                } else {
                    &mut script.members
                };
                if let Some(previous) = table.get(&member_key) {
                    let mut error = diagnostic(
                        "semantic.duplicate-member",
                        format!("duplicate member {}", member.name),
                        member.definition.unwrap(),
                    );
                    if let Some(location) = previous.definition {
                        error.related.push(RelatedLocation {
                            span: location,
                            message: "previous member".into(),
                        });
                    }
                    analysis
                        .files
                        .get_mut(&file)
                        .unwrap()
                        .diagnostics
                        .push(error);
                    continue;
                }
                table.insert(member_key, member.clone());
                analysis
                    .files
                    .get_mut(&file)
                    .unwrap()
                    .script
                    .declarations
                    .push(DeclarationFact {
                        symbol: if let Some(state) = state {
                            Symbol::StateMember {
                                script: script.name.clone(),
                                state,
                                name: member.name.clone(),
                            }
                        } else {
                            Symbol::Member {
                                script: script.name.clone(),
                                name: member.name.clone(),
                            }
                        },
                        ty: member.ty.clone(),
                        span: member.definition.unwrap(),
                    });
                let source_symbol =
                    if let Some(state) = enclosing_name(&node, SyntaxKind::StateDecl, "state") {
                        Symbol::StateMember {
                            script: script.name.clone(),
                            state,
                            name: member.name.clone(),
                        }
                    } else {
                        Symbol::Member {
                            script: script.name.clone(),
                            name: member.name.clone(),
                        }
                    };
                let (kind, flags, parameters) = match &item.declaration {
                    Declaration::Variable { flags, .. } => {
                        (HirMemberKind::Variable, flags.clone(), Vec::new())
                    }
                    Declaration::Property { flags, .. } => (
                        HirMemberKind::Property {
                            auto: flags.iter().any(|flag| {
                                flag.eq_ignore_ascii_case("auto")
                                    || flag.eq_ignore_ascii_case("autoreadonly")
                            }),
                            read_only: flags
                                .iter()
                                .any(|flag| flag.eq_ignore_ascii_case("autoreadonly")),
                        },
                        flags.clone(),
                        Vec::new(),
                    ),
                    Declaration::Function {
                        parameters,
                        modifiers,
                        ..
                    } => (
                        HirMemberKind::Function {
                            event: false,
                            global: modifiers
                                .iter()
                                .any(|modifier| modifier.eq_ignore_ascii_case("global")),
                            native: modifiers
                                .iter()
                                .any(|modifier| modifier.eq_ignore_ascii_case("native")),
                        },
                        modifiers.clone(),
                        parameters
                            .iter()
                            .map(|parameter| ParameterFact {
                                name: parameter.name.clone(),
                                ty: Type::from_spelling(&parameter.ty),
                                default_literal: parameter.default.clone(),
                                span: member.definition.unwrap(),
                            })
                            .collect(),
                    ),
                    Declaration::Event {
                        parameters,
                        modifiers,
                        ..
                    } => (
                        HirMemberKind::Function {
                            event: true,
                            global: false,
                            native: modifiers
                                .iter()
                                .any(|modifier| modifier.eq_ignore_ascii_case("native")),
                        },
                        modifiers.clone(),
                        parameters
                            .iter()
                            .map(|parameter| ParameterFact {
                                name: parameter.name.clone(),
                                ty: Type::from_spelling(&parameter.ty),
                                default_literal: parameter.default.clone(),
                                span: member.definition.unwrap(),
                            })
                            .collect(),
                    ),
                    _ => unreachable!(),
                };
                let initial_literal = direct_expression(&node)
                    .and_then(|expr| folio_papyrus::constant_literal_text(&expr));
                analysis
                    .files
                    .get_mut(&file)
                    .unwrap()
                    .script
                    .members
                    .push(MemberFact {
                        symbol: source_symbol,
                        kind,
                        ty: member.ty.clone(),
                        parameters,
                        flags,
                        initial_literal,
                        span: member.definition.unwrap(),
                    });
            }
        }
    }
    // Export the resolved ancestor layout once, so lowering never reopens source or external declarations.
    for (&file, script_key) in &file_scripts {
        let mut inherited = Vec::new();
        let mut seen = HashSet::new();
        let mut uncertain_overrides = HashSet::new();
        let mut override_diagnostics = Vec::new();
        let mut current = world
            .scripts
            .get(script_key)
            .and_then(|script| script.parent.clone());
        while let Some(parent_name) = current {
            let parent_key = key(&parent_name);
            if !seen.insert(parent_key.clone()) {
                break;
            }
            let Some(parent) = world.scripts.get(&parent_key) else {
                break;
            };
            for member in &analysis.files[&file].script.members {
                if !matches!(member.kind, HirMemberKind::Function { .. }) {
                    continue;
                }
                let (identity, inherited_member) = match &member.symbol {
                    Symbol::Member { name, .. } => (
                        key(name),
                        parent
                            .members
                            .get(&key(name))
                            .or_else(|| parent.callable_overloads.get(&key(name))),
                    ),
                    Symbol::StateMember { state, name, .. } => (
                        format!("{}:{}", key(state), key(name)),
                        parent
                            .states
                            .get(&key(state))
                            .and_then(|members| members.get(&key(name))),
                    ),
                    _ => continue,
                };
                if inherited_member.is_some_and(|item| item.kind == MemberKind::UnknownCallable)
                    && uncertain_overrides.insert(identity)
                {
                    override_diagnostics.push(
                        Diagnostic::new(
                            "semantic.override-ambiguous",
                            Severity::Error,
                            "cannot override a declaration whose event/function kind is unknown",
                        )
                        .at(member.span),
                    );
                }
            }
            if let Some((&parent_file, _)) =
                file_scripts.iter().find(|(_, key)| **key == parent_key)
            {
                inherited.extend(analysis.files[&parent_file].script.members.iter().cloned());
            } else {
                let at = analysis.files[&file]
                    .script
                    .parent
                    .as_ref()
                    .map(|name| name.span)
                    .unwrap_or(SourceSpan {
                        file,
                        range: TextRange { start: 0, end: 0 },
                    });
                inherited.extend(
                    parent
                        .members
                        .values()
                        .chain(parent.variables.values())
                        .chain(parent.callable_overloads.values())
                        .map(|member| MemberFact {
                            symbol: Symbol::Member {
                                script: parent.name.clone(),
                                name: member.name.clone(),
                            },
                            kind: match member.kind {
                                MemberKind::Variable => HirMemberKind::Variable,
                                MemberKind::Property => HirMemberKind::Property {
                                    auto: member.auto,
                                    read_only: member.read_only,
                                },
                                MemberKind::Function
                                | MemberKind::Event
                                // Calls share one opcode; overriding an unknown PEX kind was
                                // rejected before this lowering fact is assembled.
                                | MemberKind::UnknownCallable => HirMemberKind::Function {
                                    event: member.kind == MemberKind::Event,
                                    global: member.global,
                                    native: true,
                                },
                            },
                            ty: member.ty.clone(),
                            parameters: member
                                .parameters
                                .iter()
                                .map(|(name, ty, default)| ParameterFact {
                                    name: name.clone(),
                                    ty: ty.clone(),
                                    default_literal: default.literal().map(str::to_owned),
                                    span: at,
                                })
                                .collect(),
                            flags: Vec::new(),
                            initial_literal: None,
                            span: at,
                        }),
                );
            }
            current = parent.parent.clone();
        }
        analysis
            .files
            .get_mut(&file)
            .unwrap()
            .script
            .external_members = inherited;
        analysis
            .files
            .get_mut(&file)
            .unwrap()
            .diagnostics
            .extend(override_diagnostics);
    }
    if cancelled() {
        return Err(AnalysisCancelled);
    }
    validate_world(&world, &mut analysis, &file_scripts);
    for (&file, script_key) in &file_scripts {
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        let Some(parse) = view.parse(file) else {
            continue;
        };
        let file_analysis = analysis.files.get_mut(&file).unwrap();
        analyze_file(
            view,
            &world,
            file,
            script_key,
            &parse.syntax(),
            file_analysis,
            cancelled,
        )?;
    }
    // Preserve the declaration kind of cross-script uses for lowering. A typed
    // member reference alone cannot distinguish a field from a property/function.
    for &file in file_scripts.keys() {
        let refs = analysis.files[&file]
            .script
            .expressions
            .iter()
            .filter_map(|expression| {
                expression
                    .binding
                    .as_ref()
                    .map(|binding| (binding.symbol.clone(), expression.span))
            })
            .collect::<Vec<_>>();
        let mut seen = HashSet::new();
        let mut referenced = Vec::new();
        for (symbol, at) in refs {
            if !seen.insert(symbol.clone())
                || analysis.files[&file]
                    .script
                    .members
                    .iter()
                    .chain(&analysis.files[&file].script.external_members)
                    .any(|member| member.symbol == symbol)
            {
                continue;
            }
            let (script_name, member_name) = match &symbol {
                Symbol::Member { script, name } | Symbol::StateMember { script, name, .. } => {
                    (script, name)
                }
                _ => continue,
            };
            let fact = file_scripts
                .iter()
                .find(|(_, candidate)| *candidate == &key(script_name))
                .and_then(|(owner_file, _)| {
                    analysis.files[owner_file]
                        .script
                        .members
                        .iter()
                        .find(|member| member.symbol == symbol)
                        .cloned()
                })
                .or_else(|| {
                    world
                        .scripts
                        .get(&key(script_name))
                        .and_then(|script| match &symbol {
                            Symbol::StateMember { state, .. } => script
                                .states
                                .get(&key(state))
                                .and_then(|members| members.get(&key(member_name))),
                            _ => script
                                .members
                                .get(&key(member_name))
                                .or_else(|| script.variables.get(&key(member_name)))
                                .or_else(|| script.callable_overloads.get(&key(member_name))),
                        })
                        .map(|member| MemberFact {
                            symbol: symbol.clone(),
                            kind: match member.kind {
                                MemberKind::Variable => HirMemberKind::Variable,
                                MemberKind::Property => HirMemberKind::Property {
                                    auto: member.auto,
                                    read_only: member.read_only,
                                },
                                MemberKind::Function
                                | MemberKind::Event
                                | MemberKind::UnknownCallable => HirMemberKind::Function {
                                    event: member.kind == MemberKind::Event,
                                    global: member.global,
                                    native: true,
                                },
                            },
                            ty: member.ty.clone(),
                            parameters: member
                                .parameters
                                .iter()
                                .map(|(name, ty, default)| ParameterFact {
                                    name: name.clone(),
                                    ty: ty.clone(),
                                    default_literal: default.literal().map(str::to_owned),
                                    span: at,
                                })
                                .collect(),
                            flags: Vec::new(),
                            initial_literal: None,
                            span: at,
                        })
                });
            if let Some(fact) = fact {
                referenced.push(fact);
            }
        }
        analysis
            .files
            .get_mut(&file)
            .unwrap()
            .script
            .referenced_members = referenced;
    }
    if cancelled() {
        return Err(AnalysisCancelled);
    }
    tracing::debug!(
        elapsed_us = started.elapsed().as_micros(),
        files = analysis.files.len(),
        external_scripts = world.scripts.len() - file_scripts.len(),
        "semantic analysis complete"
    );
    analysis.world = world;
    Ok(analysis)
}

#[derive(Clone)]
struct Local {
    ty: Type,
    symbol: Symbol,
    definition: SourceSpan,
}

/// Call binding keeps the source-order mapping and declaration-order omitted values together.
struct CheckedCall {
    result: Type,
    target: Option<Symbol>,
    ordinals: Vec<usize>,
    defaults: Vec<Option<(Type, String)>>,
}

impl CheckedCall {
    fn error() -> Self {
        Self {
            result: Type::Error,
            target: None,
            ordinals: Vec::new(),
            defaults: Vec::new(),
        }
    }
}

struct Scope<'a> {
    world: &'a World,
    file: FileId,
    script: &'a ScriptInfo,
    member: &'a MemberInfo,
    callable: Symbol,
    state: Option<String>,
    imports: &'a [String],
    locals: BTreeMap<String, Local>,
    parameters: Vec<ParameterFact>,
    result: &'a mut FileAnalysis,
    fill_missing_arguments: bool,
    cancelled: &'a dyn Fn() -> bool,
    interrupted: bool,
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

fn direct_expression(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.children().find(|child| is_expression(child.kind()))
}

fn direct_expressions(node: &SyntaxNode) -> impl Iterator<Item = SyntaxNode> + '_ {
    node.children().filter(|child| is_expression(child.kind()))
}

mod editor;
mod file;
mod scope;
mod world;
pub(super) use editor::completion_candidates;

use file::analyze_file;
use world::*;

#[cfg(test)]
mod tests;
