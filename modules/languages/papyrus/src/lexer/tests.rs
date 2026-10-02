use super::*;

#[test]
fn continuations_preserve_trailing_comment_tokens() {
    for ending in ["\n", "\r\n", "\r"] {
        let source = format!("3 + 4 \\ ; note{ending}+ 5{ending}");
        let tokens = lex(&source);
        assert_eq!(
            tokens
                .iter()
                .map(|token| token.text(&source))
                .collect::<String>(),
            source
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.kind == SyntaxKind::Comment && token.text(&source) == "; note")
        );
        assert_eq!(
            tokens
                .iter()
                .filter(|token| token.kind == SyntaxKind::Newline)
                .count(),
            1
        );
    }
}

#[test]
fn line_comments_do_not_create_continuations() {
    let source = "3 + 4 ; note \\\n+ 5\n";
    let tokens = lex(source);
    assert_eq!(
        tokens
            .iter()
            .filter(|token| token.kind == SyntaxKind::Newline)
            .count(),
        2
    );
    assert!(
        !tokens
            .iter()
            .any(|token| token.kind == SyntaxKind::Continuation)
    );
}

#[test]
fn same_line_block_comments_can_follow_a_continuation() {
    let source = "3 + 4 \\ ;/ note /; \n+ 5\n";
    let tokens = lex(source);
    assert!(
        tokens
            .iter()
            .any(|token| token.kind == SyntaxKind::Comment && token.text(source) == ";/ note /;")
    );
    assert_eq!(
        tokens
            .iter()
            .filter(|token| token.kind == SyntaxKind::Newline)
            .count(),
        1
    );
}
