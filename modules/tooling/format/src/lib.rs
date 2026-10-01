//! Conservative, lossless Papyrus source formatting over the parsed CST.

use folio_papyrus::{PapyrusDialect, SyntaxError, SyntaxKind, SyntaxToken, parse};

const INDENT: &str = "    ";

/// A source file that cannot be formatted without guessing its structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    Syntax(Vec<SyntaxError>),
    ChangedTokens,
}

/// Formats supported Papyrus syntax while retaining every nontrivia token.
#[tracing::instrument(skip(source), fields(bytes = source.len()))]
pub fn format_source(source: &str, dialect: PapyrusDialect) -> Result<String, FormatError> {
    let parsed = parse(source, dialect);
    if !parsed.errors.is_empty() {
        tracing::debug!(
            errors = parsed.errors.len(),
            "formatting rejected invalid syntax"
        );
        return Err(FormatError::Syntax(parsed.errors));
    }
    let tokens = parsed
        .syntax()
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .collect::<Vec<_>>();
    let mut formatted = String::with_capacity(source.len());
    let mut offset = 0;
    while offset < source.len() {
        let (content_end, line_end) = line_end(source, offset);
        let line = tokens
            .iter()
            .filter(|token| {
                let range = token.text_range();
                usize::from(range.start()) < content_end && usize::from(range.end()) > offset
            })
            .collect::<Vec<_>>();
        // A token spanning lines owns its internal bytes; never rewrite part of it.
        let protected = line.iter().any(|token| {
            let range = token.text_range();
            usize::from(range.start()) < offset || usize::from(range.end()) > content_end
        }) || tokens.iter().any(|token| {
            token.kind() == SyntaxKind::Continuation
                && usize::from(token.text_range().end()) == offset
        });
        if protected {
            formatted.push_str(&source[offset..content_end]);
        } else {
            render_line(&mut formatted, &line);
        }
        formatted.push_str(&source[content_end..line_end]);
        offset = line_end;
    }
    if !formatted.is_empty() && !formatted.ends_with(['\r', '\n']) {
        formatted.push_str(preferred_newline(source));
    }
    let check = parse(&formatted, dialect);
    if !check.errors.is_empty()
        || significant(&tokens)
            != significant(
                &check
                    .syntax()
                    .descendants_with_tokens()
                    .filter_map(|element| element.into_token())
                    .collect::<Vec<_>>(),
            )
    {
        tracing::warn!("formatted candidate changed syntax tokens");
        return Err(FormatError::ChangedTokens);
    }
    tracing::debug!(changed = formatted != source, "source formatted");
    Ok(formatted)
}

fn line_end(source: &str, start: usize) -> (usize, usize) {
    let bytes = source.as_bytes();
    let mut end = start;
    while end < bytes.len() && bytes[end] != b'\n' && bytes[end] != b'\r' {
        end += 1;
    }
    let after = if bytes.get(end) == Some(&b'\r') && bytes.get(end + 1) == Some(&b'\n') {
        end + 2
    } else if end < bytes.len() {
        end + 1
    } else {
        end
    };
    (end, after)
}

fn preferred_newline(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn significant(tokens: &[SyntaxToken]) -> Vec<(SyntaxKind, String)> {
    tokens
        .iter()
        .filter(|token| !matches!(token.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline))
        .map(|token| (token.kind(), token.text().to_owned()))
        .collect()
}

fn render_line(output: &mut String, tokens: &[&SyntaxToken]) {
    let meaningful = tokens
        .iter()
        .copied()
        .filter(|token| !matches!(token.kind(), SyntaxKind::Whitespace | SyntaxKind::Newline))
        .collect::<Vec<_>>();
    let Some(first) = meaningful.first() else {
        return;
    };
    let mut depth = first
        .parent()
        .into_iter()
        .flat_map(|parent| parent.ancestors())
        .filter(|node| node.kind() == SyntaxKind::Block)
        .count();
    if first.kind() == SyntaxKind::Ident
        && ["endfunction", "endevent", "endstate", "endproperty"]
            .iter()
            .any(|word| first.text().eq_ignore_ascii_case(word))
    {
        depth = depth.saturating_sub(1);
    }
    for _ in 0..depth {
        output.push_str(INDENT);
    }
    let mut previous: Option<&SyntaxToken> = None;
    for token in meaningful {
        if let Some(before) = previous
            && needs_space(before, token)
        {
            output.push(' ');
        }
        output.push_str(token.text());
        previous = Some(token);
    }
}

fn needs_space(previous: &SyntaxToken, current: &SyntaxToken) -> bool {
    let left = previous.kind();
    let right = current.kind();
    if matches!(
        right,
        SyntaxKind::Comma | SyntaxKind::RParen | SyntaxKind::RBracket | SyntaxKind::Dot
    ) {
        return false;
    }
    if matches!(
        left,
        SyntaxKind::LParen | SyntaxKind::LBracket | SyntaxKind::Dot | SyntaxKind::Bang
    ) {
        return false;
    }
    if matches!(right, SyntaxKind::LParen | SyntaxKind::LBracket) {
        return right == SyntaxKind::LParen
            && ["if", "elseif", "while", "return"]
                .iter()
                .any(|word| previous.text().eq_ignore_ascii_case(word));
    }
    if matches!(left, SyntaxKind::Plus | SyntaxKind::Minus)
        && previous
            .parent()
            .is_some_and(|node| node.kind() == SyntaxKind::UnaryExpr)
    {
        return false;
    }
    true
}
