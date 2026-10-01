//! Project source projection into the shared, in-memory analysis host.

pub mod execute;
pub mod fingerprint;
pub mod output;
pub mod plan;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use folio_analysis::{AnalysisHost, AnalysisView, InputEdit, InputError};
use folio_diagnostics::{Diagnostic, Severity};
use folio_format_declarations::DeclarationBundle;
use folio_papyrus::{Declaration, PapyrusDialect};
use folio_project_model::{Metadata, SourceId};
use folio_project_resolve::LoadedProject;
use folio_source::{FileId, Revision, SourceSpan, TextRange};

/// One resolver-loaded source and its explicit language setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSource {
    pub package_key: String,
    pub canonical_path: PathBuf,
    pub display_path: String,
    pub script_candidate: String,
    pub dialect: PapyrusDialect,
    pub text: Arc<str>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SourceKey {
    package_key: String,
    canonical_path: PathBuf,
}

impl ProjectSource {
    fn key(&self) -> SourceKey {
        SourceKey {
            package_key: self.package_key.clone(),
            canonical_path: self.canonical_path.clone(),
        }
    }
}

/// Maps only the files explicitly loaded by the project resolver.
pub fn sources_from_loaded(project: &LoadedProject) -> Result<Vec<ProjectSource>, ProjectionError> {
    let packages = project
        .packages
        .iter()
        .map(|package| (&package.source_key, package))
        .collect::<BTreeMap<_, _>>();
    let mut sources = Vec::new();
    for input in &project.source_inputs {
        let package = packages
            .get(&input.package_key)
            .ok_or_else(|| ProjectionError::MissingPackage(input.package_key.clone()))?;
        let manifest = package
            .manifest()
            .ok_or_else(|| ProjectionError::MissingPackage(input.package_key.clone()))?;
        if !manifest.language.eq_ignore_ascii_case("papyrus") {
            return Err(ProjectionError::UnsupportedLanguage {
                package: input.package_key.clone(),
                language: manifest.language.clone(),
            });
        }
        let dialect = if manifest.dialect.eq_ignore_ascii_case("skyrim") {
            PapyrusDialect::Skyrim
        } else {
            return Err(ProjectionError::UnsupportedDialect {
                package: input.package_key.clone(),
                dialect: manifest.dialect.clone(),
            });
        };
        sources.push(ProjectSource {
            package_key: input.package_key.clone(),
            canonical_path: input.canonical_path.clone(),
            display_path: input.display_path.clone(),
            script_candidate: input.script_candidate.clone(),
            dialect,
            text: Arc::clone(&input.text),
        });
    }
    Ok(sources)
}

/// Semantic inputs for the providers selected by the project resolver.
pub struct SelectedInputs {
    pub sources: Vec<ProjectSource>,
    pub declarations: Vec<DeclarationBundle>,
    pub user_flags: Vec<String>,
}

/// Projects only resolver-selected providers into the visible semantic namespace.
pub fn selected_inputs(
    project: &LoadedProject,
    metadata: &Metadata,
) -> Result<SelectedInputs, ProjectionError> {
    let package_ids = project
        .packages
        .iter()
        .map(|package| (&package.source_key, &package.source_id))
        .collect::<BTreeMap<_, _>>();
    let selected_sources = metadata
        .scripts
        .iter()
        .filter_map(|selection| {
            selection
                .selected
                .source_path
                .as_ref()
                .map(|path| (selection.selected.package.source.clone(), path.clone()))
        })
        .collect::<BTreeSet<_>>();
    let sources: Vec<_> = sources_from_loaded(project)?
        .into_iter()
        .filter(|source| {
            package_ids
                .get(&source.package_key)
                .is_some_and(|source_id| {
                    selected_sources.contains(&((**source_id).clone(), source.display_path.clone()))
                })
        })
        .collect();
    if sources.len() != selected_sources.len() {
        return Err(ProjectionError::MissingSelectedProvider);
    }
    let mut selected_declarations = BTreeMap::<SourceId, BTreeSet<usize>>::new();
    for selection in &metadata.scripts {
        if let Some(location) = &selection.selected.declaration {
            selected_declarations
                .entry(selection.selected.package.source.clone())
                .or_default()
                .insert(location.script_index);
        }
    }
    let mut bundles = Vec::new();
    for package in &project.packages {
        let Some(bundle) = project.declaration_bundles.get(&package.source_key) else {
            continue;
        };
        let Some(indices) = selected_declarations.get(&package.source_id) else {
            continue;
        };
        let mut bundle = bundle.clone();
        if indices.iter().any(|index| *index >= bundle.scripts.len()) {
            return Err(ProjectionError::MissingSelectedProvider);
        }
        bundle.scripts = bundle
            .scripts
            .into_iter()
            .enumerate()
            .filter_map(|(index, script)| indices.contains(&index).then_some(script))
            .collect();
        bundles.push(bundle);
    }
    let root_flags = project
        .packages
        .iter()
        .find(|package| package.source_key == project.root_key)
        .and_then(|package| package.manifest())
        .map(|manifest| manifest.user_flags.clone())
        .unwrap_or_default();
    Ok(SelectedInputs {
        sources,
        declarations: bundles,
        user_flags: root_flags,
    })
}

/// Invalid projections leave the prior analysis inputs unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    MissingPackage(String),
    MissingSelectedProvider,
    UnsupportedLanguage { package: String, language: String },
    UnsupportedDialect { package: String, dialect: String },
    DuplicateSource { package: String, path: PathBuf },
    FileIdExhausted,
    RevisionExhausted(FileId),
    Input(InputError),
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPackage(package) => {
                write!(f, "no source package for loaded file in {package}")
            }
            Self::MissingSelectedProvider => {
                write!(f, "resolved script provider is missing from loaded inputs")
            }
            Self::UnsupportedLanguage { package, language } => {
                write!(f, "package {package} uses unsupported language {language}")
            }
            Self::UnsupportedDialect { package, dialect } => write!(
                f,
                "package {package} uses unsupported Papyrus dialect {dialect}"
            ),
            Self::DuplicateSource { package, path } => write!(
                f,
                "duplicate source {} in package {package}",
                path.display()
            ),
            Self::FileIdExhausted => write!(f, "analysis file ID space exhausted"),
            Self::RevisionExhausted(file) => {
                write!(f, "analysis revision space exhausted for {file:?}")
            }
            Self::Input(cause) => write!(f, "analysis input update failed: {cause:?}"),
        }
    }
}

impl std::error::Error for ProjectionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveSource {
    source: ProjectSource,
    file: FileId,
    revision: Revision,
}

/// Source identity feedback derived from the shared analysis result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceIssue {
    MissingScriptHeader {
        file: FileId,
    },
    MultipleScriptHeaders {
        file: FileId,
        names: Vec<String>,
    },
    NameMismatch {
        file: FileId,
        candidate: String,
        declared: String,
    },
}

/// A coherent project view with stable source-to-file identity.
pub struct ProjectAnalysisView {
    pub analysis: AnalysisView,
    pub sources: BTreeMap<FileId, ProjectSource>,
    pub issues: Vec<SourceIssue>,
}

impl ProjectAnalysisView {
    pub fn file_for(&self, package_key: &str, canonical_path: &Path) -> Option<FileId> {
        self.sources.iter().find_map(|(&file, source)| {
            (source.package_key == package_key && source.canonical_path == canonical_path)
                .then_some(file)
        })
    }

    /// Returns one coherent set of project and source diagnostics for CLI and IDE adapters.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut diagnostics = self.analysis.project_diagnostics();
        for file in self.analysis.file_ids() {
            if let Some(mut source_diagnostics) = self.analysis.diagnostics(file) {
                diagnostics.append(&mut source_diagnostics);
            }
        }
        for issue in &self.issues {
            let (file, code, message) = match issue {
                SourceIssue::MissingScriptHeader { file } => (
                    *file,
                    "SCRIPT001",
                    "missing ScriptName declaration".to_owned(),
                ),
                SourceIssue::MultipleScriptHeaders { file, names } => (
                    *file,
                    "SCRIPT002",
                    format!("multiple ScriptName declarations: {}", names.join(", ")),
                ),
                SourceIssue::NameMismatch {
                    file,
                    candidate,
                    declared,
                } => (
                    *file,
                    "SCRIPT003",
                    format!("script header {declared} does not match file name {candidate}"),
                ),
            };
            let range = self
                .analysis
                .located_declarations(file)
                .and_then(|items| {
                    items.iter().find_map(|item| {
                        matches!(item.declaration, Declaration::Script { .. }).then_some(item.range)
                    })
                })
                .unwrap_or(TextRange { start: 0, end: 0 });
            diagnostics.push(
                Diagnostic::new(code, Severity::Error, message).at(SourceSpan { file, range }),
            );
        }
        diagnostics.sort_by(|left, right| {
            (
                left.primary.map(|span| span.file),
                left.primary.map(|span| span.range.start),
                &left.code,
            )
                .cmp(&(
                    right.primary.map(|span| span.file),
                    right.primary.map(|span| span.range.start),
                    &right.code,
                ))
        });
        diagnostics
    }
}

/// Holds stable file IDs and projects each resolver revision into one analysis batch.
#[derive(Default)]
pub struct ProjectAnalysis {
    analysis: AnalysisHost,
    active: BTreeMap<SourceKey, ActiveSource>,
    known: BTreeMap<SourceKey, (FileId, Revision)>,
    next_file_id: u32,
}

impl ProjectAnalysis {
    pub fn new() -> Self {
        Self::default()
    }

    #[tracing::instrument(
        name = "project.analysis.sync",
        skip(self, project, metadata),
        fields(phase = "analysis.update")
    )]
    pub fn sync_project(
        &mut self,
        project: &LoadedProject,
        metadata: &Metadata,
    ) -> Result<ProjectAnalysisView, ProjectionError> {
        let selected = selected_inputs(project, metadata)?;
        let mut view = self.sync_sources(selected.sources)?;
        self.analysis
            .set_external_declarations(selected.declarations);
        self.analysis.set_user_flags(selected.user_flags);
        self.analysis
            .set_fill_missing_arguments(metadata.fill_missing_arguments);
        view.analysis = self.analysis.view();
        Ok(view)
    }

    /// Pure source projection; no file access or duplicate parser is involved.
    #[tracing::instrument(
        name = "project.analysis.project",
        skip(self, sources),
        fields(phase = "analysis.update")
    )]
    pub fn sync_sources(
        &mut self,
        sources: impl IntoIterator<Item = ProjectSource>,
    ) -> Result<ProjectAnalysisView, ProjectionError> {
        let mut incoming = BTreeMap::new();
        for source in sources {
            let key = source.key();
            if incoming.insert(key.clone(), source).is_some() {
                return Err(ProjectionError::DuplicateSource {
                    package: key.package_key,
                    path: key.canonical_path,
                });
            }
        }
        let mut known = self.known.clone();
        let mut next_file_id = self.next_file_id;
        let mut next_active = BTreeMap::new();
        let mut edits = Vec::new();
        for (key, source) in incoming {
            let (file, previous_revision) = if let Some(&(file, revision)) = known.get(&key) {
                (file, Some(revision))
            } else {
                let file = FileId(next_file_id);
                next_file_id = next_file_id
                    .checked_add(1)
                    .ok_or(ProjectionError::FileIdExhausted)?;
                (file, None)
            };
            let unchanged = self.active.get(&key).is_some_and(|old| {
                old.source.text == source.text && old.source.dialect == source.dialect
            });
            let revision = if unchanged {
                previous_revision.expect("active source has known revision")
            } else if let Some(previous) = previous_revision {
                Revision(
                    previous
                        .0
                        .checked_add(1)
                        .ok_or(ProjectionError::RevisionExhausted(file))?,
                )
            } else {
                Revision(1)
            };
            if !unchanged {
                edits.push(InputEdit::Upsert {
                    file,
                    revision,
                    text: Arc::clone(&source.text),
                    dialect: source.dialect,
                });
            }
            known.insert(key.clone(), (file, revision));
            next_active.insert(
                key,
                ActiveSource {
                    source,
                    file,
                    revision,
                },
            );
        }
        let next_keys = next_active.keys().cloned().collect::<BTreeSet<_>>();
        for (key, old) in &self.active {
            if !next_keys.contains(key) {
                let revision = Revision(
                    old.revision
                        .0
                        .checked_add(1)
                        .ok_or(ProjectionError::RevisionExhausted(old.file))?,
                );
                edits.push(InputEdit::Remove {
                    file: old.file,
                    revision,
                });
                known.insert(key.clone(), (old.file, revision));
            }
        }
        self.analysis
            .apply_batch(edits)
            .map_err(ProjectionError::Input)?;
        self.known = known;
        self.next_file_id = next_file_id;
        self.active = next_active;
        let analysis = self.analysis.view();
        let mut source_map = BTreeMap::new();
        let mut issues = Vec::new();
        for source in self.active.values() {
            source_map.insert(source.file, source.source.clone());
            let headers = analysis
                .declarations(source.file)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|declaration| match declaration {
                            Declaration::Script { name, .. } => Some(name.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            match headers.as_slice() {
                [] => issues.push(SourceIssue::MissingScriptHeader { file: source.file }),
                [declared] if !declared.eq_ignore_ascii_case(&source.source.script_candidate) => {
                    issues.push(SourceIssue::NameMismatch {
                        file: source.file,
                        candidate: source.source.script_candidate.clone(),
                        declared: declared.clone(),
                    })
                }
                [_, _, ..] => issues.push(SourceIssue::MultipleScriptHeaders {
                    file: source.file,
                    names: headers,
                }),
                _ => {}
            }
        }
        tracing::info!(
            files = source_map.len(),
            issues = issues.len(),
            generation = analysis.generation(),
            "project sources projected into analysis"
        );
        Ok(ProjectAnalysisView {
            analysis,
            sources: source_map,
            issues,
        })
    }
}

#[cfg(test)]
mod tests;
