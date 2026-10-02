//! Semantic occurrences and hierarchy facts for every editor navigation request.
use folio_build::ProjectAnalysisView;
use folio_hir::{ExpressionKind, MemberKind, Symbol};
use folio_papyrus::SyntaxKind;
use folio_source::{FileId, SourceSpan};

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
        (Symbol::Script(a), Symbol::Script(b))
        | (Symbol::ParentReceiver { script: a }, Symbol::ParentReceiver { script: b })
        | (Symbol::Intrinsic { name: a }, Symbol::Intrinsic { name: b }) => {
            a.eq_ignore_ascii_case(b)
        }
        (Symbol::Member { script: a, name: x }, Symbol::Member { script: b, name: y }) => {
            a.eq_ignore_ascii_case(b) && x.eq_ignore_ascii_case(y)
        }
        (
            Symbol::StateMember {
                script: a,
                state: s,
                name: x,
            },
            Symbol::StateMember {
                script: b,
                state: t,
                name: y,
            },
        ) => a.eq_ignore_ascii_case(b) && s.eq_ignore_ascii_case(t) && x.eq_ignore_ascii_case(y),
        (
            Symbol::PropertyAccessor {
                script: a,
                property: s,
                name: x,
            },
            Symbol::PropertyAccessor {
                script: b,
                property: t,
                name: y,
            },
        ) => a.eq_ignore_ascii_case(b) && s.eq_ignore_ascii_case(t) && x.eq_ignore_ascii_case(y),
        (Symbol::Parameter { owner: a, name: x }, Symbol::Parameter { owner: b, name: y }) => {
            same(a, b) && x.eq_ignore_ascii_case(y)
        }
        (
            Symbol::Local {
                owner: a,
                name: x,
                identity: i,
            },
            Symbol::Local {
                owner: b,
                name: y,
                identity: j,
            },
        ) => same(a, b) && x.eq_ignore_ascii_case(y) && i == j,
        _ => false,
    }
}

pub(crate) fn occurrences(view: &ProjectAnalysisView, file: FileId) -> Vec<SymbolOccurrence> {
    let Some(script) = view.analysis.hir(file) else {
        return Vec::new();
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
        result.push(SymbolOccurrence {
            symbol: item.symbol.clone(),
            span: item.span,
            definition: Some(item.span),
        });
    }
    for fact in &script.expressions {
        if let Some(binding) = &fact.binding {
            result.push(SymbolOccurrence {
                symbol: binding.symbol.clone(),
                span: binding.name.span,
                definition: binding.definition,
            });
        }
    }
    if let Some(parse) = view.analysis.parse(file) {
        for node in parse
            .syntax()
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::NamedArgument)
        {
            let Some(token) = node
                .descendants_with_tokens()
                .filter_map(|item| item.into_token())
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
            let Some(member) = super::symbols::find_member(&script, &binding.symbol) else {
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
            let definition = view.analysis.file_ids().find_map(|file| {
                view.analysis
                    .hir(file)?
                    .declarations
                    .iter()
                    .find(|item| same(&item.symbol, &symbol))
                    .map(|item| item.span)
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
            .filter_map(|item| item.into_token())
            .filter(|token| token.kind() == SyntaxKind::Ident)
        {
            let byte = usize::from(token.text_range().start());
            if let Some((name, span)) = super::symbols::script_reference(view, file, byte) {
                let definition = view.analysis.file_ids().find_map(|file| {
                    let script = view.analysis.hir(file)?;
                    let found = script.name.as_ref()?;
                    found.text.eq_ignore_ascii_case(&name).then_some(found.span)
                });
                let known = definition.is_some()
                    || view
                        .analysis
                        .external_declarations()
                        .iter()
                        .flat_map(|bundle| &bundle.scripts)
                        .any(|script| script.name.eq_ignore_ascii_case(&name));
                if known {
                    result.push(SymbolOccurrence {
                        symbol: Symbol::Script(name),
                        span,
                        definition,
                    });
                }
            }
        }
    }
    result.sort_by_key(|item| (item.span.range.start, item.span.range.end));
    result.dedup_by(|a, b| a.span == b.span && same(&a.symbol, &b.symbol));
    result
}

pub fn symbol_at(
    view: &ProjectAnalysisView,
    file: FileId,
    byte: usize,
) -> Option<SymbolOccurrence> {
    occurrences(view, file)
        .into_iter()
        .filter(|item| item.span.range.start <= byte && byte < item.span.range.end)
        .min_by_key(|item| item.span.range.end - item.span.range.start)
}

pub fn references(
    view: &ProjectAnalysisView,
    file: FileId,
    byte: usize,
    include_declaration: bool,
) -> Vec<SourceSpan> {
    let Some(target) = symbol_at(view, file, byte) else {
        return Vec::new();
    };
    references_to(view, &target, include_declaration)
}

pub fn definition_of(view: &ProjectAnalysisView, symbol: &Symbol) -> Option<SourceSpan> {
    let mut definitions = view
        .analysis
        .file_ids()
        .flat_map(|file| occurrences(view, file))
        .filter(|item| same(&item.symbol, symbol) && item.definition == Some(item.span))
        .map(|item| item.span)
        .collect::<Vec<_>>();
    definitions.sort_by_key(|span| (span.file, span.range.start));
    definitions.dedup();
    if definitions.len() == 1 {
        definitions.pop()
    } else {
        None
    }
}

pub fn references_of(
    view: &ProjectAnalysisView,
    symbol: &Symbol,
    include_declaration: bool,
) -> Vec<SourceSpan> {
    let definition = definition_of(view, symbol);
    reference_spans(view, symbol, definition, include_declaration)
}

pub(crate) fn references_to(
    view: &ProjectAnalysisView,
    target: &SymbolOccurrence,
    include_declaration: bool,
) -> Vec<SourceSpan> {
    reference_spans(view, &target.symbol, target.definition, include_declaration)
}

fn reference_spans(
    view: &ProjectAnalysisView,
    symbol: &Symbol,
    definition: Option<SourceSpan>,
    include_declaration: bool,
) -> Vec<SourceSpan> {
    let local = matches!(symbol, Symbol::Local { .. } | Symbol::Parameter { .. });
    if local && definition.is_none() {
        return Vec::new();
    }
    let mut result = view
        .analysis
        .file_ids()
        .flat_map(|file| occurrences(view, file))
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

pub fn document_highlights(
    view: &ProjectAnalysisView,
    file: FileId,
    byte: usize,
) -> Vec<SourceSpan> {
    references(view, file, byte, true)
        .into_iter()
        .filter(|span| span.file == file)
        .collect()
}

fn parent(view: &ProjectAnalysisView, name: &str) -> Option<String> {
    view.analysis
        .file_ids()
        .find_map(|file| {
            let script = view.analysis.hir(file)?;
            script
                .name
                .as_ref()?
                .text
                .eq_ignore_ascii_case(name)
                .then(|| script.parent.as_ref().map(|parent| parent.text.clone()))
                .flatten()
        })
        .or_else(|| {
            view.analysis
                .external_declarations()
                .iter()
                .flat_map(|bundle| &bundle.scripts)
                .find(|script| script.name.eq_ignore_ascii_case(name))
                .and_then(|script| script.parent.clone())
        })
}

pub(crate) fn derives(view: &ProjectAnalysisView, child: &str, ancestor: &str) -> bool {
    let mut current = parent(view, child);
    let mut seen = std::collections::BTreeSet::new();
    while let Some(name) = current {
        if name.eq_ignore_ascii_case(ancestor) {
            return true;
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            break;
        }
        current = parent(view, &name);
    }
    false
}

/// Derived scripts and callable overrides, including state implementations.
pub fn implementations(view: &ProjectAnalysisView, file: FileId, byte: usize) -> Vec<SourceSpan> {
    let Some(target) = symbol_at(view, file, byte) else {
        return Vec::new();
    };
    implementations_of(view, &target.symbol)
}

pub fn implementations_of(view: &ProjectAnalysisView, target: &Symbol) -> Vec<SourceSpan> {
    let mut result = implementation_symbols(view, target)
        .iter()
        .filter_map(|symbol| definition_of(view, symbol))
        .collect::<Vec<_>>();
    result.sort_by_key(|span| (span.file, span.range.start));
    result.dedup();
    result
}

fn callable_kind(view: &ProjectAnalysisView, symbol: &Symbol) -> Option<(bool, bool)> {
    for file in view.analysis.file_ids() {
        let script = view.analysis.hir(file)?;
        if let Some(member) = script
            .members
            .iter()
            .find(|member| same(&member.symbol, symbol))
        {
            return match member.kind {
                MemberKind::Function { event, global, .. } => Some((event, global)),
                _ => None,
            };
        }
    }
    let member = crate::presentation::external_member(view, symbol)?;
    match member.data {
        folio_format_declarations::MemberData::Function { global, .. } => Some((false, global)),
        folio_format_declarations::MemberData::Event { .. } => Some((true, false)),
        _ => None,
    }
}

/// Selected source and external descendants share semantic identity; callers choose verified carriers.
pub fn implementation_symbols(view: &ProjectAnalysisView, target: &Symbol) -> Vec<Symbol> {
    let target_kind = callable_kind(view, target);
    if !matches!(target, Symbol::Script(_)) && !matches!(target_kind, Some((_, false))) {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    for file in view.analysis.file_ids() {
        if let Some(script) = view.analysis.hir(file) {
            if let Some(name) = &script.name {
                candidates.push(Symbol::Script(name.text.clone()));
            }
            candidates.extend(script.members.iter().map(|member| member.symbol.clone()));
        }
    }
    for script in view
        .analysis
        .external_declarations()
        .iter()
        .flat_map(|bundle| &bundle.scripts)
    {
        candidates.push(Symbol::Script(script.name.clone()));
        candidates.extend(script.members.iter().map(|member| Symbol::Member {
            script: script.name.clone(),
            name: member.name.clone(),
        }));
        candidates.extend(script.states.iter().flat_map(|state| {
            state.members.iter().map(|member| Symbol::StateMember {
                script: script.name.clone(),
                state: state.name.clone(),
                name: member.name.clone(),
            })
        }));
    }
    candidates.retain(|candidate| match (target, candidate) {
        (Symbol::Script(ancestor), Symbol::Script(child)) => derives(view, child, ancestor),
        (
            Symbol::Member {
                script: owner,
                name: target_name,
            }
            | Symbol::StateMember {
                script: owner,
                name: target_name,
                ..
            },
            Symbol::Member {
                script: child,
                name,
            }
            | Symbol::StateMember {
                script: child,
                name,
                ..
            },
        ) => {
            !same(target, candidate)
                && target_name.eq_ignore_ascii_case(name)
                && (child.eq_ignore_ascii_case(owner) || derives(view, child, owner))
                && callable_kind(view, candidate) == target_kind
        }
        _ => false,
    });
    candidates.sort_by_key(|symbol| format!("{symbol:?}").to_ascii_lowercase());
    candidates.dedup_by(|a, b| same(a, b));
    candidates
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceSymbol {
    pub name: String,
    pub kind: u32,
    pub span: SourceSpan,
    pub container: Option<String>,
}

pub fn workspace_symbols(view: &ProjectAnalysisView, query: &str) -> Vec<WorkspaceSymbol> {
    fn flatten(
        file: FileId,
        items: Vec<super::DocumentSymbol>,
        container: Option<String>,
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
                    container: container.clone(),
                });
            }
            flatten(file, item.children, Some(item.name), query, result);
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
