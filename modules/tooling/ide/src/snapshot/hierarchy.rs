//! Script ancestry is indexed separately from callable members, so script hover
//! never enumerates the SDK's complete member population.
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

use folio_format_declarations::MemberData;
use folio_hir::{MemberKind, Symbol};
use folio_source::FileId;

use super::{IdeSnapshot, SymbolKey};

pub(crate) struct HierarchyIndex {
    sources: HashMap<String, FileId>,
    parents: HashMap<String, String>,
    children: HashMap<String, Vec<Symbol>>,
    implementations: Mutex<HashMap<SymbolKey, Arc<Vec<Symbol>>>>,
}

impl HierarchyIndex {
    pub(crate) fn build(view: &IdeSnapshot) -> Option<Self> {
        let mut scripts = HashMap::<String, (String, Option<String>)>::new();
        let mut sources = HashMap::new();
        for file in view.analysis.file_ids() {
            if view.is_cancelled() {
                return None;
            }
            let Some(script) = view.analysis.hir(file) else {
                continue;
            };
            if let Some(name) = &script.name {
                sources
                    .entry(name.text.to_ascii_lowercase())
                    .or_insert(file);
                scripts.entry(name.text.to_ascii_lowercase()).or_insert((
                    name.text.clone(),
                    script.parent.as_ref().map(|item| item.text.clone()),
                ));
            }
        }
        for script in view
            .analysis
            .external_declarations()
            .iter()
            .flat_map(|bundle| &bundle.scripts)
        {
            if view.is_cancelled() {
                return None;
            }
            let key = script.name.to_ascii_lowercase();
            if scripts.contains_key(&key) {
                continue;
            }
            let selected = view.analysis.external_script(&script.name)?;
            scripts.insert(key, (selected.name.clone(), selected.parent.clone()));
        }
        let mut parents = HashMap::new();
        let mut children = HashMap::<String, Vec<Symbol>>::new();
        for (key, (name, parent)) in scripts {
            if let Some(parent) = parent {
                let parent = parent.to_ascii_lowercase();
                parents.insert(key, parent.clone());
                children
                    .entry(parent)
                    .or_default()
                    .push(Symbol::Script(name));
            }
        }
        tracing::debug!(scripts = parents.len(), "editor ancestry index ready");
        Some(Self {
            sources,
            parents,
            children,
            implementations: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn derives(&self, child: &str, ancestor: &str) -> bool {
        let ancestor = ancestor.to_ascii_lowercase();
        let mut current = self.parents.get(&child.to_ascii_lowercase());
        let mut seen = BTreeSet::new();
        while let Some(name) = current {
            if *name == ancestor {
                return true;
            }
            if !seen.insert(name) {
                break;
            }
            current = self.parents.get(name);
        }
        false
    }

    fn descendants(&self, view: &IdeSnapshot, ancestor: &str) -> Option<Vec<Symbol>> {
        let mut pending = vec![ancestor.to_ascii_lowercase()];
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        while let Some(name) = pending.pop() {
            if view.is_cancelled() {
                return None;
            }
            if !seen.insert(name.clone()) {
                continue;
            }
            if let Some(children) = self.children.get(&name) {
                for child in children {
                    if let Symbol::Script(name) = child {
                        pending.push(name.to_ascii_lowercase());
                    }
                    result.push(child.clone());
                }
            }
        }
        Some(result)
    }

    /// Find compatible overrides in selected source and external descendant scripts.
    fn member_implementations(
        &self,
        view: &IdeSnapshot,
        target: &Symbol,
        owner: &str,
        name: &str,
    ) -> Option<Vec<Symbol>> {
        let Some(kind @ (_, false)) = callable_kind(view, target) else {
            return Some(Vec::new());
        };
        let mut eligible = self
            .descendants(view, owner)?
            .into_iter()
            .filter_map(|symbol| {
                if let Symbol::Script(name) = symbol {
                    Some(name.to_ascii_lowercase())
                } else {
                    None
                }
            })
            .collect::<BTreeSet<_>>();
        eligible.insert(owner.to_ascii_lowercase());
        let mut candidates = Vec::new();
        for child in &eligible {
            if view.is_cancelled() {
                return None;
            }
            if let Some(file) = self.sources.get(child) {
                let Some(script) = view.analysis.hir(*file) else {
                    continue;
                };
                candidates.extend(
                    script
                        .members
                        .iter()
                        .filter(|item| {
                            crate::navigation::name(&item.symbol).eq_ignore_ascii_case(name)
                        })
                        .map(|item| item.symbol.clone()),
                );
                continue;
            }
            let Some(script) = view.analysis.external_script(child) else {
                continue;
            };
            candidates.extend(
                script
                    .members
                    .iter()
                    .filter(|item| item.name.eq_ignore_ascii_case(name))
                    .map(|item| Symbol::Member {
                        script: script.name.clone(),
                        name: item.name.clone(),
                    }),
            );
            for state in &script.states {
                if view.is_cancelled() {
                    return None;
                }
                candidates.extend(
                    state
                        .members
                        .iter()
                        .filter(|item| item.name.eq_ignore_ascii_case(name))
                        .map(|item| Symbol::StateMember {
                            script: script.name.clone(),
                            state: state.name.clone(),
                            name: item.name.clone(),
                        }),
                );
            }
        }
        candidates.retain(|candidate| {
            !crate::navigation::same(target, candidate)
                && callable_kind(view, candidate) == Some(kind)
        });
        Some(candidates)
    }

    pub(crate) fn implementations(
        &self,
        view: &IdeSnapshot,
        target: &Symbol,
    ) -> Option<Arc<Vec<Symbol>>> {
        let key = SymbolKey::new(target);
        if let Some(cached) = self
            .implementations
            .lock()
            .expect("validated semantic index")
            .get(&key)
        {
            return Some(Arc::clone(cached));
        }
        if view.is_cancelled() {
            return None;
        }
        let mut result = match target {
            Symbol::Script(name) => self.descendants(view, name)?,
            Symbol::Member {
                script: owner,
                name,
            }
            | Symbol::StateMember {
                script: owner,
                name,
                ..
            } => self.member_implementations(view, target, owner, name)?,
            _ => Vec::new(),
        };
        result.sort_by_key(|symbol| format!("{symbol:?}").to_ascii_lowercase());
        result.dedup_by(|a, b| crate::navigation::same(a, b));
        if view.is_cancelled() {
            return None;
        }
        tracing::debug!(symbol = ?target, implementations = result.len(), "editor implementations indexed");
        let result = Arc::new(result);
        let mut cache = self
            .implementations
            .lock()
            .expect("validated semantic index");
        Some(Arc::clone(cache.entry(key).or_insert(result)))
    }
}

fn callable_kind(view: &IdeSnapshot, symbol: &Symbol) -> Option<(bool, bool)> {
    // Definition lookup narrows a source member query to its owning file.
    for definition in view.definitions(symbol)? {
        let script = view.analysis.hir(definition.file)?;
        if let Some(member) = script
            .members
            .iter()
            .find(|item| crate::navigation::same(&item.symbol, symbol))
        {
            return match member.kind {
                MemberKind::Function { event, global, .. } => Some((event, global)),
                _ => None,
            };
        }
    }
    match crate::presentation::external_member(view, symbol)?.data {
        MemberData::Function { global, .. } => Some((false, global)),
        MemberData::Event { .. } => Some((true, false)),
        _ => None,
    }
}
