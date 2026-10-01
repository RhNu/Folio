//! Count physical lines containing Rust code, excluding comments and blank lines.

use rustc_lexer::{FrontmatterAllowed, TokenKind, is_whitespace, strip_shebang, tokenize};

pub const WARN_LINES: usize = 650;
pub const ERROR_LINES: usize = 1200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warning,
    Error,
}

impl Level {
    /// Limits are inclusive: only exceeding a limit raises its severity.
    pub fn for_lines(lines: usize) -> Self {
        if lines > ERROR_LINES {
            Self::Error
        } else if lines > WARN_LINES {
            Self::Warning
        } else {
            Self::Ok
        }
    }
}

/// Lex complete Rust tokens so literal contents never become comments.
/// Count each occupied physical line once, including lines in multiline literals,
/// while excluding blank literal lines. Malformed code remains countable.
pub fn code_lines(source: &str) -> usize {
    // rustc removes a UTF-8 BOM and a script shebang before tokenization.
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let source = &source[strip_shebang(source).unwrap_or(0)..];
    let mut remaining = source;
    let mut count = 0;
    let mut occupied = false;
    for token in tokenize(source, FrontmatterAllowed::No) {
        let ignored = matches!(
            token.kind,
            TokenKind::Whitespace | TokenKind::LineComment { .. } | TokenKind::BlockComment { .. }
        );
        let (text, rest) = remaining.split_at(token.len as usize);
        remaining = rest;
        for character in text.chars() {
            if character == '\n' {
                count += usize::from(occupied);
                occupied = false;
            } else if !ignored && !is_whitespace(character) {
                occupied = true;
            }
        }
    }
    count + usize::from(occupied)
}

#[cfg(test)]
mod tests;
