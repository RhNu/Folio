//! Pure navigation into a supplied PSC snapshot; never resolves filesystem paths.

use folio_build::ProjectAnalysisView;
use folio_hir::Symbol;
use folio_papyrus::{PapyrusDialect, SyntaxKind, SyntaxNode, parse};
use folio_source::{FileId, TextRange};

/// Preserve the selected semantic owner when navigating inherited external members.
pub fn referenced_symbol(view: &ProjectAnalysisView, file: FileId, byte: usize) -> Option<Symbol> {
    if let Some((name, _)) = super::symbols::script_reference(view, file, byte) {
        return Some(Symbol::Script(name));
    }
    let script = view.analysis.hir(file)?;
    script
        .expression_at(byte)?
        .binding
        .as_ref()
        .map(|binding| binding.symbol.clone())
}

/// External navigation is limited to script and member identities.
pub fn symbol_script(symbol: &Symbol) -> Option<&str> {
    match symbol {
        Symbol::Script(script)
        | Symbol::Member { script, .. }
        | Symbol::StateMember { script, .. } => Some(script),
        _ => None,
    }
}

/// Find one declaration name in the matching script and state, without checking bodies.
/// Ambiguous declarations produce no navigation rather than an arbitrary location.
pub fn external_declaration_range(text: &str, symbol: &Symbol) -> Option<TextRange> {
    let owner = symbol_script(symbol)?;
    let parsed = parse(text, PapyrusDialect::Skyrim);
    let root = parsed.syntax();
    let script = root
        .children()
        .find(|node| node.kind() == SyntaxKind::ScriptDecl)?;
    let (script_name, script_range) = name_after(&script, "scriptname")?;
    if !script_name.eq_ignore_ascii_case(owner) {
        return None;
    }
    if matches!(symbol, Symbol::Script(_)) {
        return Some(script_range);
    }
    let (wanted, wanted_state) = match symbol {
        Symbol::Member { name, .. } => (name.as_str(), None),
        Symbol::StateMember { name, state, .. } => (name.as_str(), Some(state.as_str())),
        _ => return None,
    };
    let mut found = None;
    for node in root.descendants() {
        let keyword = match node.kind() {
            SyntaxKind::FunctionDecl => "function",
            SyntaxKind::EventDecl => "event",
            SyntaxKind::PropertyDecl => "property",
            SyntaxKind::VariableDecl => "",
            _ => continue,
        };
        // Accessors and local variables are not exported script members.
        if node.ancestors().skip(1).any(|ancestor| {
            matches!(
                ancestor.kind(),
                SyntaxKind::FunctionDecl | SyntaxKind::EventDecl | SyntaxKind::PropertyDecl
            )
        }) {
            continue;
        }
        let state_node = node
            .ancestors()
            .find(|ancestor| ancestor.kind() == SyntaxKind::StateDecl);
        let state_name = state_node
            .as_ref()
            .and_then(|state| name_after(state, "state").map(|(name, _)| name));
        if !match (wanted_state, state_name.as_deref()) {
            (None, None) => true,
            (Some(expected), Some(actual)) => expected.eq_ignore_ascii_case(actual),
            _ => false,
        } {
            continue;
        }
        let Some((name, range)) = name_after(&node, keyword) else {
            continue;
        };
        if name.eq_ignore_ascii_case(wanted) {
            if found.is_some() {
                return None;
            }
            found = Some(range);
        }
    }
    found
}

fn name_after(node: &SyntaxNode, keyword: &str) -> Option<(String, TextRange)> {
    let mut eligible = keyword.is_empty();
    for token in node
        .children_with_tokens()
        .filter_map(|item| item.into_token())
    {
        if token.kind() != SyntaxKind::Ident {
            continue;
        }
        if eligible {
            let range = token.text_range();
            return Some((
                token.text().into(),
                TextRange {
                    start: usize::from(range.start()),
                    end: usize::from(range.end()),
                },
            ));
        }
        eligible = token.text().eq_ignore_ascii_case(keyword);
    }
    None
}

#[cfg(test)]
mod tests;
