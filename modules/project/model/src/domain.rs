//! Project identities and resolved decisions, independent of file loading.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Byte offsets in a named manifest or declaration carrier.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SourceSpan {
    pub source: String,
    pub start: usize,
    pub end: usize,
}

/// Root packages have versions; dependency aliases identify versionless API inputs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct PackageId {
    pub name: String,
    pub version: Option<String>,
    pub source: SourceId,
}

/// Paths are portable, relative identities supplied by the declaring manifest.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "source_kind", rename_all = "kebab-case")]
pub enum SourceId {
    Project,
    Dependency {
        index: usize,
        kind: DependencyKind,
        path: String,
        digest: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyKind {
    Psc,
    Pex,
    Decl,
    Repo,
}

/// Decoding applies equally to direct PSC dependencies and declaration generation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceEncoding {
    #[default]
    Utf8,
    Windows1252,
}

/// A parsed single-package manifest with field-level source positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub source: String,
    pub fields: BTreeMap<String, SourceSpan>,
    pub name: String,
    pub version: String,
    pub source_path: LocatedString,
    pub output_path: LocatedString,
    pub language: String,
    pub dialect: String,
    pub extensions: Vec<String>,
    /// Project-defined Papyrus modifier names available to semantic analysis.
    pub user_flags: Vec<folio_profiles::UserFlag>,
    /// Whether omitted required Papyrus call arguments receive type defaults.
    pub fill_missing_arguments: bool,
    /// Rule ID to severity (off, info, warning, error) for project linting.
    pub lint_rules: BTreeMap<String, String>,
    pub target: String,
    pub profile: String,
    /// Whether generated PEX includes source line debug information.
    pub debug_info: bool,
    /// Explicit gate for binary PEX API dependencies.
    pub experimental_pex_dependencies: bool,
    pub emit: Vec<String>,
    pub dependencies: Vec<DependencySpec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatedString {
    pub value: String,
    pub span: SourceSpan,
}

/// Each dependency supplies leaf declarations; only PSC inputs have a text encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencySpec {
    pub name: LocatedString,
    pub kind: DependencyKind,
    pub path: LocatedString,
    pub encoding: SourceEncoding,
}

/// A source file candidate recorded by the I/O shell, before Papyrus parsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFile {
    /// Portable root-relative path used for identity.
    pub path: String,
    /// Original manifest-root spelling retained for explanation.
    pub display_path: String,
    pub script_candidate: String,
}

/// Exported script identity and optional source span; signatures remain in the
/// declaration bundle retained by project/resolve for semantic projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredScript {
    pub name: String,
    pub location: DeclarationLocation,
}

/// Position inside a declaration carrier, with optional original source map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DeclarationLocation {
    pub carrier_path: String,
    pub script_name: String,
    pub source_path: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

/// All inputs to the pure resolver are supplied explicitly by the loading shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedRoot {
    pub source_key: String,
    pub source_id: SourceId,
    pub manifest: Manifest,
    pub source_files: Vec<SourceFile>,
}

/// The source of one dependency, independent of how its scripts are selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedDependency {
    pub source_key: String,
    pub source_id: SourceId,
    pub kind: DependencyKind,
    pub name: String,
    pub declared_path: String,
    pub canonical_path: PathBuf,
    pub declaration: SourceSpan,
    pub profile: String,
    pub scripts: Vec<DeclaredScript>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DependencyEdge {
    pub from: PackageId,
    pub to: PackageId,
    pub kind: DependencyKind,
    pub declared_path: String,
    pub declaration: SourceSpan,
}

/// Providers are ordered from lowest to highest precedence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ScriptProvider {
    pub script: String,
    pub package: PackageId,
    pub definition: Option<SourceSpan>,
    pub declaration: Option<DeclarationLocation>,
    pub source_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ScriptSelection {
    pub script: String,
    pub selected: ScriptProvider,
    pub providers: Vec<ScriptProvider>,
    pub reason: SelectionReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SelectionReason {
    SoleProvider,
    DependencyOrder,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResolvedPackage {
    pub id: PackageId,
    pub source_root: Option<String>,
    pub source_files: Vec<String>,
    pub language: Option<String>,
    pub dialect: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExternalRequirement {
    pub package: PackageId,
    pub target: String,
    pub abi: String,
    pub reason: String,
}

/// Versioned, deterministic machine payload for `metadata --format json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Metadata {
    pub schema: u32,
    pub root: PackageId,
    pub target: String,
    pub profile: String,
    pub fill_missing_arguments: bool,
    pub debug_info: bool,
    pub source: String,
    pub output: String,
    pub user_flags: Vec<folio_profiles::UserFlag>,
    pub packages: Vec<ResolvedPackage>,
    pub dependencies: Vec<DependencyEdge>,
    pub scripts: Vec<ScriptSelection>,
    pub external_requirements: Vec<ExternalRequirement>,
}
