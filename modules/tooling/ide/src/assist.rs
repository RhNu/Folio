//! Completion edits and argument hints derived from shared semantic facts.
use std::{cmp::Ordering, collections::HashMap};

use folio_build::ProjectAnalysisView;
use folio_hir::{ExpressionKind, Symbol, Type};
use folio_papyrus::SyntaxKind;
use folio_source::{FileId, TextRange};

use crate::{navigation::name, symbols};

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
pub fn completion_hover(view: &crate::IdeSnapshot, item: &CompletionItem) -> Option<crate::Hover> {
    let symbol = item.symbol.as_ref()?;
    tracing::debug!(?symbol, "resolving completion presentation");
    if let Symbol::Intrinsic { name } = symbol {
        crate::presentation::intrinsic_hover(name, item.receiver.as_ref())
    } else {
        crate::hover_symbol(view, symbol)
    }
}

/// Offers selected semantic names; the checker owns scope and member precedence.
/// # Errors
/// Returns an analysis cancellation error when the supplied predicate cancels the query.
pub fn completion(
    view: &ProjectAnalysisView,
    file: FileId,
    byte: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<CompletionItem>, folio_analysis::AnalysisCancelled> {
    checkpoint(cancelled)?;
    let Some(text) = view.analysis.text(file) else {
        return Ok(Vec::new());
    };
    if byte > text.len() || !text.is_char_boundary(byte) {
        return Ok(Vec::new());
    }
    let Some(parse) = view.analysis.parse(file) else {
        return Ok(Vec::new());
    };
    let Some(offset) = byte.try_into().ok() else {
        return Ok(Vec::new());
    };
    if let Some(token) = parse.syntax().token_at_offset(offset).right_biased() {
        let range = token.text_range();
        if usize::from(range.start()) <= byte
            && byte < usize::from(range.end())
            && matches!(token.kind(), SyntaxKind::String | SyntaxKind::Comment)
        {
            return Ok(Vec::new());
        }
    }
    let mut cancellation = Cancellation::new(cancelled);
    let replacement = identifier_range(text, byte);
    let TextRange { start, .. } = replacement;
    let prefix = &text[start..byte];
    let before = text[..start].trim_end();
    view.analysis.try_warm_semantics(cancelled)?;
    let Some(script) = view.analysis.hir(file) else {
        return Ok(Vec::new());
    };
    let mut receiver = None;
    let mut global = false;
    if let Some(before_dot) = before.strip_suffix('.') {
        let dot = before_dot.len();
        let mut selected = None;
        for fact in &script.expressions {
            cancellation.check()?;
            if fact.span.range.end <= dot
                && text[fact.span.range.end..dot].trim().is_empty()
                && selected.is_none_or(|previous: &folio_hir::ExpressionFact| {
                    previous.span.range.end <= fact.span.range.end
                })
            {
                selected = Some(fact);
            }
        }
        if let Some(fact) = selected
            && matches!(fact.ty, Type::Script(_) | Type::Array(_))
        {
            receiver = Some(&fact.ty);
            global = matches!(
                fact.binding.as_ref().map(|binding| &binding.symbol),
                Some(Symbol::Script(_))
            ) && !text[fact.span.range.start..fact.span.range.end]
                .eq_ignore_ascii_case("self")
                && !text[fact.span.range.start..fact.span.range.end].eq_ignore_ascii_case("parent");
        }
        if receiver.is_none() {
            return Ok(Vec::new());
        }
    }
    let candidates = view
        .analysis
        .completion_candidates(file, byte, receiver, global, prefix, cancelled)?;
    let mut result = present_candidates(
        view,
        &script,
        candidates,
        receiver,
        prefix,
        replacement,
        &mut cancellation,
    )?;
    // Checker maps already order ASCII identifiers; preserve legacy ordering for
    // declaration carriers with Unicode names using an allocation-free comparator.
    if result.iter().any(|item| !item.label.is_ascii()) {
        result.sort_by(|a, b| compare_labels(&a.label, &b.label));
    }
    if receiver.is_none() {
        result = merge_keywords(result, prefix, replacement, &mut cancellation)?;
    }
    result.dedup_by(|a, b| a.label.eq_ignore_ascii_case(&b.label));
    checkpoint(cancelled)?;
    tracing::debug!(
        ?file,
        byte,
        candidates = result.len(),
        "completion candidates collected"
    );
    Ok(result)
}

/// Replace the whole ASCII identifier around a valid UTF-8 cursor position.
fn identifier_range(text: &str, byte: usize) -> TextRange {
    let mut start = byte;
    while start > 0
        && (text.as_bytes()[start - 1].is_ascii_alphanumeric()
            || text.as_bytes()[start - 1] == b'_')
    {
        start -= 1;
    }
    let mut end = byte;
    while end < text.len()
        && (text.as_bytes()[end].is_ascii_alphanumeric() || text.as_bytes()[end] == b'_')
    {
        end += 1;
    }
    TextRange { start, end }
}

/// Render only filtered semantic candidates, reusing per-owner kind indices.
fn present_candidates(
    view: &ProjectAnalysisView,
    script: &folio_hir::Script,
    candidates: Vec<folio_analysis::CompletionCandidate>,
    receiver: Option<&Type>,
    prefix: &str,
    replacement: TextRange,
    cancellation: &mut Cancellation<'_>,
) -> Result<Vec<CompletionItem>, folio_analysis::AnalysisCancelled> {
    let mut members = HashMap::new();
    for member in script
        .members
        .iter()
        .chain(&script.external_members)
        .chain(&script.referenced_members)
    {
        cancellation.check()?;
        if !name(&member.symbol)
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        {
            continue;
        }
        members.entry(&member.symbol).or_insert(member);
    }
    let mut external_kinds = HashMap::new();
    candidates
        .into_iter()
        .map(|candidate| {
            cancellation.check()?;
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
                },
                _ => {
                    let member = members.get(&candidate.symbol);
                    match member.map(|item| &item.kind) {
                        Some(folio_hir::MemberKind::Function { .. }) => 3,
                        Some(folio_hir::MemberKind::Property { .. }) => 10,
                        _ => external_completion_kind(
                            view,
                            &candidate.symbol,
                            &mut external_kinds,
                            prefix,
                            cancellation,
                        )?,
                    }
                },
            };
            Ok(CompletionItem {
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
            })
        })
        .collect::<Result<Vec<_>, folio_analysis::AnalysisCancelled>>()
}

/// Merge ordered keywords with semantic names, preferring semantic entries on collisions.
fn merge_keywords(
    result: Vec<CompletionItem>,
    prefix: &str,
    replacement: TextRange,
    cancellation: &mut Cancellation<'_>,
) -> Result<Vec<CompletionItem>, folio_analysis::AnalysisCancelled> {
    let mut keywords = Vec::new();
    for keyword in [
        "As",
        "Auto",
        "Auto State",
        "AutoReadOnly",
        "Bool",
        "Conditional",
        "Else",
        "ElseIf",
        "EndEvent",
        "EndFunction",
        "EndIf",
        "EndProperty",
        "EndState",
        "EndWhile",
        "Event",
        "False",
        "Float",
        "Function",
        "Global",
        "Hidden",
        "If",
        "Import",
        "Int",
        "Native",
        "New",
        "None",
        "Parent",
        "Property",
        "Return",
        "Self",
        "State",
        "String",
        "True",
        "While",
    ] {
        cancellation.check()?;
        if keyword
            .get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        {
            keywords.push(CompletionItem {
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
    // Merge already ordered inputs, retaining the semantic item on keyword collisions.
    let mut candidates = result.into_iter().peekable();
    let mut keywords = keywords.into_iter().peekable();
    let mut result = Vec::with_capacity(candidates.len() + keywords.len());
    while let (Some(candidate), Some(keyword)) = (candidates.peek(), keywords.peek()) {
        cancellation.check()?;
        match compare_labels(&candidate.label, &keyword.label) {
            Ordering::Less => result.push(candidates.next().expect("peeked completion item")),
            Ordering::Greater => result.push(keywords.next().expect("peeked completion item")),
            Ordering::Equal => {
                result.push(candidates.next().expect("peeked completion item"));
                keywords.next();
            },
        }
    }
    result.extend(candidates);
    result.extend(keywords);
    Ok(result)
}

fn compare_labels(left: &str, right: &str) -> Ordering {
    left.bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .cmp(right.bytes().map(|byte| byte.to_ascii_lowercase()))
}

fn checkpoint(cancelled: &dyn Fn() -> bool) -> Result<(), folio_analysis::AnalysisCancelled> {
    if cancelled() {
        Err(folio_analysis::AnalysisCancelled)
    } else {
        Ok(())
    }
}

/// Bound cancellation latency without acquiring the adapter's request lock per item.
struct Cancellation<'a> {
    cancelled: &'a dyn Fn() -> bool,
    remaining: u8,
}

impl<'a> Cancellation<'a> {
    fn new(cancelled: &'a dyn Fn() -> bool) -> Self {
        Self {
            cancelled,
            remaining: 0,
        }
    }

    fn check(&mut self) -> Result<(), folio_analysis::AnalysisCancelled> {
        if self.remaining == 0 {
            checkpoint(self.cancelled)?;
            self.remaining = 63;
        } else {
            self.remaining -= 1;
        }
        Ok(())
    }
}

type ExternalKinds = HashMap<String, HashMap<(Option<String>, String), u32>>;

/// Index declaration kinds once per owner, without rendering candidate signatures or docs.
fn external_completion_kind(
    view: &ProjectAnalysisView,
    symbol: &Symbol,
    kinds: &mut ExternalKinds,
    prefix: &str,
    cancellation: &mut Cancellation<'_>,
) -> Result<u32, folio_analysis::AnalysisCancelled> {
    let (owner, state, name) = match symbol {
        Symbol::Member { script, name } => (script, None, name),
        Symbol::StateMember {
            script,
            state,
            name,
        } => (script, Some(state.to_ascii_lowercase()), name),
        _ => return Ok(6),
    };
    let owner_key = owner.to_ascii_lowercase();
    if !kinds.contains_key(&owner_key) {
        let mut result = HashMap::new();
        if let Some(script) = view.analysis.external_script(owner) {
            for (state, member) in script.members.iter().map(|member| (None, member)).chain(
                script.states.iter().flat_map(|state| {
                    state
                        .members
                        .iter()
                        .map(move |member| (Some(state.name.as_str()), member))
                }),
            ) {
                cancellation.check()?;
                if !member
                    .name
                    .get(..prefix.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
                {
                    continue;
                }
                let kind = match member.kind() {
                    folio_format_declarations::MemberKind::Function
                    | folio_format_declarations::MemberKind::Event
                    | folio_format_declarations::MemberKind::UnknownCallable => 3,
                    folio_format_declarations::MemberKind::Property => 10,
                    folio_format_declarations::MemberKind::Variable => 6,
                };
                result
                    .entry((
                        state.map(str::to_ascii_lowercase),
                        member.name.to_ascii_lowercase(),
                    ))
                    .or_insert(kind);
            }
        }
        kinds.insert(owner_key.clone(), result);
    }
    Ok(kinds[&owner_key]
        .get(&(state, name.to_ascii_lowercase()))
        .copied()
        .unwrap_or(6))
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
    let Some(text) = view.analysis.text(file) else {
        return Vec::new();
    };
    let Some(parse) = view.analysis.parse(file) else {
        return Vec::new();
    };
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
            if !(range.start..=range.end).contains(&at.start)
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
