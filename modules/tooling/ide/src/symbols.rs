//! Editor symbol views derived from the parser and the shared typed HIR.

use folio_build::ProjectAnalysisView;
use folio_hir::{ExpressionKind, MemberFact, MemberKind, Script, Symbol, Type};
use folio_papyrus::{SyntaxKind, SyntaxNode};
use folio_source::{FileId, SourceSpan, TextRange};

use crate::display_type;

fn contains(span: SourceSpan, byte: usize) -> bool {
    span.range.start <= byte && byte < span.range.end
}

fn node_range(node: &SyntaxNode) -> TextRange {
    let range = node.text_range();
    TextRange {
        start: usize::from(range.start()),
        end: usize::from(range.end()),
    }
}

fn token_range(token: &folio_papyrus::SyntaxToken) -> TextRange {
    let range = token.text_range();
    TextRange {
        start: usize::from(range.start()),
        end: usize::from(range.end()),
    }
}

/// Returns the script that owns a bound symbol, including locals and parameters.
pub(crate) fn owner_script(symbol: &Symbol) -> Option<String> {
    match symbol {
        Symbol::Script(name) => Some(name.clone()),
        Symbol::Member { script, .. }
        | Symbol::StateMember { script, .. }
        | Symbol::PropertyAccessor { script, .. } => Some(script.clone()),
        Symbol::Parameter { owner, .. } | Symbol::Local { owner, .. } => owner_script(owner),
        Symbol::Intrinsic { .. } => None,
    }
}

fn symbol_name(symbol: &Symbol) -> &str {
    match symbol {
        Symbol::Script(name) => name,
        Symbol::Intrinsic { name }
        | Symbol::Member { name, .. }
        | Symbol::StateMember { name, .. }
        | Symbol::PropertyAccessor { name, .. }
        | Symbol::Parameter { name, .. }
        | Symbol::Local { name, .. } => name,
    }
}

fn find_member<'a>(script: &'a Script, symbol: &Symbol) -> Option<&'a MemberFact> {
    script
        .members
        .iter()
        .chain(&script.external_members)
        .chain(&script.referenced_members)
        .find(|member| &member.symbol == symbol)
}

fn signature(member: &MemberFact) -> String {
    let name = symbol_name(&member.symbol);
    let args = member
        .parameters
        .iter()
        .map(|parameter| {
            let default = parameter
                .default_literal
                .as_ref()
                .map_or(String::new(), |value| format!(" = {value}"));
            format!(
                "{} {}{default}",
                display_type(&parameter.ty),
                parameter.name
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let base = match member.kind {
        MemberKind::Function { event: true, .. } => format!("Event {name}({args})"),
        MemberKind::Function { .. } if member.ty == Type::Void => {
            format!("Function {name}({args})")
        }
        MemberKind::Function { .. } => {
            format!("{} Function {name}({args})", display_type(&member.ty))
        }
        MemberKind::Property { .. } => format!("{} {name} Property", display_type(&member.ty)),
        MemberKind::Variable => format!("{} {name}", display_type(&member.ty)),
    };
    if member.flags.is_empty() {
        base
    } else {
        format!("{base} {}", member.flags.join(" "))
    }
}

/// Formats a resolved symbol with its name and callable shape where available.
pub(crate) fn describe_symbol(script: &Script, symbol: &Symbol, ty: &Type) -> String {
    let owner = owner_script(symbol);
    let header = if let Some(member) = find_member(script, symbol) {
        signature(member)
    } else {
        match symbol {
            Symbol::Script(name) => format!("script {name}"),
            Symbol::Intrinsic { name } if name.eq_ignore_ascii_case("GetState") => {
                "String Function GetState()".into()
            }
            Symbol::Intrinsic { name } if name.eq_ignore_ascii_case("GotoState") => {
                "Function GotoState(String stateName)".into()
            }
            _ => format!("{}: {}", symbol_name(symbol), display_type(ty)),
        }
    };
    match symbol {
        Symbol::Member { .. } | Symbol::StateMember { .. } | Symbol::PropertyAccessor { .. } => {
            format!("{}\nDeclared in {}", header, owner.unwrap_or_default())
        }
        Symbol::Parameter { .. } => format!("parameter {header}"),
        Symbol::Local { .. } => format!("local {header}"),
        _ => header,
    }
}

/// Finds a script name written as a type, import, or parent without inventing a binding.
pub(crate) fn script_reference(
    view: &ProjectAnalysisView,
    file: FileId,
    byte: usize,
) -> Option<(String, SourceSpan)> {
    let parse = view.analysis.parse(file)?;
    let token = parse
        .syntax()
        .descendants_with_tokens()
        .filter_map(|item| item.into_token())
        .find(|token| {
            let range = token_range(token);
            range.start <= byte && byte < range.end
        })?;
    if token.kind() != SyntaxKind::Ident {
        return None;
    }
    let parent = token.parent()?;
    let name = token.text();
    let eligible = parent.kind() == SyntaxKind::TypeRef
        || (parent.kind() == SyntaxKind::ImportDecl && !name.eq_ignore_ascii_case("import"))
        || (parent.kind() == SyntaxKind::ScriptDecl
            && view
                .analysis
                .hir(file)?
                .parent
                .as_ref()
                .is_some_and(|item| contains(item.span, byte)));
    if !eligible
        || ["int", "float", "bool", "string", "none"]
            .iter()
            .any(|item| name.eq_ignore_ascii_case(item))
    {
        return None;
    }
    Some((
        name.to_string(),
        SourceSpan {
            file,
            range: token_range(&token),
        },
    ))
}

/// Navigates only to selected PSC source spans; external declaration carriers have no FileId.
pub fn source_declaration(
    view: &ProjectAnalysisView,
    file: FileId,
    byte: usize,
) -> Option<SourceSpan> {
    if let Some(span) = view.analysis.definition(file, byte) {
        return Some(span);
    }
    let script = view.analysis.hir(file)?;
    if let Some(name) = &script.name
        && contains(name.span, byte)
    {
        return Some(name.span);
    }
    if let Some(declaration) = script
        .declarations
        .iter()
        .find(|item| contains(item.span, byte))
    {
        return Some(declaration.span);
    }
    let (target_name, _) = script_reference(view, file, byte)?;
    for candidate in view.analysis.file_ids() {
        let Some(other) = view.analysis.hir(candidate) else {
            continue;
        };
        if let Some(name) = &other.name
            && name.text.eq_ignore_ascii_case(&target_name)
        {
            return Some(name.span);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticTokenKind {
    Class,
    Type,
    Namespace,
    Function,
    Method,
    Event,
    Property,
    Variable,
    Parameter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticToken {
    pub range: TextRange,
    pub kind: SemanticTokenKind,
    pub declaration: bool,
    pub readonly: bool,
}

fn member_kind(script: &Script, symbol: &Symbol) -> SemanticTokenKind {
    match find_member(script, symbol).map(|item| &item.kind) {
        Some(MemberKind::Function { event: true, .. }) => SemanticTokenKind::Event,
        Some(MemberKind::Function { global: true, .. }) => SemanticTokenKind::Function,
        Some(MemberKind::Function { .. }) => SemanticTokenKind::Method,
        Some(MemberKind::Property { .. }) => SemanticTokenKind::Property,
        Some(MemberKind::Variable) => SemanticTokenKind::Variable,
        None => SemanticTokenKind::Variable,
    }
}

fn classify(script: &Script, symbol: &Symbol) -> SemanticTokenKind {
    match symbol {
        Symbol::Script(_) => SemanticTokenKind::Class,
        Symbol::Intrinsic { .. } => SemanticTokenKind::Method,
        Symbol::Member { .. } | Symbol::StateMember { .. } | Symbol::PropertyAccessor { .. } => {
            member_kind(script, symbol)
        }
        Symbol::Parameter { .. } => SemanticTokenKind::Parameter,
        Symbol::Local { .. } => SemanticTokenKind::Variable,
    }
}

/// Classifies identifiers using typed bindings, plus parser roles for type names.
pub fn semantic_tokens(view: &ProjectAnalysisView, file: FileId) -> Vec<SemanticToken> {
    let Some(script) = view.analysis.hir(file) else {
        return Vec::new();
    };
    let mut tokens = std::collections::BTreeMap::<(usize, usize), SemanticToken>::new();
    let mut add = |range: TextRange, kind, declaration, readonly| {
        if range.start < range.end {
            tokens.insert(
                (range.start, range.end),
                SemanticToken {
                    range,
                    kind,
                    declaration,
                    readonly,
                },
            );
        }
    };
    if let Some(parse) = view.analysis.parse(file) {
        for node in parse.syntax().descendants() {
            if node.kind() == SyntaxKind::TypeRef || node.kind() == SyntaxKind::ImportDecl {
                for token in node
                    .children_with_tokens()
                    .filter_map(|item| item.into_token())
                {
                    if token.kind() != SyntaxKind::Ident
                        || token.text().eq_ignore_ascii_case("import")
                    {
                        continue;
                    }
                    let kind = if ["int", "float", "bool", "string"]
                        .iter()
                        .any(|name| token.text().eq_ignore_ascii_case(name))
                    {
                        SemanticTokenKind::Type
                    } else {
                        SemanticTokenKind::Class
                    };
                    add(token_range(&token), kind, false, false);
                }
            } else if node.kind() == SyntaxKind::StateDecl
                && let Some((_, range)) = declaration_name(&node, "state")
            {
                add(range, SemanticTokenKind::Namespace, true, false);
            }
        }
    }
    if let Some(name) = &script.name {
        add(name.span.range, SemanticTokenKind::Class, true, false);
    }
    if let Some(parent) = &script.parent {
        add(parent.span.range, SemanticTokenKind::Class, false, false);
    }
    for declaration in &script.declarations {
        let kind = classify(&script, &declaration.symbol);
        let readonly = find_member(&script, &declaration.symbol).is_some_and(|item| {
            matches!(
                item.kind,
                MemberKind::Property {
                    read_only: true,
                    ..
                }
            )
        });
        add(declaration.span.range, kind, true, readonly);
    }
    for expression in &script.expressions {
        if let Some(binding) = &expression.binding {
            let kind = classify(&script, &binding.symbol);
            let readonly = find_member(&script, &binding.symbol).is_some_and(|item| {
                matches!(
                    item.kind,
                    MemberKind::Property {
                        read_only: true,
                        ..
                    }
                )
            });
            add(binding.name.span.range, kind, false, readonly);
        }
    }
    tokens.into_values().collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentSymbol {
    pub name: String,
    pub detail: String,
    pub kind: u32,
    pub range: TextRange,
    pub selection_range: TextRange,
    pub children: Vec<DocumentSymbol>,
}

fn declaration_name(node: &SyntaxNode, keyword: &str) -> Option<(String, TextRange)> {
    let mut after = false;
    for token in node
        .children_with_tokens()
        .filter_map(|item| item.into_token())
    {
        if token.kind() != SyntaxKind::Ident {
            continue;
        }
        if after {
            let range = token.text_range();
            return Some((
                token.text().to_string(),
                TextRange {
                    start: usize::from(range.start()),
                    end: usize::from(range.end()),
                },
            ));
        }
        after = token.text().eq_ignore_ascii_case(keyword);
    }
    None
}

fn node_symbol(node: &SyntaxNode, script: &Script) -> Option<DocumentSymbol> {
    let (mut kind, keyword) = match node.kind() {
        SyntaxKind::ScriptDecl => (5, "scriptname"),
        SyntaxKind::StateDecl => (3, "state"),
        SyntaxKind::FunctionDecl => (6, "function"),
        SyntaxKind::EventDecl => (24, "event"),
        SyntaxKind::PropertyDecl => (7, "property"),
        SyntaxKind::VariableDecl => (13, ""),
        _ => return None,
    };
    let (name, selection_range) = if keyword.is_empty() {
        let token = node
            .children_with_tokens()
            .filter_map(|item| item.into_token())
            .find(|token| token.kind() == SyntaxKind::Ident)?;
        let range = token.text_range();
        (
            token.text().to_string(),
            TextRange {
                start: usize::from(range.start()),
                end: usize::from(range.end()),
            },
        )
    } else {
        declaration_name(node, keyword)?
    };
    let member = script
        .members
        .iter()
        .find(|member| member.span.range == selection_range);
    if member.is_some_and(|item| matches!(item.kind, MemberKind::Function { global: true, .. })) {
        kind = 12;
    }
    let detail = member.map(signature).unwrap_or_default();
    let children =
        if node.kind() == SyntaxKind::StateDecl || node.kind() == SyntaxKind::PropertyDecl {
            node.descendants()
                .skip(1)
                .filter_map(|child| {
                    (child.parent().is_some_and(|parent| {
                        parent.kind() == node.kind() || parent.kind() == SyntaxKind::Block
                    }))
                    .then(|| node_symbol(&child, script))
                    .flatten()
                })
                .collect()
        } else {
            Vec::new()
        };
    Some(DocumentSymbol {
        name,
        detail,
        kind,
        range: node_range(node),
        selection_range,
        children,
    })
}

/// Builds a source-order outline from declarations in the parsed document.
pub fn document_symbols(view: &ProjectAnalysisView, file: FileId) -> Vec<DocumentSymbol> {
    let Some(parse) = view.analysis.parse(file) else {
        return Vec::new();
    };
    let Some(script) = view.analysis.hir(file) else {
        return Vec::new();
    };
    let mut symbols = parse
        .syntax()
        .children()
        .filter_map(|node| node_symbol(&node, &script))
        .collect::<Vec<_>>();
    if let Some(index) = symbols.iter().position(|item| item.kind == 5) {
        let mut root = symbols.remove(index);
        root.range = TextRange {
            start: 0,
            end: view.analysis.text(file).map_or(root.range.end, str::len),
        };
        root.children = symbols;
        vec![root]
    } else {
        symbols
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignatureInfo {
    pub label: String,
    pub parameters: Vec<String>,
    pub active_parameter: usize,
}

/// Uses the innermost typed call and its declaration-order argument map.
pub fn signature_help(
    view: &ProjectAnalysisView,
    file: FileId,
    byte: usize,
) -> Option<SignatureInfo> {
    let script = view.analysis.hir(file)?;
    let call = script
        .expressions
        .iter()
        .filter(|fact| {
            fact.span.range.start <= byte
                && byte <= fact.span.range.end
                && matches!(fact.kind, ExpressionKind::Call { .. })
        })
        .min_by_key(|fact| fact.span.range.end - fact.span.range.start)?;
    let ExpressionKind::Call {
        callee,
        arguments,
        argument_ordinals,
        ..
    } = &call.kind
    else {
        return None;
    };
    let binding = callee.binding.as_ref()?;
    let member = find_member(&script, &binding.symbol)?;
    if !matches!(member.kind, MemberKind::Function { .. }) {
        return None;
    }
    let source_index = arguments
        .iter()
        .take_while(|arg| arg.span.range.end < byte)
        .count();
    let active_parameter = argument_ordinals
        .get(source_index)
        .copied()
        .unwrap_or(source_index)
        .min(member.parameters.len().saturating_sub(1));
    Some(SignatureInfo {
        label: signature(member),
        parameters: member
            .parameters
            .iter()
            .map(|item| {
                let default = item
                    .default_literal
                    .as_ref()
                    .map_or(String::new(), |value| format!(" = {value}"));
                format!("{} {}{default}", display_type(&item.ty), item.name)
            })
            .collect(),
        active_parameter,
    })
}
