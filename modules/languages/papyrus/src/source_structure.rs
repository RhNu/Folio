//! Source-wide rules that do not affect the recoverable CST shape.

use crate::{SyntaxError, SyntaxErrorKind, SyntaxKind, SyntaxNode};
use folio_source::TextRange;
use std::collections::HashSet;

pub(crate) fn validate(root: &SyntaxNode, source: &str, errors: &mut Vec<SyntaxError>) {
    let headers = root
        .children()
        .filter(|node| node.kind() == SyntaxKind::ScriptDecl)
        .collect::<Vec<_>>();
    if headers.is_empty() {
        let start = root
            .children()
            .next()
            .map_or(source.len(), |node| usize::from(node.text_range().start()));
        errors.push(SyntaxError {
            kind: SyntaxErrorKind::ExpectedDeclaration,
            range: TextRange { start, end: start },
            message: "expected ScriptName header".into(),
        });
    }
    for (index, header) in headers.iter().enumerate() {
        if index > 0 || root.children().next().as_ref() != Some(header) {
            errors.push(SyntaxError {
                kind: SyntaxErrorKind::ExpectedDeclaration,
                range: span(header.text_range()),
                message: "ScriptName must be the first and only script header".into(),
            });
        }
    }

    let permitted_docs: HashSet<_> = root
        .descendants()
        .filter_map(|node| crate::documentation::declaration_documentation_token(&node))
        .map(|token| token.text_range().start())
        .collect();
    for token in root
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| {
            matches!(
                token.kind(),
                SyntaxKind::Comment | SyntaxKind::UnclosedComment
            ) && token.text().starts_with('{')
        })
    {
        let end = usize::from(token.text_range().end());
        let rest_of_line = source[end..].split(['\r', '\n']).next().unwrap_or("");
        if !permitted_docs.contains(&token.text_range().start())
            || !rest_of_line.trim_matches([' ', '\t']).is_empty()
        {
            errors.push(SyntaxError {
                kind: SyntaxErrorKind::UnexpectedText,
                range: span(token.text_range()),
                message: "documentation must occupy the line immediately following a script, property, function or event header".into(),
            });
        }
    }
}

fn span(range: rowan::TextRange) -> TextRange {
    TextRange {
        start: usize::from(range.start()),
        end: usize::from(range.end()),
    }
}
