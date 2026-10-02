//! Incremental syntax inputs and consistent, owned analysis views.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use folio_diagnostics::Diagnostic;
use folio_format_declarations::DeclarationBundle;
use folio_hir::{Script, Type};
use folio_papyrus::{Declaration, LocatedDeclaration, PapyrusDialect, Parse};
use folio_source::{FileId, LineIndex, Revision, SourceSpan};
use salsa::Setter as _;

mod intrinsics;
mod semantic;
pub use intrinsics::{IntrinsicSignature, intrinsic_signature};

/// Visible semantic name with an optional real source definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionCandidate {
    pub symbol: folio_hir::Symbol,
    pub ty: Type,
    pub definition: Option<SourceSpan>,
}

/// An analysis request was abandoned before its result was published to the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisCancelled;

#[salsa::input]
struct SourceInput {
    #[returns(clone)]
    text: Arc<str>,
    #[returns(copy)]
    dialect: PapyrusDialect,
}

#[salsa::tracked]
fn parsed(db: &dyn salsa::Database, input: SourceInput) -> Arc<Parse> {
    let text = input.text(db);
    let parsed = folio_papyrus::parse(&text, input.dialect(db));
    tracing::trace!(
        errors = parsed.errors.len(),
        "analysis parse query executed"
    );
    Arc::new(parsed)
}

#[salsa::tracked]
fn located_declarations(
    db: &dyn salsa::Database,
    input: SourceInput,
) -> Arc<Vec<LocatedDeclaration>> {
    let parse = parsed(db, input);
    Arc::new(folio_papyrus::declarations(parse))
}

#[salsa::tracked]
fn declaration_summary(db: &dyn salsa::Database, input: SourceInput) -> Arc<Vec<Declaration>> {
    let declarations = located_declarations(db, input)
        .iter()
        .map(|item| item.declaration.clone())
        .collect::<Vec<_>>();
    tracing::trace!(
        count = declarations.len(),
        "analysis declaration query executed"
    );
    Arc::new(declarations)
}

struct ActiveFile {
    input: SourceInput,
    revision: Revision,
}

/// A full replacement or removal of one host-owned source input.
#[derive(Debug, Clone)]
pub enum InputEdit {
    Upsert {
        file: FileId,
        revision: Revision,
        text: Arc<str>,
        dialect: PapyrusDialect,
    },
    Remove {
        file: FileId,
        revision: Revision,
    },
}

impl InputEdit {
    fn file(&self) -> FileId {
        match *self {
            Self::Upsert { file, .. } | Self::Remove { file, .. } => file,
        }
    }

    fn revision(&self) -> Revision {
        match *self {
            Self::Upsert { revision, .. } | Self::Remove { revision, .. } => revision,
        }
    }
}

/// Invalid batches leave both active inputs and the Salsa database unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputError {
    DuplicateFile(FileId),
    UnknownFile(FileId),
    StaleRevision {
        file: FileId,
        current: Revision,
        proposed: Revision,
    },
    TextTooLarge(FileId),
}

/// Owns editable inputs; Salsa handles never cross this API boundary.
#[derive(Default)]
pub struct AnalysisHost {
    db: salsa::DatabaseImpl,
    active: BTreeMap<FileId, ActiveFile>,
    last_revisions: BTreeMap<FileId, Revision>,
    generation: u64,
    external_declarations: Arc<Vec<DeclarationBundle>>,
    user_flags: Arc<Vec<folio_profiles::UserFlag>>,
    fill_missing_arguments: bool,
    view: OnceLock<AnalysisView>,
}

impl AnalysisHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the selected external API snapshots independently of editable files.
    pub fn set_external_declarations(&mut self, bundles: Vec<DeclarationBundle>) {
        if *self.external_declarations == bundles {
            return;
        }
        tracing::info!(
            bundles = bundles.len(),
            "replaced analysis external declarations"
        );
        self.external_declarations = Arc::new(bundles);
        self.generation += 1;
        self.view.take();
    }

    /// Replaces available project-defined flags; source references remain explicit.
    pub fn set_user_flags(&mut self, flags: Vec<folio_profiles::UserFlag>) {
        if *self.user_flags == flags {
            return;
        }
        tracing::info!(flags = flags.len(), "replaced analysis user flags");
        self.user_flags = Arc::new(flags);
        self.generation += 1;
        self.view.take();
    }

    /// Updates the project call policy used by all later analysis views.
    pub fn set_fill_missing_arguments(&mut self, enabled: bool) {
        if self.fill_missing_arguments != enabled {
            tracing::info!(enabled, "updated missing argument policy");
            self.fill_missing_arguments = enabled;
            self.generation += 1;
            self.view.take();
        }
    }

    pub fn upsert(
        &mut self,
        file: FileId,
        revision: Revision,
        text: Arc<str>,
        dialect: PapyrusDialect,
    ) -> Result<(), InputError> {
        self.apply_batch([InputEdit::Upsert {
            file,
            revision,
            text,
            dialect,
        }])
    }

    pub fn remove(&mut self, file: FileId, revision: Revision) -> Result<(), InputError> {
        self.apply_batch([InputEdit::Remove { file, revision }])
    }

    /// Validates a complete project update before changing any input.
    #[tracing::instrument(skip(self, edits), fields(generation = self.generation, phase = "analysis.update"))]
    pub fn apply_batch(
        &mut self,
        edits: impl IntoIterator<Item = InputEdit>,
    ) -> Result<(), InputError> {
        let edits = edits.into_iter().collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        for edit in &edits {
            let file = edit.file();
            if !seen.insert(file) {
                return Err(InputError::DuplicateFile(file));
            }
            if let Some(current) = self.last_revisions.get(&file)
                && edit.revision() <= *current
            {
                return Err(InputError::StaleRevision {
                    file,
                    current: *current,
                    proposed: edit.revision(),
                });
            }
            match edit {
                InputEdit::Remove { .. } if !self.active.contains_key(&file) => {
                    return Err(InputError::UnknownFile(file));
                }
                InputEdit::Upsert { text, .. } if text.len() > u32::MAX as usize => {
                    return Err(InputError::TextTooLarge(file));
                }
                _ => {}
            }
        }
        if edits.is_empty() {
            return Ok(());
        }
        tracing::debug!(count = edits.len(), "applying analysis input batch");
        for edit in edits {
            let file = edit.file();
            let revision = edit.revision();
            match edit {
                InputEdit::Upsert { text, dialect, .. } => {
                    if let Some(active) = self.active.get_mut(&file) {
                        if active.input.text(&self.db).as_ref() != text.as_ref() {
                            active.input.set_text(&mut self.db).to(text);
                        }
                        if active.input.dialect(&self.db) != dialect {
                            active.input.set_dialect(&mut self.db).to(dialect);
                        }
                        active.revision = revision;
                        tracing::debug!(?file, ?revision, "replaced analysis source");
                    } else {
                        let input = SourceInput::new(&self.db, text, dialect);
                        self.active.insert(file, ActiveFile { input, revision });
                        tracing::debug!(?file, ?revision, "added analysis source");
                    }
                }
                InputEdit::Remove { .. } => {
                    self.active.remove(&file);
                    tracing::debug!(?file, ?revision, "removed analysis source");
                }
            }
            self.last_revisions.insert(file, revision);
        }
        self.generation += 1;
        self.view.take();
        tracing::info!(
            generation = self.generation,
            files = self.active.len(),
            "analysis inputs updated"
        );
        Ok(())
    }

    /// Materializes one coherent result set; it can outlive later host edits.
    #[tracing::instrument(skip(self), fields(generation = self.generation, phase = "analysis.query"))]
    pub fn view(&self) -> AnalysisView {
        self.view.get_or_init(|| self.materialize_view()).clone()
    }

    /// Retains complete semantic facts until an actual host input changes.
    fn materialize_view(&self) -> AnalysisView {
        let mut files = BTreeMap::new();
        for (&file, active) in &self.active {
            let parse = Arc::clone(parsed(&self.db, active.input));
            let declarations = Arc::clone(declaration_summary(&self.db, active.input));
            let located = Arc::clone(located_declarations(&self.db, active.input));
            files.insert(
                file,
                ViewFile {
                    revision: active.revision,
                    dialect: active.input.dialect(&self.db),
                    text: active.input.text(&self.db),
                    parse,
                    declarations,
                    located,
                },
            );
        }
        tracing::debug!(
            generation = self.generation,
            files = files.len(),
            "materialized analysis view"
        );
        AnalysisView {
            generation: self.generation,
            files: Arc::new(files),
            external_declarations: Arc::clone(&self.external_declarations),
            user_flags: Arc::clone(&self.user_flags),
            fill_missing_arguments: self.fill_missing_arguments,
            semantic: Arc::new(OnceLock::new()),
        }
    }
}

struct ViewFile {
    revision: Revision,
    dialect: PapyrusDialect,
    text: Arc<str>,
    parse: Arc<Parse>,
    declarations: Arc<Vec<Declaration>>,
    located: Arc<Vec<LocatedDeclaration>>,
}

/// An immutable view of files and semantic facts at one update generation.
#[derive(Clone)]
pub struct AnalysisView {
    generation: u64,
    files: Arc<BTreeMap<FileId, ViewFile>>,
    external_declarations: Arc<Vec<DeclarationBundle>>,
    user_flags: Arc<Vec<folio_profiles::UserFlag>>,
    fill_missing_arguments: bool,
    semantic: Arc<OnceLock<semantic::Analysis>>,
}

impl AnalysisView {
    /// Selected immutable API snapshots, including documentation and provenance.
    pub fn external_declarations(&self) -> &[DeclarationBundle] {
        &self.external_declarations
    }

    pub fn user_flags(&self) -> &[folio_profiles::UserFlag] {
        &self.user_flags
    }
    pub fn fill_missing_arguments(&self) -> bool {
        self.fill_missing_arguments
    }

    /// Enumerates names through the same selected world and lookup precedence as checking.
    pub fn completion_candidates(
        &self,
        file: FileId,
        byte: usize,
        receiver: Option<&Type>,
        global: bool,
    ) -> Vec<CompletionCandidate> {
        semantic::completion_candidates(self, file, byte, receiver, global)
    }
    fn semantic(&self) -> &semantic::Analysis {
        self.semantic.get_or_init(|| semantic::analyze(self))
    }

    /// Computes semantic facts cooperatively and publishes only a complete result.
    ///
    /// Callers may then use `type_at`, `definition`, or `hir` against the warmed view.
    /// The predicate may be checked many times and should be inexpensive.
    pub fn try_warm_semantics(
        &self,
        cancelled: impl Fn() -> bool,
    ) -> Result<(), AnalysisCancelled> {
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        if self.semantic.get().is_some() {
            return Ok(());
        }
        let analysis = semantic::analyze_with_cancel(self, &cancelled)?;
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        // Another reader may have completed the same immutable view first.
        let _ = self.semantic.set(analysis);
        Ok(())
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn file_ids(&self) -> impl Iterator<Item = FileId> + '_ {
        self.files.keys().copied()
    }
    pub fn revision(&self, file: FileId) -> Option<Revision> {
        self.files.get(&file).map(|item| item.revision)
    }
    pub fn dialect(&self, file: FileId) -> Option<PapyrusDialect> {
        self.files.get(&file).map(|item| item.dialect)
    }
    pub fn text(&self, file: FileId) -> Option<&str> {
        self.files.get(&file).map(|item| item.text.as_ref())
    }
    pub fn line_index(&self, file: FileId) -> Option<LineIndex> {
        self.text(file).map(LineIndex::new)
    }
    pub fn parse(&self, file: FileId) -> Option<Arc<Parse>> {
        self.files.get(&file).map(|item| Arc::clone(&item.parse))
    }
    pub fn declarations(&self, file: FileId) -> Option<Arc<Vec<Declaration>>> {
        self.files
            .get(&file)
            .map(|item| Arc::clone(&item.declarations))
    }
    pub fn located_declarations(&self, file: FileId) -> Option<Arc<Vec<LocatedDeclaration>>> {
        self.files.get(&file).map(|item| Arc::clone(&item.located))
    }

    /// Reports syntax and semantic problems for one editable source file.
    pub fn diagnostics(&self, file: FileId) -> Option<Vec<Diagnostic>> {
        self.files.get(&file)?;
        Some(self.semantic().file(file)?.diagnostics.clone())
    }

    /// Reports errors in external declarations without pretending they have a source FileId.
    pub fn project_diagnostics(&self) -> Vec<Diagnostic> {
        self.semantic().project_diagnostics.clone()
    }

    /// Returns a partial typed model even when sibling syntax or names are invalid.
    pub fn hir(&self, file: FileId) -> Option<Arc<Script>> {
        Some(Arc::new(self.semantic().file(file)?.script.clone()))
    }

    /// Finds the type of the smallest expression covering a UTF-8 byte offset.
    pub fn type_at(&self, file: FileId, offset: usize) -> Option<Type> {
        let analysis = self.semantic();
        analysis
            .file(file)?
            .script
            .expression_at(offset)
            .map(|fact| fact.ty.clone())
    }

    /// Finds a source definition for a bound name; external declarations have no local span.
    pub fn definition(&self, file: FileId, offset: usize) -> Option<SourceSpan> {
        let analysis = self.semantic();
        let script = &analysis.file(file)?.script;
        script
            .expressions
            .iter()
            .filter_map(|fact| fact.binding.as_ref())
            .find(|binding| {
                binding.name.span.range.start <= offset && offset < binding.name.span.range.end
            })
            .and_then(|binding| binding.definition)
    }
}

#[cfg(test)]
mod tests;
