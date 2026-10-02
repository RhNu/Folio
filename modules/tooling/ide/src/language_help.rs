//! Offline, dialect-specific help for source language tokens and literal values.
use crate::Hover;
use folio_papyrus::{PapyrusDialect, SyntaxKind, SyntaxNode, SyntaxToken};
use folio_source::TextRange;

mod catalog;

/// Trusted language examples and reference URLs, separate from user-authored prose.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguageHelp {
    pub dialect: PapyrusDialect,
    pub example: Option<&'static str>,
    pub reference_url: &'static str,
}

/// Query a supplied snapshot, including dependency PSC and generated API documents.
/// This never loads files or accesses documentation websites.
pub fn language_hover(
    text: &str,
    dialect: PapyrusDialect,
    byte: usize,
) -> Option<(Hover, TextRange)> {
    if !text.is_char_boundary(byte) {
        return None;
    }
    let parse = folio_papyrus::parse(text, dialect);
    at(&parse.syntax(), dialect, byte)
}

pub(crate) fn token_at(root: &SyntaxNode, byte: usize) -> Option<SyntaxToken> {
    if byte >= usize::from(root.text_range().end()) {
        return None;
    }
    let token = root.token_at_offset(byte.try_into().ok()?).right_biased()?;
    let range = crate::symbols::token_range(&token);
    (range.start <= byte && byte < range.end).then_some(token)
}

pub(crate) fn at(
    root: &SyntaxNode,
    dialect: PapyrusDialect,
    byte: usize,
) -> Option<(Hover, TextRange)> {
    let token = token_at(root, byte)?;
    let parent = token.parent()?;
    let word = token.text().to_ascii_lowercase();
    let literal = match token.kind() {
        SyntaxKind::String => Some("string"),
        SyntaxKind::Number if word.starts_with("0x") || !word.contains('.') => Some("int"),
        SyntaxKind::Number => Some("float"),
        SyntaxKind::Ident if parent.kind() == SyntaxKind::LiteralExpr => match word.as_str() {
            "true" | "false" => Some("bool"),
            "none" => Some("none"),
            _ => None,
        },
        _ => None,
    };
    let entry = match dialect {
        PapyrusDialect::Skyrim => {
            if let Some(ty) = literal {
                catalog::skyrim(ty, parent.kind())?
            } else if token.kind() == SyntaxKind::Ident {
                // Flags are not reserved identifiers. Only explain their declaration use.
                if matches!(word.as_str(), "hidden" | "conditional") && !standard_flag(&token) {
                    return None;
                }
                if matches!(word.as_str(), "self" | "parent")
                    && parent.kind() != SyntaxKind::NameExpr
                {
                    return None;
                }
                catalog::skyrim(&word, parent.kind())?
            } else {
                catalog::operator(token.kind(), parent.kind() == SyntaxKind::UnaryExpr)?
            }
        }
    };
    let mut range = crate::symbols::token_range(&token);
    let mut declaration = entry.title.to_owned();
    let mut details = Vec::new();
    let mut literal_valid = None;
    if let Some(ty) = literal {
        let sign = if token.kind() == SyntaxKind::Number {
            negative_literal(&parent)
        } else {
            None
        };
        let negative = sign.is_some();
        if let Some(sign) = sign {
            range.start = sign.start;
        }
        let value = match ty {
            "string" => folio_hir::decode_string_literal(token.text()).map(|value| {
                // Keep decoded control characters visible without turning them into layout.
                format!("\"{}\"", value.escape_debug())
            }),
            "int" => integer_value(token.text(), negative).map(|value| value.to_string()),
            "float" => token
                .text()
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
                .map(|value| if negative { -value } else { value })
                .map(|value| value.to_string()),
            "bool" => Some(if word == "true" { "True" } else { "False" }.into()),
            "none" => Some("None".into()),
            _ => None,
        };
        literal_valid = Some(value.is_some());
        details.push(value.map_or_else(
            || format!("Invalid {} literal; no value is available.", entry.title),
            |value| format!("Value of literal: {value}"),
        ));
    } else if parent.kind() == SyntaxKind::TypeRef {
        // A built-in array type still documents its element type, while keeping [] visible.
        if parent
            .children_with_tokens()
            .filter_map(|item| item.into_token())
            .any(|item| item.kind() == SyntaxKind::LBracket)
            && token.kind() == SyntaxKind::Ident
        {
            declaration.push_str("[]");
            details.push("One-dimensional array; its default reference is None.".into());
        }
    }
    tracing::debug!(
        ?dialect,
        start = range.start,
        end = range.end,
        literal = literal.is_some(),
        ?literal_valid,
        "resolved language hover"
    );
    Some((
        Hover {
            content: declaration.clone(),
            symbol: None,
            declaration,
            documentation: Some(entry.description.into()),
            language: Some(LanguageHelp {
                dialect,
                example: if literal.is_some() {
                    None
                } else {
                    entry.example
                },
                reference_url: entry.reference_url,
            }),
            details,
            span: None,
            owner_script: None,
        },
        range,
    ))
}

/// Only a direct unary minus belongs to a number; subtraction and ! do not.
fn negative_literal(literal: &SyntaxNode) -> Option<TextRange> {
    let parent = literal.parent()?;
    if parent.kind() != SyntaxKind::UnaryExpr {
        return None;
    }
    let sign = parent
        .children_with_tokens()
        .filter_map(|item| item.into_token())
        .find(|token| {
            !matches!(
                token.kind(),
                SyntaxKind::Whitespace | SyntaxKind::Comment | SyntaxKind::Continuation
            )
        })?;
    (sign.kind() == SyntaxKind::Minus).then(|| crate::symbols::token_range(&sign))
}

/// Preview exactly the integer representation accepted by generation.
fn integer_value(text: &str, negative: bool) -> Option<i32> {
    folio_hir::decode_integer_literal(&if negative {
        format!("-{text}")
    } else {
        text.to_owned()
    })
}

/// The parser keeps flag words as Ident tokens, just like declaration names.
fn standard_flag(token: &SyntaxToken) -> bool {
    let Some(parent) = token.parent() else {
        return false;
    };
    let kind = parent.kind();
    if !matches!(
        kind,
        SyntaxKind::ScriptDecl | SyntaxKind::PropertyDecl | SyntaxKind::VariableDecl
    ) {
        return false;
    }
    if kind == SyntaxKind::VariableDecl
        && (token.text().eq_ignore_ascii_case("hidden")
            || parent.ancestors().any(|node| {
                matches!(
                    node.kind(),
                    SyntaxKind::FunctionDecl | SyntaxKind::EventDecl
                )
            }))
    {
        return false;
    }
    let words = parent
        .children_with_tokens()
        .filter_map(|item| item.into_token())
        .take_while(|item| item.kind() != SyntaxKind::Newline)
        .filter(|item| item.kind() == SyntaxKind::Ident)
        .collect::<Vec<_>>();
    let skip = match kind {
        SyntaxKind::ScriptDecl
            if words
                .get(2)
                .is_some_and(|item| item.text().eq_ignore_ascii_case("extends")) =>
        {
            4
        }
        SyntaxKind::ScriptDecl | SyntaxKind::PropertyDecl => 2,
        SyntaxKind::VariableDecl => 1,
        _ => return false,
    };
    words.iter().skip(skip).any(|item| item == token)
}

#[cfg(test)]
mod tests;
