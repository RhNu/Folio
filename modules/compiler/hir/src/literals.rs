//! Literal decoding shared by lowering and editor value previews.

/// Decode a quoted string once, so an escaped backslash never starts another escape.
pub fn decode_string_literal(text: &str) -> Option<String> {
    let body = text.strip_prefix('"')?.strip_suffix('"')?;
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
