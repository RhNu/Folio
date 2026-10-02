//! Literal decoding shared by lowering and editor value previews.

use crate::{SyntaxKind, SyntaxNode};

/// Extract a declaration literal while preserving spaces inside string tokens.
/// Trivia between a numeric minus and its value is not part of the stored value.
pub fn constant_literal_text(node: &SyntaxNode) -> Option<String> {
    crate::is_constant_literal(node)
        .then(|| normalize_constant_literal_text(&node.text().to_string()))
        .flatten()
}

/// Normalize literal syntax from either source or an external declaration carrier.
/// This validates shape only: type/range/escape validation belongs to the decoders.
/// An interior physical newline needs a backslash to continue the expression.
pub fn normalize_constant_literal_text(text: &str) -> Option<String> {
    let text = text.trim_matches([' ', '\t', '\r', '\n']);
    let tokens = crate::lex(text)
        .into_iter()
        .filter(|token| !token.kind.is_trivia())
        .collect::<Vec<_>>();
    match tokens.as_slice() {
        [token] if matches!(token.kind, SyntaxKind::Number | SyntaxKind::String) => {}
        [token]
            if token.kind == SyntaxKind::Ident
                && ["true", "false", "none"]
                    .iter()
                    .any(|word| token.text(text).eq_ignore_ascii_case(word)) => {}
        [minus, value] if minus.kind == SyntaxKind::Minus && value.kind == SyntaxKind::Number => {}
        _ => return None,
    }
    Some(tokens.iter().map(|token| token.text(text)).collect())
}

/// Declaration constants require their declared type, with Int-to-Float widening
/// and None reference initialization. Runtime expression conversions are separate.
pub fn constant_literal_matches_type(text: &str, ty: &str) -> bool {
    let Some(spelling) = normalize_constant_literal_text(text) else {
        return false;
    };
    let literal = crate::lex(&spelling)
        .into_iter()
        .last()
        .expect("normalized nonempty literal");
    let ty = ty.trim();
    match literal.kind {
        SyntaxKind::Number if spelling.contains('.') => {
            ty.eq_ignore_ascii_case("float") && spelling.parse::<f32>().is_ok_and(f32::is_finite)
        }
        SyntaxKind::Number => {
            (ty.eq_ignore_ascii_case("int") || ty.eq_ignore_ascii_case("float"))
                && decode_integer_literal(&spelling).is_some()
        }
        SyntaxKind::String => {
            ty.eq_ignore_ascii_case("string") && decode_string_literal(&spelling).is_some()
        }
        SyntaxKind::Ident
            if spelling.eq_ignore_ascii_case("true") || spelling.eq_ignore_ascii_case("false") =>
        {
            ty.eq_ignore_ascii_case("bool")
        }
        SyntaxKind::Ident if spelling.eq_ignore_ascii_case("none") => {
            if let Some(element) = ty.strip_suffix("[]") {
                valid_type_name(element)
            } else {
                valid_type_name(ty)
                    && !["bool", "int", "float", "string"]
                        .iter()
                        .any(|builtin| ty.eq_ignore_ascii_case(builtin))
            }
        }
        _ => false,
    }
}

fn valid_type_name(text: &str) -> bool {
    let builtin = ["bool", "int", "float", "string"]
        .iter()
        .any(|name| text.eq_ignore_ascii_case(name));
    !text.is_empty()
        && text.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && (builtin || !folio_profiles::is_skyrim_keyword(text))
}

/// Decode a signed-range Papyrus integer, including decimal minimum and hex.
/// Full-width unsigned hexadecimal bit-pattern interpretation remains unresolved.
pub fn decode_integer_literal(text: &str) -> Option<i32> {
    // Unary plus remains a Folio expression extension, while declaration
    // constant syntax is restricted to an optional numeric minus.
    let normalized = if let Some(positive) = text.trim().strip_prefix('+') {
        let value = normalize_constant_literal_text(positive)?;
        if value.starts_with('-') {
            return None;
        }
        value
    } else {
        normalize_constant_literal_text(text)?
    };
    let (negative, digits) = if let Some(digits) = normalized.strip_prefix('-') {
        (true, digits)
    } else {
        (false, normalized.as_str())
    };
    let (radix, digits) = match digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        Some(hex) => (16, hex),
        None => (10, digits),
    };
    if digits.is_empty()
        || !digits.bytes().all(|byte| match radix {
            16 => byte.is_ascii_hexdigit(),
            _ => byte.is_ascii_digit(),
        })
    {
        return None;
    }
    let magnitude = i64::from_str_radix(digits, radix).ok()?;
    let value = if negative { -magnitude } else { magnitude };
    i32::try_from(value).ok()
}

/// Decode a quoted string once, so an escaped backslash never starts another escape.
pub fn decode_string_literal(text: &str) -> Option<String> {
    let normalized = normalize_constant_literal_text(text)?;
    let body = normalized.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            if matches!(ch, '"' | '\n' | '\r') {
                return None;
            }
            out.push(ch);
            continue;
        }
        out.push(match chars.next()? {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '\\' => '\\',
            '"' => '"',
            _ => return None,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests;
