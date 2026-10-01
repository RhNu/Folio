use super::*;

#[test]
fn decodes_escapes_once_and_preserves_unicode() {
    assert_eq!(decode_string_literal(r#""\\n""#), Some("\\n".into()));
    assert_eq!(decode_string_literal(r#""a\n\"b""#), Some("a\n\"b".into()));
    assert_eq!(
        decode_string_literal(r#""雪\t🦊\r""#),
        Some("雪\t🦊\r".into())
    );
    assert_eq!(decode_string_literal("\"\""), Some(String::new()));
}

#[test]
fn rejects_incomplete_or_invalid_string_literals() {
    for text in [
        "plain",
        "\"",
        "\"unterminated",
        "\"a\nb\"",
        "\"a\rb\"",
        r#""a\q""#,
        r#""a\""#,
        r#""a"b""#,
    ] {
        assert_eq!(decode_string_literal(text), None, "{text:?}");
    }
}
