//! Presentation facts recovered from the lossless declaration syntax.

use crate::{SyntaxKind, SyntaxNode, SyntaxToken};

/// Return the Papyrus `{ ... }` documentation immediately following a header.
/// Ordinary comments and documentation inside executable statements are excluded.
pub fn declaration_documentation(node: &SyntaxNode) -> Option<String> {
    let header = node
        .children_with_tokens()
        .take_while(|item| item.kind() != SyntaxKind::Block);
    for token in header.filter_map(|item| item.into_token()) {
        if let Some(doc) = documentation(&token) {
            return Some(doc);
        }
    }
    if let Some(block) = node
        .children()
        .find(|child| child.kind() == SyntaxKind::Block)
    {
        return prefix_documentation(block.children_with_tokens());
    }
    // Script headers, auto properties, variables and native callables have no block.
    // Their following comments belong to the parent CST, not to the declaration.
    prefix_documentation(node.siblings_with_tokens(rowan::Direction::Next).skip(1))
}

fn prefix_documentation(
    items: impl Iterator<Item = rowan::NodeOrToken<SyntaxNode, SyntaxToken>>,
) -> Option<String> {
    for item in items {
        let token = item.into_token()?;
        if let Some(doc) = documentation(&token) {
            return Some(doc);
        }
        if !matches!(
            token.kind(),
            SyntaxKind::Whitespace | SyntaxKind::Newline | SyntaxKind::Comment
        ) {
            break;
        }
    }
    None
}

fn documentation(token: &SyntaxToken) -> Option<String> {
    if token.kind() != SyntaxKind::Comment {
        return None;
    }
    let text = token.text().strip_prefix('{')?.strip_suffix('}')?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Preserve the complete declaration signature without executable body text.
/// Manual properties also expose the getter/setter signatures and their shape.
pub fn declaration_header(node: &SyntaxNode) -> String {
    let mut header = String::new();
    for item in node
        .children_with_tokens()
        .take_while(|item| item.kind() != SyntaxKind::Block)
    {
        let nested = item.as_node().is_some();
        let tokens: Vec<_> = match item {
            rowan::NodeOrToken::Token(token) => vec![token],
            rowan::NodeOrToken::Node(child) => child
                .descendants_with_tokens()
                .filter_map(|item| item.into_token())
                .collect(),
        };
        for token in tokens {
            match token.kind() {
                SyntaxKind::Newline if !nested => {
                    return property_header(node, header.trim().to_owned());
                }
                SyntaxKind::Newline => {
                    if !header.ends_with(' ') {
                        header.push(' ');
                    }
                }
                // Comments separate lexical words even without surrounding spaces.
                SyntaxKind::Whitespace
                | SyntaxKind::Continuation
                | SyntaxKind::Comment
                | SyntaxKind::UnclosedComment => {
                    if !header.ends_with(' ') {
                        header.push(' ');
                    }
                }
                SyntaxKind::Missing => {}
                _ => header.push_str(token.text()),
            }
        }
    }
    property_header(node, header.trim().to_owned())
}

fn property_header(node: &SyntaxNode, mut header: String) -> String {
    if node.kind() == SyntaxKind::PropertyDecl {
        if let Some(block) = node
            .children()
            .find(|child| child.kind() == SyntaxKind::Block)
        {
            for accessor in block
                .children()
                .filter(|child| child.kind() == SyntaxKind::FunctionDecl)
            {
                header.push_str("\n    ");
                header.push_str(&declaration_header(&accessor));
                if accessor
                    .children()
                    .any(|child| child.kind() == SyntaxKind::Block)
                {
                    header.push_str("\n    EndFunction");
                }
            }
            header.push_str("\nEndProperty");
        }
    }
    header
}

#[cfg(test)]
mod tests;
