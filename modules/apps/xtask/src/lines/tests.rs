use super::{Level, code_lines};

#[test]
fn thresholds_are_strict() {
    assert_eq!(Level::for_lines(650), Level::Ok);
    assert_eq!(Level::for_lines(651), Level::Warning);
    assert_eq!(Level::for_lines(1200), Level::Warning);
    assert_eq!(Level::for_lines(1201), Level::Error);
}

#[test]
fn comments_and_blank_lines_do_not_raise_severity() {
    for (size, expected) in [
        (650, Level::Ok),
        (651, Level::Warning),
        (1200, Level::Warning),
        (1201, Level::Error),
    ] {
        let source = "// ignored\n\nvalue(); /* ignored */\n".repeat(size);
        let count = code_lines(&source);
        assert_eq!(count, size);
        assert_eq!(Level::for_lines(count), expected);
    }
}

#[test]
fn empty_and_comment_only_sources_have_no_code() {
    for source in [
        "",
        " \t\r\n\n",
        "// comment",
        "/// documentation\n//! module documentation\n",
        "/* outer\n /* inner */\n still comment */\n",
        "/* unterminated\n comment",
    ] {
        assert_eq!(code_lines(source), 0);
    }
}

#[test]
fn code_on_either_side_of_comments_counts_once() {
    let source =
        "/* lead */ let /* middle */ x = 1; // tail\n/* start\nend */ x += 1;\nx /* tail\nend */\n";
    assert_eq!(code_lines(source), 3);
}

#[test]
fn literals_characters_and_lifetimes_preserve_comment_markers() {
    let source = r####"let url = "https://example.test/*text*/";
let escaped = "\"//text";
let raw = r###"" /* literal */ // literal
next"###;
let bytes = br#"" // bytes
next"#;
let c = cr#"" // C string
next"#;
let c_escaped = c"\"/*text*/";
let character = '/'; let escaped_char = '\''; let byte = b'/';
fn borrow<'a>(x: &'a str) -> &'a str { x }
"####;
    assert_eq!(code_lines(source), 11);
}

#[test]
fn multiline_literals_count_only_nonblank_physical_lines() {
    assert_eq!(
        code_lines("let text = r#\"first\n// literal\n\n \t\n/* literal */\nlast\"#;\n"),
        4
    );
    assert_eq!(code_lines("let text = \"first\\\n\n  \nlast\";\n"), 2);
    assert_eq!(code_lines("let text = \"unfinished\n\n"), 1);
}

#[test]
fn line_endings_and_unterminated_final_lines_count_once() {
    assert_eq!(code_lines("x();\r\n// ignored\r\ny();"), 2);
    assert_eq!(code_lines("x(); y(); /* tail */"), 1);
    assert_eq!(code_lines("x();\n"), 1);
}

#[test]
fn unicode_identifiers_and_rust_whitespace_are_supported() {
    assert_eq!(
        code_lines("let 名称 = \"中文 // 文本\";\n\u{200e}\u{200f}\n"),
        1
    );
    assert_eq!(code_lines("let raw = r#\"start\n\u{200e}\nend\"#;"), 2);
}

#[test]
fn shebang_and_bom_are_excluded_but_attributes_are_code() {
    assert_eq!(
        code_lines("\u{feff}#!/usr/bin/env rust-script\nfn main() {}\n"),
        1
    );
    assert_eq!(code_lines("\u{feff}// comment\n"), 0);
    assert_eq!(code_lines("#![allow(dead_code)]\nfn main() {}\n"), 2);
}
