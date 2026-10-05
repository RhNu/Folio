//! Disposable editor indices bound to one immutable semantic project snapshot.
use std::{
    collections::{BTreeMap, HashMap},
    ops::Deref,
    sync::{Arc, OnceLock},
};

use folio_build::ProjectAnalysisView;
use folio_hir::Symbol;
use folio_source::{FileId, SourceSpan};

use crate::navigation::SymbolOccurrence;

mod hierarchy;
pub(crate) use hierarchy::HierarchyIndex;

/// A case-insensitive semantic key. Local declaration offsets remain part of identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SymbolKey(Symbol);

impl SymbolKey {
    pub(crate) fn new(symbol: &Symbol) -> Self {
        fn normalize(symbol: &mut Symbol) {
            match symbol {
                Symbol::Script(name)
                | Symbol::ParentReceiver { script: name }
                | Symbol::Intrinsic { name } => name.make_ascii_lowercase(),
                Symbol::Member { script, name } => {
                    script.make_ascii_lowercase();
                    name.make_ascii_lowercase();
                },
                Symbol::StateMember {
                    script,
                    state,
                    name,
                } => {
                    script.make_ascii_lowercase();
                    state.make_ascii_lowercase();
                    name.make_ascii_lowercase();
                },
                Symbol::PropertyAccessor {
                    script,
                    property,
                    name,
                } => {
                    script.make_ascii_lowercase();
                    property.make_ascii_lowercase();
                    name.make_ascii_lowercase();
                },
                Symbol::Parameter { owner, name } | Symbol::Local { owner, name, .. } => {
                    normalize(owner);
                    name.make_ascii_lowercase();
                },
            }
        }
        let mut symbol = symbol.clone();
        normalize(&mut symbol);
        Self(symbol)
    }
}

#[derive(Default)]
struct DeclarationIndex {
    symbols: HashMap<SymbolKey, Vec<SourceSpan>>,
    scripts: HashMap<String, SourceSpan>,
}

struct QueryCache {
    occurrences: BTreeMap<FileId, OnceLock<Vec<SymbolOccurrence>>>,
    declarations: OnceLock<DeclarationIndex>,
    references: OnceLock<HashMap<SymbolKey, Vec<SymbolOccurrence>>>,
    hierarchy: OnceLock<HierarchyIndex>,
}

/// Shares lazy query indices across requests over precisely the same analysis inputs.
/// A request cancellation predicate belongs to its clone, never to the shared cache.
#[derive(Clone)]
pub struct IdeSnapshot {
    project: Arc<ProjectAnalysisView>,
    cache: Arc<QueryCache>,
    cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl IdeSnapshot {
    pub fn new(project: Arc<ProjectAnalysisView>) -> Self {
        let occurrences = project
            .analysis
            .file_ids()
            .map(|file| (file, OnceLock::new()))
            .collect();
        Self {
            project,
            cache: Arc::new(QueryCache {
                occurrences,
                declarations: OnceLock::new(),
                references: OnceLock::new(),
                hierarchy: OnceLock::new(),
            }),
            cancelled: Arc::new(|| false),
        }
    }

    /// Adds cooperative cancellation while keeping every completed immutable index shared.
    #[must_use]
    pub fn with_cancellation(&self, cancelled: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self {
            cancelled,
            ..self.clone()
        }
    }

    pub fn is_cancelled(&self) -> bool { (self.cancelled)() }

    fn declarations(&self) -> Option<&DeclarationIndex> {
        cached(self, &self.cache.declarations, || {
            let mut index = DeclarationIndex::default();
            for file in self.analysis.file_ids() {
                if self.is_cancelled() {
                    return None;
                }
                let Some(script) = self.analysis.hir(file) else {
                    continue;
                };
                if let Some(name) = &script.name {
                    index
                        .scripts
                        .entry(name.text.to_ascii_lowercase())
                        .or_insert(name.span);
                    index
                        .symbols
                        .entry(SymbolKey::new(&Symbol::Script(name.text.clone())))
                        .or_default()
                        .push(name.span);
                }
                for item in &script.declarations {
                    index
                        .symbols
                        .entry(SymbolKey::new(&item.symbol))
                        .or_default()
                        .push(item.span);
                }
            }
            for spans in index.symbols.values_mut() {
                spans.sort_by_key(|span| (span.file, span.range.start));
                spans.dedup();
            }
            tracing::debug!(
                symbols = index.symbols.len(),
                "editor declaration index ready"
            );
            Some(index)
        })
    }

    pub(crate) fn definitions(&self, symbol: &Symbol) -> Option<&[SourceSpan]> {
        let index = self.declarations()?;
        Some(
            index
                .symbols
                .get(&SymbolKey::new(symbol))
                .map_or(&[], Vec::as_slice),
        )
    }

    pub(crate) fn script_definition(&self, name: &str) -> Option<SourceSpan> {
        self.declarations()?
            .scripts
            .get(&name.to_ascii_lowercase())
            .copied()
    }

    pub(crate) fn occurrences(&self, file: FileId) -> Option<&[SymbolOccurrence]> {
        let cell = self.cache.occurrences.get(&file)?;
        cached(self, cell, || {
            crate::navigation::collect_occurrences(self, file)
        })
        .map(Vec::as_slice)
    }

    pub(crate) fn references(&self, symbol: &Symbol) -> Option<&[SymbolOccurrence]> {
        let index = cached(self, &self.cache.references, || {
            let mut index = HashMap::<SymbolKey, Vec<SymbolOccurrence>>::new();
            for file in self.analysis.file_ids() {
                if self.is_cancelled() {
                    return None;
                }
                for occurrence in self.occurrences(file)? {
                    index
                        .entry(SymbolKey::new(&occurrence.symbol))
                        .or_default()
                        .push(occurrence.clone());
                }
            }
            tracing::debug!(symbols = index.len(), "editor reference index ready");
            Some(index)
        })?;
        Some(
            index
                .get(&SymbolKey::new(symbol))
                .map_or(&[], Vec::as_slice),
        )
    }

    pub(crate) fn hierarchy(&self) -> Option<&HierarchyIndex> {
        cached(self, &self.cache.hierarchy, || HierarchyIndex::build(self))
    }
}

/// Build outside the cell: interruption never commits an incomplete index, and
/// simultaneous requests may finish independently without holding a shared mutex.
fn cached<'a, T>(
    view: &IdeSnapshot,
    cell: &'a OnceLock<T>,
    build: impl FnOnce() -> Option<T>,
) -> Option<&'a T> {
    if view.is_cancelled() {
        return None;
    }
    if let Some(value) = cell.get() {
        return Some(value);
    }
    let value = build()?;
    if view.is_cancelled() {
        return None;
    }
    drop(cell.set(value));
    cell.get()
}

impl From<ProjectAnalysisView> for IdeSnapshot {
    fn from(project: ProjectAnalysisView) -> Self { Self::new(Arc::new(project)) }
}

impl Deref for IdeSnapshot {
    type Target = ProjectAnalysisView;

    fn deref(&self) -> &Self::Target { &self.project }
}

#[cfg(test)]
mod tests;
