use super::decode_string;

#[test]
fn decodes_escaped_backslashes_once() {
    assert_eq!(decode_string("\\\\n"), Some("\\n".into()));
    assert_eq!(decode_string("a\\n\\\"b"), Some("a\n\"b".into()));
    assert_eq!(decode_string("a\\q"), None);
}
