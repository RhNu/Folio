//! Completion edits and argument hints derived from shared semantic facts.
use crate::{navigation::name, symbols};
use folio_build::ProjectAnalysisView;
use folio_hir::{ExpressionKind, Symbol, Type};
use folio_papyrus::SyntaxKind;
use folio_source::{FileId, TextRange};
use std::collections::HashMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: String,
    pub kind: u32,
    pub symbol: Option<Symbol>,
    pub replacement: TextRange,
    pub insert_text: String,
    pub documentation: Option<String>,
    /// Receiver type retained for lazy presentation of array intrinsics.
    pub receiver: Option<Type>,
}

/// Expands only the selected completion into its full declaration and documentation.
pub fn completion_hover(view: &ProjectAnalysisView, item: &CompletionItem) -> Option<crate::Hover> {
    let symbol = item.symbol.as_ref()?;
    tracing::debug!(?symbol, "resolving completion presentation");
    if let Symbol::Intrinsic { name } = symbol {
        crate::presentation::intrinsic_hover(name, item.receiver.as_ref())
    } else {
        crate::hover_symbol(view, symbol)
    }
}

/// Offers selected semantic names; the checker owns scope and member precedence.
pub fn completion(view: &ProjectAnalysisView, file: FileId, byte: usize) -> Vec<CompletionItem> {
    let Some(text) = view.analysis.text(file) else {
        return Vec::new();
    };
    if byte > text.len() || !text.is_char_boundary(byte) {
        return Vec::new();
    }
    let parse = view.analysis.parse(file).unwrap();
    if parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(|item| item.into_token())
        .any(|token| {
            let range = token.text_range();
            usize::from(range.start()) <= byte
                && byte < usize::from(range.end())
                && matches!(token.kind(), SyntaxKind::String | SyntaxKind::Comment)
        })
    {
        return Vec::new();
    }
    let mut start = byte;
    while start > 0 && text.as_bytes()[start - 1].is_ascii_alphanumeric()
        || start > 0 && text.as_bytes()[start - 1] == b'_'
    {
        start -= 1;
    }
    let mut end = byte;
    while end < text.len()
        && (text.as_bytes()[end].is_ascii_alphanumeric() || text.as_bytes()[end] == b'_')
    {
        end += 1;
    }
    let prefix = &text[start..byte];
    let before = text[..start].trim_end();
    let script = view.analysis.hir(file).unwrap();
    let mut receiver = None;
    let mut global = false;
    if let Some(before_dot) = before.strip_suffix('.') {
        let dot = before_dot.len();
        let fact = script
            .expressions
            .iter()
            .filter(|fact| {
                fact.span.range.end <= dot && text[fact.span.range.end..dot].trim().is_empty()
            })
            .max_by_key(|fact| fact.span.range.end);
        if let Some(fact) = fact {
            if matches!(fact.ty, Type::Script(_) | Type::Array(_)) {
                receiver = Some(&fact.ty);
                global = matches!(
                    fact.binding.as_ref().map(|binding| &binding.symbol),
                    Some(Symbol::Script(_))
                ) && !text[fact.span.range.start..fact.span.range.end]
                    .eq_ignore_ascii_case("self")
                    && !text[fact.span.range.start..fact.span.range.end]
                        .eq_ignore_ascii_case("parent");
            }
        }
        if receiver.is_none() {
            return Vec::new();
        }
    }
    let replacement = TextRange { start, end };
    let prefix = prefix.to_ascii_lowercase();
    let mut members = HashMap::new();
    for member in script
        .members
        .iter()
        .chain(&script.external_members)
        .chain(&script.referenced_members)
    {
        members.entry(&member.symbol).or_insert(member);
    }
    let mut external_kinds = HashMap::new();
    let mut result = view
        .analysis
        .completion_candidates(file, byte, receiver, global)
        .into_iter()
        .filter(|candidate| {
            name(&candidate.symbol)
                .to_ascii_lowercase()
                .starts_with(&prefix)
        })
        .map(|candidate| {
            let label = name(&candidate.symbol).to_owned();
            let kind = match &candidate.symbol {
                Symbol::Script(_) => 7,
                Symbol::Parameter { .. } | Symbol::Local { .. } => 6,
                Symbol::Intrinsic { name } => {
                    if folio_analysis::intrinsic_signature(name, receiver)
                        .is_some_and(|signature| signature.callable)
                    {
                        3
                    } else {
                        10
                    }
                }
                _ => {
                    let member = members.get(&candidate.symbol);
                    match member.map(|item| &item.kind) {
                        Some(folio_hir::MemberKind::Function { .. }) => 3,
                        Some(folio_hir::MemberKind::Property { .. }) => 10,
                        _ => external_completion_kind(view, &candidate.symbol, &mut external_kinds),
                    }
                }
            };
            CompletionItem {
                label: label.clone(),
                insert_text: label,
                kind,
                detail: crate::display_type(&candidate.ty),
                documentation: None,
                receiver: matches!(candidate.symbol, Symbol::Intrinsic { .. })
                    .then(|| receiver.cloned())
                    .flatten(),
                symbol: Some(candidate.symbol),
                replacement,
            }
        })
        .collect::<Vec<_>>();
    if receiver.is_none() {
        for keyword in [
            "Int",
            "Float",
            "Bool",
            "String",
            "None",
            "Self",
            "Parent",
            "If",
            "ElseIf",
            "Else",
            "EndIf",
            "While",
            "EndWhile",
            "Return",
            "Function",
            "EndFunction",
            "Event",
            "EndEvent",
            "Property",
            "EndProperty",
            "Auto",
            "AutoReadOnly",
            "State",
            "EndState",
            "Import",
            "New",
            "As",
            "True",
            "False",
            "Hidden",
            "Conditional",
            "Global",
            "Native",
            "Auto State",
        ] {
            if keyword.to_ascii_lowercase().starts_with(&prefix) {
                result.push(CompletionItem {
                    label: keyword.into(),
                    detail: "Papyrus keyword".into(),
                    kind: 14,
                    symbol: None,
                    replacement,
                    insert_text: keyword.into(),
                    documentation: None,
                    receiver: None,
                });
            }
        }
    }
    result.sort_by(|a, b| {
        a.label
            .to_ascii_lowercase()
            .cmp(&b.label.to_ascii_lowercase())
    });
    result.dedup_by(|a, b| a.label.eq_ignore_ascii_case(&b.label));
    tracing::debug!(
        ?file,
        byte,
        candidates = result.len(),
        "completion candidates collected"
    );
    result
}

type ExternalKinds = HashMap<String, HashMap<(Option<String>, String), u32>>;

/// Index declaration kinds once per owner, without rendering candidate signatures or docs.
fn external_completion_kind(
    view: &ProjectAnalysisView,
    symbol: &Symbol,
    kinds: &mut ExternalKinds,
) -> u32 {
    let (owner, state, name) = match symbol {
        Symbol::Member { script, name } => (script, None, name),
        Symbol::StateMember {
            script,
            state,
            name,
        } => (script, Some(state.to_ascii_lowercase()), name),
        _ => return 6,
    };
    let members = kinds.entry(owner.to_ascii_lowercase()).or_insert_with(|| {
        let mut result = HashMap::new();
        if let Some(script) = view.analysis.external_script(owner) {
            for (state, member) in script.members.iter().map(|member| (None, member)).chain(
                script.states.iter().flat_map(|state| {
                    state
                        .members
                        .iter()
                        .map(move |member| (Some(state.name.to_ascii_lowercase()), member))
                }),
            ) {
                let kind = match member.kind() {
                    folio_format_declarations::MemberKind::Function
                    | folio_format_declarations::MemberKind::Event
                    | folio_format_declarations::MemberKind::UnknownCallable => 3,
                    folio_format_declarations::MemberKind::Property => 10,
                    _ => 6,
                };
                result
                    .entry((state, member.name.to_ascii_lowercase()))
                    .or_insert(kind);
            }
        }
        result
    });
    members
        .get(&(state, name.to_ascii_lowercase()))
        .copied()
        .unwrap_or(6)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InlayHint {
    pub byte: usize,
    pub label: String,
    pub parameter: Option<Symbol>,
}

/// Maps source-order arguments back to declaration order and omits explicit or obvious labels.
pub fn inlay_hints(view: &ProjectAnalysisView, file: FileId, range: TextRange) -> Vec<InlayHint> {
    let Some(script) = view.analysis.hir(file) else {
        return Vec::new();
    };
    let text = view.analysis.text(file).unwrap();
    let parse = view.analysis.parse(file).unwrap();
    let named = parse
        .syntax()
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::NamedArgument)
        .map(|node| {
            let at = node.text_range();
            TextRange {
                start: usize::from(at.start()),
                end: usize::from(at.end()),
            }
        })
        .collect::<Vec<_>>();
    let mut result = Vec::new();
    for fact in &script.expressions {
        let ExpressionKind::Call {
            callee,
            arguments,
            argument_ordinals,
            ..
        } = &fact.kind
        else {
            continue;
        };
        let Some(binding) = &callee.binding else {
            continue;
        };
        let intrinsic = crate::presentation::intrinsic_call_member(callee);
        let Some(member) = symbols::find_member(&script, &binding.symbol).or(intrinsic.as_ref())
        else {
            continue;
        };
        for (source_index, argument) in arguments.iter().enumerate() {
            let at = argument.span.range;
            if at.start < range.start
                || at.start > range.end
                || named
                    .iter()
                    .any(|range| range.start <= at.start && at.end <= range.end)
            {
                continue;
            }
            let Some(parameter) = argument_ordinals
                .get(source_index)
                .and_then(|ordinal| member.parameters.get(*ordinal))
            else {
                continue;
            };
            if text[at.start..at.end]
                .trim()
                .eq_ignore_ascii_case(&parameter.name)
            {
                continue;
            }
            result.push(InlayHint {
                byte: at.start,
                label: format!("{}:", parameter.name),
                parameter: Some(Symbol::Parameter {
                    owner: Box::new(member.symbol.clone()),
                    name: parameter.name.clone(),
                }),
            });
        }
    }
    result.sort_by_key(|item| item.byte);
    result.dedup_by(|a, b| a.byte == b.byte && a.label == b.label);
    result
}

#[cfg(test)]
mod tests;
