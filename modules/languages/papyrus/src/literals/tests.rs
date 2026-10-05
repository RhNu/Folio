use super::*;

#[test]
fn declaration_constants_check_values_and_declared_types() {
    for (literal, ty) in [
        ("1", "Int"),
        ("-2147483648", "Int"),
        ("0x10", "Int"),
        ("0x10", "Float"),
        ("0x80000000", "Int"),
        ("0xFFFFFFFF", "Int"),
        ("0xFFFFFFFF", "Float"),
        ("-1", "Float"),
        ("-1.25", "Float"),
        ("True", "Bool"),
        ("\"two words\"", "String"),
        ("None", "Actor"),
        ("None", "Int[]"),
        ("- ;/ note /; 2", "Int"),
        ("- \\ ; note\n2", "Int"),
    ] {
        assert!(
            constant_literal_matches_type(literal, ty),
            "{literal} as {ty}"
        );
    }
    for (literal, ty) in [
        ("True", "Int"),
        ("1", "Bool"),
        ("1", "String"),
        ("1.0", "Int"),
        ("\"bad\"", "Int"),
        ("None", "String"),
        ("None", "None"),
        ("None", "Int[][]"),
        ("2147483648", "Float"),
        ("2147483648", "Int"),
        ("0x100000000", "Int"),
        ("-0xFFFFFFFF", "Int"),
        ("999999999999999999999999999999999999999.0", "Float"),
        ("\"bad\\q\"", "String"),
        ("1 + 2", "Int"),
        ("name", "Actor"),
        ("1e3", "Float"),
        ("0xF.0", "Float"),
        ("+1", "Int"),
        ("1\n2", "Int"),
        ("-\n1", "Int"),
    ] {
        assert!(
            !constant_literal_matches_type(literal, ty),
            "{literal} as {ty}"
        );
    }
}

#[test]
fn extracts_constant_spelling_without_token_trivia() {
    let parsed = crate::parse(
        "ScriptName Sample\nInt x = - ;/ note /; 2\nString text = \"two words\"\n",
        crate::PapyrusDialect::Skyrim,
    );
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let values = parsed
        .syntax()
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::VariableDecl)
        .map(|node| {
            node.children()
                .find_map(|child| constant_literal_text(&child))
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(values, ["-2", "\"two words\""]);
}

#[test]
fn decodes_decimal_and_hexadecimal_integer_values() {
    for (text, expected) in [
        ("0", 0),
        ("10", 10),
        ("0x10", 16),
        ("0X7f", 127),
        ("0x80000000", i32::MIN),
        ("0x80000001", -2_147_483_647),
        ("0XfFfFfFfF", -1),
        ("0xFFFFFFFE", -2),
        ("0x00000000FFFFFFFF", -1),
        ("+0xFFFFFFFF", -1),
        ("-0x10", -16),
        ("+16", 16),
        ("2147483647", i32::MAX),
        ("-2147483648", i32::MIN),
        ("-0x80000000", i32::MIN),
    ] {
        assert_eq!(decode_integer_literal(text), Some(expected), "{text}");
    }
}

#[test]
fn rejects_incomplete_out_of_range_or_noninteger_spelling() {
    for text in [
        "",
        "-",
        "0x",
        "0xG1",
        "1.0",
        "1_0",
        "--1",
        "+-1",
        "2147483648",
        "-2147483649",
        "4294967295",
        "-0x80000001",
        "-0xFFFFFFFF",
        "0x100000000",
    ] {
        assert_eq!(decode_integer_literal(text), None, "{text}");
    }
}

#[test]
fn external_literal_trivia_normalizes_before_value_decoding() {
    for (text, expected) in [
        (" - 2 ", -2),
        ("\r\n - 2 \t\r\n", -2),
        ("- ;/ note /; 2", -2),
        ("- \\ ; note\n 0x10", -16),
        ("- ;/ min /; 2147483648", i32::MIN),
        (" ;/ mask /; 0xFFFFFFFF", -1),
    ] {
        assert!(constant_literal_matches_type(text, "Int"));
        assert_eq!(decode_integer_literal(text), Some(expected));
    }
    let text = " ;/ note /; \"two words\" ; trailing";
    assert!(constant_literal_matches_type(text, "String"));
    assert_eq!(decode_string_literal(text).as_deref(), Some("two words"));
    for (text, expected, ty) in [
        ("- ;/ note /; 1.25", "-1.25", "Float"),
        (" ;/ note /; True ", "True", "Bool"),
        ("None ; trailing", "None", "Actor"),
    ] {
        assert!(constant_literal_matches_type(text, ty));
        assert_eq!(
            normalize_constant_literal_text(text).as_deref(),
            Some(expected)
        );
    }
    for text in ["-\n2", "1 2", "1 + 2", "identifier", "--2", "\"unclosed"] {
        assert_eq!(normalize_constant_literal_text(text), None, "{text}");
    }
}

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
