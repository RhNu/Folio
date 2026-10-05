//! Pure navigation into a supplied PSC snapshot; never resolves filesystem paths.

use crate::IdeSnapshot;
use folio_hir::Symbol;
use folio_papyrus::{PapyrusDialect, SyntaxKind, SyntaxNode, parse};
use folio_source::{FileId, TextRange};

/// Preserve the selected semantic owner when navigating inherited external members.
pub fn referenced_symbol(view: &IdeSnapshot, file: FileId, byte: usize) -> Option<Symbol> {
    super::symbol_at(view, file, byte).map(|item| item.symbol)
}

/// External navigation is limited to script and member identities.
pub fn symbol_script(symbol: &Symbol) -> Option<&str> {
    match symbol {
        Symbol::Script(script)
        | Symbol::ParentReceiver { script }
        | Symbol::Member { script, .. }
        | Symbol::StateMember { script, .. } => Some(script),
        _ => None,
    }
}

/// Identifies declaration names and nominal references in a supplied read-only snapshot.
pub fn external_declaration_symbol(text: &str, byte: usize) -> Option<Symbol> {
    let parsed = parse(text, PapyrusDialect::Skyrim);
    let root = parsed.syntax();
    let declaration = root
        .children()
        .find(|node| node.kind() == SyntaxKind::ScriptDecl)?;
    let (owner, script_range) = name_after(&declaration, "scriptname")?;
    if script_range.start <= byte && byte < script_range.end {
        return Some(Symbol::Script(owner));
    }
    let token = root
        .descendants_with_tokens()
        .filter_map(|item| item.into_token())
        .find(|token| {
            let range = token.text_range();
            usize::from(range.start()) <= byte && byte < usize::from(range.end())
        })?;
    if token.kind() != SyntaxKind::Ident {
        return None;
    }
    let parent = token.parent()?;
    if parent.kind() == SyntaxKind::TypeRef
        || parent.kind() == SyntaxKind::ImportDecl
        || parent.kind() == SyntaxKind::ScriptDecl
    {
        let name = token.text();
        if [
            "int",
            "float",
            "bool",
            "string",
            "none",
            "import",
            "extends",
            "scriptname",
            "native",
            "conditional",
            "hidden",
        ]
        .iter()
        .any(|word| name.eq_ignore_ascii_case(word))
        {
            return None;
        }
        return Some(Symbol::Script(name.into()));
    }
    for node in root.descendants() {
        let keyword = match node.kind() {
            SyntaxKind::FunctionDecl => "function",
            SyntaxKind::EventDecl => "event",
            SyntaxKind::PropertyDecl => "property",
            SyntaxKind::VariableDecl => "",
            _ => continue,
        };
        if node.ancestors().skip(1).any(|ancestor| {
            matches!(
                ancestor.kind(),
                SyntaxKind::FunctionDecl | SyntaxKind::EventDecl | SyntaxKind::PropertyDecl
            )
        }) {
            continue;
        }
        let Some((name, range)) = name_after(&node, keyword) else {
            continue;
        };
        if !(range.start <= byte && byte < range.end) {
            continue;
        }
        let state = node
            .ancestors()
            .find(|ancestor| ancestor.kind() == SyntaxKind::StateDecl)
            .and_then(|state| name_after(&state, "state").map(|(name, _)| name));
        let symbol = if let Some(state) = state {
            Symbol::StateMember {
                script: owner,
                state,
                name,
            }
        } else {
            Symbol::Member {
                script: owner,
                name,
            }
        };
        return (external_declaration_range(text, &symbol) == Some(range)).then_some(symbol);
    }
    None
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
