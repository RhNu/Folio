//! Completion edits and argument hints derived from shared semantic facts.
use crate::{navigation::name, symbols};
use folio_build::ProjectAnalysisView;
use folio_hir::{ExpressionKind, Symbol, Type};
use folio_papyrus::SyntaxKind;
use folio_source::{FileId, TextRange};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: String,
    pub kind: u32,
    pub symbol: Option<Symbol>,
    pub replacement: TextRange,
    pub insert_text: String,
    pub documentation: Option<String>,
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
    let mut result = view
        .analysis
        .completion_candidates(file, byte, receiver, global)
        .into_iter()
        .filter(|candidate| {
            name(&candidate.symbol)
                .to_ascii_lowercase()
                .starts_with(&prefix.to_ascii_lowercase())
        })
        .map(|candidate| {
            let label = name(&candidate.symbol).to_owned();
            let hover = if let Symbol::Intrinsic { name } = &candidate.symbol {
                crate::presentation::intrinsic_hover(name, receiver)
            } else {
                crate::hover_symbol(view, &candidate.symbol)
            };
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
                    let member = symbols::find_member(&script, &candidate.symbol);
                    match member.map(|item| &item.kind) {
                        Some(folio_hir::MemberKind::Function { .. }) => 3,
                        Some(folio_hir::MemberKind::Property { .. }) => 10,
                        _ => match crate::presentation::external_member(view, &candidate.symbol)
                            .map(|member| member.kind())
                        {
                            Some(
                                folio_format_declarations::MemberKind::Function
                                | folio_format_declarations::MemberKind::Event
                                | folio_format_declarations::MemberKind::UnknownCallable,
                            ) => 3,
                            Some(folio_format_declarations::MemberKind::Property) => 10,
                            _ => 6,
                        },
                    }
                }
            };
            CompletionItem {
                label: label.clone(),
                insert_text: label,
                kind,
                detail: hover.as_ref().map_or_else(
                    || crate::display_type(&candidate.ty),
                    |hover| hover.declaration.clone(),
                ),
                documentation: hover.and_then(|hover| hover.documentation),
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
            if keyword
                .to_ascii_lowercase()
                .starts_with(&prefix.to_ascii_lowercase())
            {
                result.push(CompletionItem {
                    label: keyword.into(),
                    detail: "Papyrus keyword".into(),
                    kind: 14,
                    symbol: None,
                    replacement,
                    insert_text: keyword.into(),
                    documentation: None,
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
    result
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
