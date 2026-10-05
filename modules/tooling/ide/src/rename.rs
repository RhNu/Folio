//! Project rename is accepted only after rechecking the complete edited semantic input.
use folio_analysis::AnalysisHost;
use folio_hir::{MemberKind, Symbol};
use folio_source::{FileId, Revision, SourceSpan};

use crate::{
    IdeSnapshot,
    navigation::{self, SymbolOccurrence, name, same},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RenameError {
    NoSymbol,
    UnsupportedSymbol,
    IncompleteAnalysis,
    InvalidName,
    Collision,
    UnverifiableEdit,
}

impl std::fmt::Display for RenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoSymbol => "No resolved symbol at this position",
            Self::UnsupportedSymbol => {
                "Only verifiable project members, local variables, and parameters can be renamed"
            },
            Self::IncompleteAnalysis => "Fix project analysis errors before renaming",
            Self::InvalidName => {
                "The new name must be a Papyrus identifier and cannot be a keyword"
            },
            Self::Collision => "The new name conflicts with a visible declaration",
            Self::UnverifiableEdit => "The edited project does not preserve semantic bindings",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenameTarget {
    pub symbol: Symbol,
    pub span: SourceSpan,
    pub placeholder: String,
    pub definition: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenameEdit {
    pub span: SourceSpan,
    pub replacement: String,
}

/// Select a project symbol whose definition and rename safety can be verified.
///
/// # Errors
/// Returns an error when analysis is incomplete or the selected symbol cannot be safely renamed.
pub fn prepare_rename(
    view: &IdeSnapshot,
    file: FileId,
    byte: usize,
) -> Result<RenameTarget, RenameError> {
    if !view.issues.is_empty()
        || view
            .analysis
            .project_diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.severity == folio_diagnostics::Severity::Error)
        || view.analysis.file_ids().any(|file| {
            view.analysis.diagnostics(file).is_none_or(|items| {
                items
                    .iter()
                    .any(|diagnostic| diagnostic.severity == folio_diagnostics::Severity::Error)
            })
        })
    {
        return Err(RenameError::IncompleteAnalysis);
    }
    let target = crate::symbol_at(view, file, byte).ok_or(RenameError::NoSymbol)?;
    let definition = target.definition.ok_or(RenameError::UnsupportedSymbol)?;
    let script = view
        .analysis
        .hir(definition.file)
        .ok_or(RenameError::UnsupportedSymbol)?;
    match &target.symbol {
        Symbol::Local { .. } => {},
        Symbol::Parameter { owner, .. } => {
            let member = crate::symbols::find_member(&script, owner)
                .ok_or(RenameError::UnsupportedSymbol)?;
            if matches!(
                member.kind,
                MemberKind::Function { native: true, .. }
                    | MemberKind::Function { event: true, .. }
            ) {
                return Err(RenameError::UnsupportedSymbol);
            }
        },
        Symbol::Member {
            script: owner,
            name,
        } => {
            let member = script
                .members
                .iter()
                .find(|item| same(&item.symbol, &target.symbol))
                .ok_or(RenameError::UnsupportedSymbol)?;
            if matches!(
                member.kind,
                MemberKind::Function { native: true, .. }
                    | MemberKind::Function { event: true, .. }
            ) || !crate::implementation_symbols(view, &target.symbol).is_empty()
            {
                return Err(RenameError::UnsupportedSymbol);
            }
            if script
                .external_members
                .iter()
                .any(|item| navigation::name(&item.symbol).eq_ignore_ascii_case(name))
            {
                return Err(RenameError::UnsupportedSymbol);
            }
            if view
                .analysis
                .external_declarations()
                .iter()
                .flat_map(|bundle| &bundle.scripts)
                .any(|script| {
                    (navigation::derives(view, owner, &script.name)
                        || navigation::derives(view, &script.name, owner))
                        && script
                            .members
                            .iter()
                            .any(|member| member.name.eq_ignore_ascii_case(name))
                })
            {
                return Err(RenameError::UnsupportedSymbol);
            }
        },
        _ => return Err(RenameError::UnsupportedSymbol),
    }
    tracing::debug!(?file,byte,symbol=?target.symbol,"rename target validated");
    Ok(RenameTarget {
        placeholder: name(&target.symbol).into(),
        symbol: target.symbol,
        span: target.span,
        definition,
    })
}

fn valid_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && !folio_profiles::is_skyrim_keyword(value)
}

/// Rename the selected symbol only when complete reanalysis preserves every binding.
///
/// # Errors
/// Returns an error for an invalid name, collisions, incomplete analysis, or edits whose bindings cannot be verified.
pub fn rename(
    view: &IdeSnapshot,
    file: FileId,
    byte: usize,
    new_name: &str,
) -> Result<Vec<RenameEdit>, RenameError> {
    let target = prepare_rename(view, file, byte)?;
    if !valid_name(new_name) {
        return Err(RenameError::InvalidName);
    }
    let occurrence = SymbolOccurrence {
        symbol: target.symbol.clone(),
        span: target.span,
        definition: Some(target.definition),
    };
    // Local collisions depend on lexical scope. The edited-project analysis and
    // occurrence checks below reject overlapping declarations and captured uses.
    if let Symbol::Member { script: owner, .. } = &target.symbol {
        for file in view.analysis.file_ids() {
            let script = view
                .analysis
                .hir(file)
                .ok_or(RenameError::IncompleteAnalysis)?;
            let Some(script_name) = &script.name else {
                continue;
            };
            if (script_name.text.eq_ignore_ascii_case(owner)
                || navigation::derives(view, &script_name.text, owner)
                || navigation::derives(view, owner, &script_name.text))
                && script
                    .members
                    .iter()
                    .chain(&script.external_members)
                    .any(|item| {
                        !same(&item.symbol, &target.symbol)
                            && name(&item.symbol).eq_ignore_ascii_case(new_name)
                    })
            {
                return Err(RenameError::Collision);
            }
        }
    }
    let mut edits = navigation::references_to(view, &occurrence, true)
        .into_iter()
        .map(|span| RenameEdit {
            span,
            replacement: new_name.into(),
        })
        .collect::<Vec<_>>();
    if edits.is_empty() {
        return Err(RenameError::UnverifiableEdit);
    }
    edits.sort_by_key(|edit| (edit.span.file, edit.span.range.start));
    let edited = edited_project(view, &edits, new_name)?;
    // Every original occurrence must retain its selected declaration, including unrelated names.
    for file in view.analysis.file_ids() {
        for original in navigation::occurrences(view, file) {
            let start = map_offset(file, original.span.range.start, &edits);
            let actual =
                crate::symbol_at(&edited, file, start).ok_or(RenameError::UnverifiableEdit)?;
            let mut expected = if matches!(
                target.symbol,
                Symbol::Local { .. } | Symbol::Parameter { .. }
            ) && original.definition != Some(target.definition)
            {
                original.symbol.clone()
            } else {
                renamed_symbol(&original.symbol, &target.symbol, new_name)
            };
            // Local identity is its declaration offset; edits before it move that identity.
            if let Symbol::Local { identity, .. } = &mut expected {
                *identity = map_offset(
                    original.definition.map_or(file, |span| span.file),
                    *identity,
                    &edits,
                );
            }
            let expected_definition = original.definition.map(|span| SourceSpan {
                file: span.file,
                range: folio_source::TextRange {
                    start: map_offset(span.file, span.range.start, &edits),
                    end: map_offset(span.file, span.range.end, &edits),
                },
            });
            if !same(&actual.symbol, &expected) || actual.definition != expected_definition {
                return Err(RenameError::UnverifiableEdit);
            }
        }
    }
    tracing::info!(edits=edits.len(),symbol=?target.symbol,"verified project rename");
    Ok(edits)
}

/// Reanalyze complete edited inputs before checking occurrence identities.
fn edited_project(
    view: &IdeSnapshot,
    edits: &[RenameEdit],
    new_name: &str,
) -> Result<IdeSnapshot, RenameError> {
    let mut host = AnalysisHost::new();
    host.set_external_declarations(view.analysis.external_declarations().to_vec());
    host.set_user_flags(view.analysis.user_flags().to_vec());
    host.set_fill_missing_arguments(view.analysis.fill_missing_arguments());
    for file in view.analysis.file_ids() {
        let mut text = view
            .analysis
            .text(file)
            .ok_or(RenameError::IncompleteAnalysis)?
            .to_owned();
        for edit in edits.iter().rev().filter(|edit| edit.span.file == file) {
            text.replace_range(edit.span.range.start..edit.span.range.end, new_name);
        }
        host.upsert(
            file,
            Revision(1),
            text.into(),
            view.analysis
                .dialect(file)
                .ok_or(RenameError::IncompleteAnalysis)?,
        )
        .map_err(|cause| {
            tracing::debug!(?cause, "rename input rejected");
            RenameError::UnverifiableEdit
        })?;
    }
    let analysis = host.view();
    if analysis
        .project_diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.severity == folio_diagnostics::Severity::Error)
        || analysis.file_ids().any(|file| {
            analysis.diagnostics(file).is_none_or(|items| {
                items
                    .iter()
                    .any(|diagnostic| diagnostic.severity == folio_diagnostics::Severity::Error)
            })
        })
    {
        return Err(RenameError::Collision);
    }
    Ok(IdeSnapshot::from(folio_build::ProjectAnalysisView {
        analysis,
        sources: view.sources.clone(),
        issues: Vec::new(),
    }))
}

fn map_offset(file: FileId, byte: usize, edits: &[RenameEdit]) -> usize {
    let mut mapped = byte;
    for edit in edits
        .iter()
        .filter(|edit| edit.span.file == file && edit.span.range.end <= byte)
    {
        let original = edit.span.range.end - edit.span.range.start;
        mapped = mapped - original + edit.replacement.len();
    }
    mapped
}

fn renamed_symbol(symbol: &Symbol, target: &Symbol, new_name: &str) -> Symbol {
    if same(symbol, target) {
        match symbol {
            Symbol::Local {
                owner, identity, ..
            } => Symbol::Local {
                owner: owner.clone(),
                name: new_name.into(),
                identity: *identity,
            },
            Symbol::Parameter { owner, .. } => Symbol::Parameter {
                owner: owner.clone(),
                name: new_name.into(),
            },
            Symbol::Member { script, .. } => Symbol::Member {
                script: script.clone(),
                name: new_name.into(),
            },
            _ => symbol.clone(),
        }
    } else {
        match symbol {
            Symbol::PropertyAccessor {
                script,
                property,
                name,
            } => {
                let property = if matches!(target,Symbol::Member{script:owner,name:target_name} if owner.eq_ignore_ascii_case(script)&&target_name.eq_ignore_ascii_case(property))
                {
                    new_name.into()
                } else {
                    property.clone()
                };
                Symbol::PropertyAccessor {
                    script: script.clone(),
                    property,
                    name: name.clone(),
                }
            },
            Symbol::Local {
                owner,
                name,
                identity,
            } => Symbol::Local {
                owner: Box::new(renamed_symbol(owner, target, new_name)),
                name: name.clone(),
                identity: *identity,
            },
            Symbol::Parameter { owner, name } => Symbol::Parameter {
                owner: Box::new(renamed_symbol(owner, target, new_name)),
                name: name.clone(),
            },
            _ => symbol.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
