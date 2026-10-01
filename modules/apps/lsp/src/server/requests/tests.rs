use super::{encode_semantic_tokens, request_stale};
use folio_ide::PositionEncoding;
use folio_source::TextRange;

#[test]
fn stale_or_cancelled_navigation_is_discarded() {
    assert!(!request_stale(3, 3, false));
    assert!(request_stale(3, 4, false));
    assert!(request_stale(3, 3, true));
}

#[test]
fn semantic_token_deltas_use_negotiated_character_units() {
    use folio_ide::{SemanticToken, SemanticTokenKind};

    let text = "🦊 Foo\nBar";
    let tokens = [
        SemanticToken {
            range: TextRange { start: 5, end: 8 },
            kind: SemanticTokenKind::Class,
            declaration: true,
            readonly: false,
        },
        SemanticToken {
            range: TextRange { start: 9, end: 12 },
            kind: SemanticTokenKind::Variable,
            declaration: false,
            readonly: false,
        },
    ];
    assert_eq!(
        encode_semantic_tokens(text, &tokens, PositionEncoding::Utf16),
        [0, 3, 3, 0, 1, 1, 0, 3, 7, 0]
    );
    assert_eq!(
        encode_semantic_tokens(text, &tokens, PositionEncoding::Utf8),
        [0, 5, 3, 0, 1, 1, 0, 3, 7, 0]
    );
}
