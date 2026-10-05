//! Pure, recoverable symbol and type analysis over one coherent input view.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::Arc,
};

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

/// Finished files share their HIR directly with consumers without cloning bodies.
pub(super) struct AnalyzedFile {
    pub script: Arc<Script>,
    pub diagnostics: Vec<Diagnostic>,
}

pub(super) struct Analysis {
    files: BTreeMap<FileId, AnalyzedFile>,
    pub project_diagnostics: Vec<Diagnostic>,
    world: World,
}

struct WorkingAnalysis {
    files: BTreeMap<FileId, FileAnalysis>,
    pub project_diagnostics: Vec<Diagnostic>,
}

impl Analysis {
    pub fn file(&self, file: FileId) -> Option<&AnalyzedFile> { self.files.get(&file) }
}

#[derive(Clone)]
struct MemberInfo {
    name: String,
    ty: Type,
    kind: MemberKind,
    parameters: Vec<(String, Type, ParameterDefault)>,
    global: bool,
    property: PropertyForm,
    readable: bool,
    writable: bool,
    definition: Option<SourceSpan>,
}

/// Property declaration shape is distinct from access permissions and callable flags.
#[derive(Clone)]
struct PropertyForm {
    auto: bool,
    read_only: bool,
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
    scripts: BTreeMap<String, Arc<ScriptInfo>>,
}

fn key(name: &str) -> String { name.to_lowercase() }

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
        .filter_map(rowan::NodeOrToken::into_token)
        .filter(|token| token.kind() == SyntaxKind::Ident)
        .find(|token| {
            !skip
                .iter()
                .any(|word| token.text().eq_ignore_ascii_case(word))
        })
}

fn type_text(node: &SyntaxNode) -> String {
    node.children_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
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
        .filter_map(rowan::NodeOrToken::into_token)
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

#[tracing::instrument(skip(view, cancelled), fields(generation = view.generation(), phase = "analysis.semantic"))]
pub(super) fn analyze_with_cancel(
    view: &AnalysisView,
    cancelled: &dyn Fn() -> bool,
) -> Result<Analysis, AnalysisCancelled> {
    let started = std::time::Instant::now();
    if cancelled() {
        return Err(AnalysisCancelled);
    }
    let mut analysis = WorkingAnalysis {
        files: BTreeMap::new(),
        project_diagnostics: Vec::new(),
    };
    let mut world = World {
        scripts: BTreeMap::new(),
    };
    let mut file_scripts = BTreeMap::new();
    collect_source_scripts(
        view,
        cancelled,
        &mut analysis,
        &mut world,
        &mut file_scripts,
    )?;
    collect_external_scripts(view, cancelled, &mut analysis, &mut world)?;
    collect_source_members(view, cancelled, &mut analysis, &mut world, &file_scripts)?;
    export_inherited_members(&mut analysis, &world, &file_scripts);
    if cancelled() {
        return Err(AnalysisCancelled);
    }
    analysis.project_diagnostics.extend(
        view.external_declarations
            .validate_world(&world, cancelled)?
            .iter()
            .cloned(),
    );
    validate_world(&world, &mut analysis, &file_scripts);
    for (&file, script_key) in &file_scripts {
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        let Some(parse) = view.parse(file) else {
            continue;
        };
        let file_analysis = analysis
            .files
            .get_mut(&file)
            .expect("file_scripts only contains initialized source files");
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
    export_referenced_members(&mut analysis, &world, &file_scripts);
    if cancelled() {
        return Err(AnalysisCancelled);
    }
    tracing::debug!(
        elapsed_us = started.elapsed().as_micros(),
        files = analysis.files.len(),
        external_scripts = world.scripts.len() - file_scripts.len(),
        "semantic analysis complete"
    );
    Ok(Analysis {
        files: analysis
            .files
            .into_iter()
            .map(|(file, result)| {
                (
                    file,
                    AnalyzedFile {
                        script: Arc::new(result.script),
                        diagnostics: result.diagnostics,
                    },
                )
            })
            .collect(),
        project_diagnostics: analysis.project_diagnostics,
        world,
    })
}

mod pipeline;
use pipeline::{
    collect_external_scripts, collect_source_members, collect_source_scripts,
    export_inherited_members, export_referenced_members,
};

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
use world::{
    assignable, castable, implicitly_convertible, known_type, lookup_callable_member,
    lookup_member, lookup_state_member, member_from_source, script_from_external,
    validate_external_defaults, validate_external_initializers, validate_external_world,
    validate_world,
};
mod external;
pub(crate) use external::ExternalDeclarations;

#[cfg(test)]
mod tests;
