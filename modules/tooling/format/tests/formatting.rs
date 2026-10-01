//! Public crate behavior over in-memory inputs.
use folio_format::{FormatError, format_source};
use folio_papyrus::PapyrusDialect;

#[test]
fn formats_nested_layout_and_spacing_idempotently() {
    let source = "Scriptname Example\nInt Function Run(Int value)\nIf value!=0\nReturn value+1\nElse\nReturn 0\nEndIf\nEndFunction\n";
    let formatted = format_source(source, PapyrusDialect::Skyrim).unwrap();
    assert_eq!(
        formatted,
        "Scriptname Example\nInt Function Run(Int value)\n    If value != 0\n        Return value + 1\n    Else\n        Return 0\n    EndIf\nEndFunction\n"
    );
    assert_eq!(
        format_source(&formatted, PapyrusDialect::Skyrim).unwrap(),
        formatted
    );
}

#[test]
fn preserves_comments_strings_and_crlf() {
    let source =
        "Scriptname S\r\nFunction F()\r\n; hello\r\nReturn \"a b\"; inline\r\nEndFunction\r\n";
    let formatted = format_source(source, PapyrusDialect::Skyrim).unwrap();
    assert!(formatted.contains("    ; hello\r\n"));
    assert!(formatted.contains("    Return \"a b\" ; inline\r\n"));
}

#[test]
fn refuses_broken_syntax() {
    assert!(matches!(
        format_source("Scriptname S\nFunction F()\n", PapyrusDialect::Skyrim),
        Err(FormatError::Syntax(_))
    ));
}

#[test]
fn keeps_multiline_comments_and_continuations_intact() {
    let source = "Scriptname S\n;/ one\ntwo /;\nInt Function F()\nReturn \\\n 1\nEndFunction\n";
    let formatted = format_source(source, PapyrusDialect::Skyrim).unwrap();
    assert!(formatted.contains(";/ one\ntwo /;"));
    assert!(formatted.contains("Return \\\n 1"));
    assert_eq!(
        format_source(&formatted, PapyrusDialect::Skyrim).unwrap(),
        formatted
    );
}
