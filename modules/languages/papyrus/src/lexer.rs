use crate::SyntaxKind;
use folio_source::TextRange;

/// One token with its original UTF-8 byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexToken {
    pub kind: SyntaxKind,
    pub range: TextRange,
}

impl LexToken {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.range.start..self.range.end]
    }
}

/// Lexes every byte, including malformed strings and otherwise unknown characters.
pub fn lex(source: &str) -> Vec<LexToken> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    let mut continued_newline = None;
    while offset < source.len() {
        let remaining = &source[offset..];
        let first = remaining.chars().next().expect("nonempty remainder");
        let (kind, len) = match first {
            '\r' | '\n' => {
                let kind = if continued_newline == Some(offset) {
                    continued_newline = None;
                    SyntaxKind::Continuation
                } else {
                    SyntaxKind::Newline
                };
                (kind, if remaining.starts_with("\r\n") { 2 } else { 1 })
            }
            ' ' | '\t' => (
                SyntaxKind::Whitespace,
                remaining
                    .bytes()
                    .take_while(|b| *b == b' ' || *b == b'\t')
                    .count(),
            ),
            ';' if remaining.starts_with(";/") => match remaining.find("/;") {
                Some(end) => (SyntaxKind::Comment, end + 2),
                None => (SyntaxKind::UnclosedComment, remaining.len()),
            },
            ';' => (
                SyntaxKind::Comment,
                remaining.find(['\r', '\n']).unwrap_or(remaining.len()),
            ),
            '{' => match remaining.find('}') {
                Some(end) => (SyntaxKind::Comment, end + 1),
                None => (SyntaxKind::UnclosedComment, remaining.len()),
            },
            '\\' => {
                if let Some(newline) = continuation_newline(remaining) {
                    // Preserve a trailing comment as its own token. Only the
                    // physical newline loses its statement-boundary meaning.
                    continued_newline = Some(offset + newline);
                    (SyntaxKind::Continuation, 1)
                } else {
                    (SyntaxKind::Unknown, 1)
                }
            }
            '"' => {
                let mut escaped = false;
                let mut end = None;
                for (index, ch) in remaining.char_indices().skip(1) {
                    if ch == '\r' || ch == '\n' {
                        break;
                    }
                    if ch == '"' && !escaped {
                        end = Some(index + ch.len_utf8());
                        break;
                    }
                    escaped = ch == '\\' && !escaped;
                }
                match end {
                    Some(end) => (SyntaxKind::String, end),
                    None => (
                        SyntaxKind::UnclosedString,
                        remaining.find(['\r', '\n']).unwrap_or(remaining.len()),
                    ),
                }
            }
            '0'..='9' => {
                let mut len = if remaining.starts_with("0x") || remaining.starts_with("0X") {
                    let digits = remaining.as_bytes()[2..]
                        .iter()
                        .take_while(|byte| byte.is_ascii_hexdigit())
                        .count();
                    if digits > 0 { 2 + digits } else { 1 }
                } else {
                    remaining.bytes().take_while(u8::is_ascii_digit).count()
                };
                if remaining.as_bytes().get(len) == Some(&b'.') {
                    let fractional = remaining.as_bytes()[len + 1..]
                        .iter()
                        .take_while(|byte| byte.is_ascii_digit())
                        .count();
                    if fractional > 0 {
                        len += 1 + fractional;
                    }
                }
                (SyntaxKind::Number, len)
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let len = remaining
                    .bytes()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    .count();
                (SyntaxKind::Ident, len)
            }
            '(' => (SyntaxKind::LParen, 1),
            ')' => (SyntaxKind::RParen, 1),
            '[' => (SyntaxKind::LBracket, 1),
            ']' => (SyntaxKind::RBracket, 1),
            ',' => (SyntaxKind::Comma, 1),
            '.' => (SyntaxKind::Dot, 1),
            '=' if remaining.starts_with("==") => (SyntaxKind::EqEq, 2),
            '=' => (SyntaxKind::Equals, 1),
            '+' if remaining.starts_with("+=") => (SyntaxKind::PlusEq, 2),
            '-' if remaining.starts_with("-=") => (SyntaxKind::MinusEq, 2),
            '*' if remaining.starts_with("*=") => (SyntaxKind::StarEq, 2),
            '/' if remaining.starts_with("/=") => (SyntaxKind::SlashEq, 2),
            '%' if remaining.starts_with("%=") => (SyntaxKind::PercentEq, 2),
            '!' if remaining.starts_with("!=") => (SyntaxKind::NotEq, 2),
            '<' if remaining.starts_with("<=") => (SyntaxKind::LessEq, 2),
            '>' if remaining.starts_with(">=") => (SyntaxKind::GreaterEq, 2),
            '&' if remaining.starts_with("&&") => (SyntaxKind::AndAnd, 2),
            '|' if remaining.starts_with("||") => (SyntaxKind::OrOr, 2),
            '+' => (SyntaxKind::Plus, 1),
            '-' => (SyntaxKind::Minus, 1),
            '*' => (SyntaxKind::Star, 1),
            '/' => (SyntaxKind::Slash, 1),
            '%' => (SyntaxKind::Percent, 1),
            '<' => (SyntaxKind::Less, 1),
            '>' => (SyntaxKind::Greater, 1),
            '!' => (SyntaxKind::Bang, 1),
            _ => (SyntaxKind::Unknown, first.len_utf8()),
        };
        let end = offset + len;
        tokens.push(LexToken {
            kind,
            range: TextRange { start: offset, end },
        });
        offset = end;
    }
    tokens
}

fn continuation_newline(text: &str) -> Option<usize> {
    let mut offset = 1;
    loop {
        offset += text[offset..]
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let tail = &text[offset..];
        if tail.starts_with(['\r', '\n']) {
            return Some(offset);
        }
        if let Some(comment) = tail.strip_prefix(";/") {
            let end = comment.find("/;")?;
            // A multiline block comment itself ends the code line.
            if comment[..end].contains(['\r', '\n']) {
                return None;
            }
            offset += end + 4;
        } else if tail.starts_with(';') {
            return tail.find(['\r', '\n']).map(|end| offset + end);
        } else {
            return None;
        }
    }
}

#[cfg(test)]
mod tests;
