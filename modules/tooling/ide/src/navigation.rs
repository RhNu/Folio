//! Semantic occurrences and hierarchy facts for every editor navigation request.
use std::collections::HashMap;

use folio_hir::{ExpressionKind, Symbol};
use folio_papyrus::SyntaxKind;
use folio_source::{FileId, SourceSpan};

use crate::IdeSnapshot;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolOccurrence {
    pub symbol: Symbol,
    pub span: SourceSpan,
    pub definition: Option<SourceSpan>,
}

pub(crate) fn name(symbol: &Symbol) -> &str {
    match symbol {
        Symbol::Script(name)
        | Symbol::ParentReceiver { script: name }
        | Symbol::Intrinsic { name }
        | Symbol::Member { name, .. }
        | Symbol::StateMember { name, .. }
        | Symbol::PropertyAccessor { name, .. }
        | Symbol::Parameter { name, .. }
        | Symbol::Local { name, .. } => name,
    }
}

/// Papyrus semantic identity ignores identifier casing; locals also need their definition span.
pub(crate) fn same(left: &Symbol, right: &Symbol) -> bool {
    match (left, right) {
        (Symbol::Script(left_owner), Symbol::Script(right_owner))
        | (
            Symbol::ParentReceiver { script: left_owner },
            Symbol::ParentReceiver {
                script: right_owner,
            },
        )
        | (Symbol::Intrinsic { name: left_owner }, Symbol::Intrinsic { name: right_owner }) => {
            left_owner.eq_ignore_ascii_case(right_owner)
        },
        (
            Symbol::Member {
                script: left_owner,
                name: left_name,
            },
            Symbol::Member {
                script: right_owner,
                name: right_name,
            },
        ) => {
            left_owner.eq_ignore_ascii_case(right_owner)
                && left_name.eq_ignore_ascii_case(right_name)
        },
        (
            Symbol::StateMember {
                script: left_owner,
                state: left_scope,
                name: left_name,
            },
            Symbol::StateMember {
                script: right_owner,
                state: right_scope,
                name: right_name,
            },
        )
        | (
            Symbol::PropertyAccessor {
                script: left_owner,
                property: left_scope,
                name: left_name,
            },
            Symbol::PropertyAccessor {
                script: right_owner,
                property: right_scope,
                name: right_name,
            },
        ) => {
            left_owner.eq_ignore_ascii_case(right_owner)
                && left_scope.eq_ignore_ascii_case(right_scope)
                && left_name.eq_ignore_ascii_case(right_name)
        },
        (
            Symbol::Parameter {
                owner: left_owner,
                name: left_name,
            },
            Symbol::Parameter {
                owner: right_owner,
                name: right_name,
            },
        ) => same(left_owner, right_owner) && left_name.eq_ignore_ascii_case(right_name),
        (
            Symbol::Local {
                owner: left_owner,
                name: left_name,
                identity: left_identity,
            },
            Symbol::Local {
                owner: right_owner,
                name: right_name,
                identity: right_identity,
            },
        ) => {
            same(left_owner, right_owner)
                && left_name.eq_ignore_ascii_case(right_name)
                && left_identity == right_identity
        },
        _ => false,
    }
}

pub(crate) fn collect_occurrences(
    view: &IdeSnapshot,
    file: FileId,
) -> Option<Vec<SymbolOccurrence>> {
    let Some(script) = view.analysis.hir(file) else {
        return Some(Vec::new());
    };
    let mut result = Vec::new();
    if let Some(name) = &script.name {
        result.push(SymbolOccurrence {
            symbol: Symbol::Script(name.text.clone()),
            span: name.span,
            definition: Some(name.span),
        });
    }
    for item in &script.declarations {
        if view.is_cancelled() {
            return None;
        }
        result.push(SymbolOccurrence {
            symbol: item.symbol.clone(),
            span: item.span,
            definition: Some(item.span),
        });
    }
    for fact in &script.expressions {
        if view.is_cancelled() {
            return None;
        }
        if let Some(binding) = &fact.binding {
            result.push(SymbolOccurrence {
                symbol: binding.symbol.clone(),
                span: binding.name.span,
                definition: binding.definition,
            });
        }
    }
    if let Some(parse) = view.analysis.parse(file) {
        collect_syntax_occurrences(view, file, &script, &parse, &mut result)?;
    }
    result.sort_by_key(|item| (item.span.range.start, item.span.range.end));
    result.dedup_by(|a, b| a.span == b.span && same(&a.symbol, &b.symbol));
    tracing::debug!(
        ?file,
        occurrences = result.len(),
        "editor file occurrences indexed"
    );
    Some(result)
}

/// Add named argument and script-reference tokens to semantic occurrences.
fn collect_syntax_occurrences(
    view: &IdeSnapshot,
    file: FileId,
    script: &folio_hir::Script,
    parse: &folio_papyrus::Parse,
    result: &mut Vec<SymbolOccurrence>,
) -> Option<()> {
    let mut parameter_definitions = HashMap::new();
    for node in parse
        .syntax()
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::NamedArgument)
    {
        if view.is_cancelled() {
            return None;
        }
        let Some(token) = node
            .descendants_with_tokens()
            .filter_map(folio_papyrus::SyntaxElement::into_token)
            .find(|token| token.kind() == SyntaxKind::Ident)
        else {
            continue;
        };
        let start = usize::from(node.text_range().start());
        let end = usize::from(node.text_range().end());
        let call = script
            .expressions
            .iter()
            .filter(|fact| {
                matches!(fact.kind, ExpressionKind::Call { .. })
                    && fact.span.range.start <= start
                    && end <= fact.span.range.end
            })
            .min_by_key(|fact| fact.span.range.end - fact.span.range.start);
        let Some(call) = call else {
            continue;
        };
        let ExpressionKind::Call { callee, .. } = &call.kind else {
            continue;
        };
        let Some(binding) = &callee.binding else {
            continue;
        };
        let Some(member) = super::symbols::find_member(script, &binding.symbol) else {
            continue;
        };
        let Some(parameter) = member
            .parameters
            .iter()
            .find(|parameter| parameter.name.eq_ignore_ascii_case(token.text()))
        else {
            continue;
        };
        let symbol = Symbol::Parameter {
            owner: Box::new(member.symbol.clone()),
            name: parameter.name.clone(),
        };
        let definition = *parameter_definitions
            .entry(crate::snapshot::SymbolKey::new(&symbol))
            .or_insert_with(|| {
                view.definitions(&symbol)
                    .and_then(|spans| spans.first().copied())
            });
        result.push(SymbolOccurrence {
            symbol,
            span: SourceSpan {
                file,
                range: super::symbols::token_range(&token),
            },
            definition,
        });
    }
    for token in parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(folio_papyrus::SyntaxElement::into_token)
        .filter(|token| token.kind() == SyntaxKind::Ident)
    {
        if view.is_cancelled() {
            return None;
        }
        if let Some((name, span)) = super::symbols::script_reference_token(script, file, &token) {
            let definition = view.script_definition(&name);
            let known = definition.is_some() || view.analysis.external_script(&name).is_some();
            if known {
                result.push(SymbolOccurrence {
                    symbol: Symbol::Script(name),
                    span,
                    definition,
                });
            }
        }
    }
    Some(())
}

pub(crate) fn occurrences(view: &IdeSnapshot, file: FileId) -> &[SymbolOccurrence] {
    view.occurrences(file).unwrap_or(&[])
}

pub fn symbol_at(view: &IdeSnapshot, file: FileId, byte: usize) -> Option<SymbolOccurrence> {
    occurrences(view, file)
        .iter()
        .filter(|item| item.span.range.start <= byte && byte < item.span.range.end)
        .min_by_key(|item| item.span.range.end - item.span.range.start)
        .cloned()
}

pub fn references(
    view: &IdeSnapshot,
    file: FileId,
    byte: usize,
    include_declaration: bool,
) -> Vec<SourceSpan> {
    let Some(target) = symbol_at(view, file, byte) else {
        return Vec::new();
    };
    references_to(view, &target, include_declaration)
}

pub fn definition_of(view: &IdeSnapshot, symbol: &Symbol) -> Option<SourceSpan> {
    let definitions = view.definitions(symbol)?;
    (definitions.len() == 1).then(|| definitions[0])
}

pub fn references_of(
    view: &IdeSnapshot,
    symbol: &Symbol,
    include_declaration: bool,
) -> Vec<SourceSpan> {
    let definition = definition_of(view, symbol);
    reference_spans(view, symbol, definition, include_declaration)
}

pub(crate) fn references_to(
    view: &IdeSnapshot,
    target: &SymbolOccurrence,
    include_declaration: bool,
) -> Vec<SourceSpan> {
    reference_spans(view, &target.symbol, target.definition, include_declaration)
}

fn reference_spans(
    view: &IdeSnapshot,
    symbol: &Symbol,
    definition: Option<SourceSpan>,
    include_declaration: bool,
) -> Vec<SourceSpan> {
    let local = matches!(symbol, Symbol::Local { .. } | Symbol::Parameter { .. });
    if local && definition.is_none() {
        return Vec::new();
    }
    let mut result = view
        .references(symbol)
        .unwrap_or(&[])
        .iter()
        .filter(|item| {
            same(&item.symbol, symbol)
                && (!local || item.definition == definition)
                && (include_declaration || Some(item.span) != item.definition)
        })
        .map(|item| item.span)
        .collect::<Vec<_>>();
    result.sort_by_key(|span| (span.file, span.range.start, span.range.end));
    result.dedup();
    result
}

pub fn document_highlights(view: &IdeSnapshot, file: FileId, byte: usize) -> Vec<SourceSpan> {
    let Some(target) = symbol_at(view, file, byte) else {
        return Vec::new();
    };
    let local = matches!(
        target.symbol,
        Symbol::Local { .. } | Symbol::Parameter { .. }
    );
    if local && target.definition.is_none() {
        return Vec::new();
    }
    let mut result = occurrences(view, file)
        .iter()
        .filter(|item| {
            same(&item.symbol, &target.symbol) && (!local || item.definition == target.definition)
        })
        .map(|item| item.span)
        .collect::<Vec<_>>();
    result.sort_by_key(|span| (span.range.start, span.range.end));
    result.dedup();
    result
}

pub(crate) fn derives(view: &IdeSnapshot, child: &str, ancestor: &str) -> bool {
    view.hierarchy()
        .is_some_and(|index| index.derives(child, ancestor))
}

/// Derived scripts and callable overrides, including state implementations.
pub fn implementations(view: &IdeSnapshot, file: FileId, byte: usize) -> Vec<SourceSpan> {
    let Some(target) = symbol_at(view, file, byte) else {
        return Vec::new();
    };
    implementations_of(view, &target.symbol)
}

pub fn implementations_of(view: &IdeSnapshot, target: &Symbol) -> Vec<SourceSpan> {
    let mut result = implementation_symbols(view, target)
        .iter()
        .filter_map(|symbol| definition_of(view, symbol))
        .collect::<Vec<_>>();
    result.sort_by_key(|span| (span.file, span.range.start));
    result.dedup();
    result
}

/// Selected source and external descendants share semantic identity; callers choose verified carriers.
pub fn implementation_symbols(view: &IdeSnapshot, target: &Symbol) -> Vec<Symbol> {
    view.hierarchy()
        .and_then(|index| index.implementations(view, target))
        .map_or_else(Vec::new, |symbols| (*symbols).clone())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceSymbol {
    pub name: String,
    pub kind: u32,
    pub span: SourceSpan,
    pub container: Option<String>,
}

pub fn workspace_symbols(view: &IdeSnapshot, query: &str) -> Vec<WorkspaceSymbol> {
    fn flatten(
        file: FileId,
        items: Vec<super::DocumentSymbol>,
        container: Option<&str>,
        query: &str,
        result: &mut Vec<WorkspaceSymbol>,
    ) {
        for item in items {
            if item
                .name
                .to_ascii_lowercase()
                .contains(&query.to_ascii_lowercase())
            {
                result.push(WorkspaceSymbol {
                    name: item.name.clone(),
                    kind: item.kind,
                    span: SourceSpan {
                        file,
                        range: item.selection_range,
                    },
                    container: container.map(str::to_owned),
                });
            }
            flatten(file, item.children, Some(&item.name), query, result);
        }
    }
    let mut result = Vec::new();
    for file in view.analysis.file_ids() {
        flatten(
            file,
            super::document_symbols(view, file),
            None,
            query,
            &mut result,
        );
    }
    result
}

#[cfg(test)]
mod tests;
