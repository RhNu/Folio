use super::*;
use crate::{PapyrusDialect, parse};

#[test]
fn parameter_defaults_preserve_string_spaces_and_remove_numeric_trivia() {
    let source = "ScriptName Sample\nFunction Run(Int first = - ;/ note /; 2, Int second = - \\ ; note\n3, String label = \"two words\") Native\n";
    let parsed = parse(source, PapyrusDialect::Skyrim);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let function = parsed
        .syntax()
        .descendants()
        .find_map(FunctionAst::cast)
        .unwrap();
    let defaults = function
        .parameters()
        .into_iter()
        .map(|parameter| parameter.default.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(defaults, ["-2", "-3", "\"two words\""]);
}
