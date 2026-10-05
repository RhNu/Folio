use super::*;

#[test]
fn trusted_markdown_treats_documentation_as_prose() {
    let escaped = prose("[run](command:evil) <img>");
    assert!(escaped.contains(r"\[run\]\(command:evil\)"));
    assert!(escaped.contains("&lt;img&gt;"));
}

#[test]
fn command_arguments_round_trip_unicode_and_reserved_characters() {
    let args = json!(["file:///D:/Mod%20API.psc", {"line":4,"character":2}, "说明"]);
    let link = command_link("Declaration", "folio.openLocation", &args);
    let encoded = link.split_once('?').unwrap().1.strip_suffix(')').unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&percent_decode(encoded).unwrap()).unwrap(),
        args
    );
}

#[test]
fn virtual_uris_are_names_not_paths() {
    assert_eq!(
        virtual_owner(&virtual_uri("MyScript")),
        Some("MyScript".into())
    );
    assert!(virtual_owner("folio-declaration:/..%2Fsecret.psc").is_none());
}

#[test]
fn code_fences_cannot_be_terminated_by_string_defaults() {
    let rendered = code("Function Run(String s = \"~~~p\")");
    assert!(rendered.starts_with("~~~~papyrus\n"));
    assert!(rendered.ends_with("\n~~~~"));
}

#[test]
fn long_signatures_wrap_only_between_parameters() {
    let declaration = "String Function LongFunctionName(String firstParameter = \"a,b with many characters in the literal\", Int secondParameter = 12) Native";
    let rendered = readable_declaration(declaration);
    assert!(rendered.contains("\"a,b with many characters in the literal\""));
    assert!(rendered.contains("\n    Int secondParameter = 12"));
    assert!(rendered.ends_with(") Native"));
    let before = folio_papyrus::lex(declaration)
        .into_iter()
        .filter(|token| {
            !matches!(
                token.kind,
                folio_papyrus::SyntaxKind::Whitespace
                    | folio_papyrus::SyntaxKind::Newline
                    | folio_papyrus::SyntaxKind::Continuation
            )
        })
        .map(|token| token.text(declaration).to_owned())
        .collect::<Vec<_>>();
    let after = folio_papyrus::lex(&rendered)
        .into_iter()
        .filter(|token| {
            !matches!(
                token.kind,
                folio_papyrus::SyntaxKind::Whitespace
                    | folio_papyrus::SyntaxKind::Newline
                    | folio_papyrus::SyntaxKind::Continuation
            )
        })
        .map(|token| token.text(&rendered).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(before, after);
}
