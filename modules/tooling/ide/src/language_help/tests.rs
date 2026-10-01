use super::*;
use crate::tests::{file, project};

fn language(text: &str, needle: &str) -> (Hover, TextRange) {
    language_hover(text, PapyrusDialect::Skyrim, text.find(needle).unwrap()).unwrap()
}

#[test]
fn keywords_work_case_insensitively_in_incomplete_source() {
    let text = "sCrIpTnAmE Example\nFunction Run()\n If True\n";
    for (needle, title) in [
        ("sCrIpTnAmE", "ScriptName"),
        ("Function", "Function"),
        ("If", "If"),
    ] {
        let (item, range) = language(text, needle);
        assert_eq!(item.declaration, title);
        assert!(item.documentation.is_some());
        assert_eq!(&text[range.start..range.end], needle);
        assert_eq!(item.language.unwrap().dialect, PapyrusDialect::Skyrim);
    }
}

#[test]
fn standard_flags_are_contextual_and_auto_states_have_state_help() {
    let text = "ScriptName Example Hidden Conditional\nInt Property Count Auto Hidden Conditional\nAuto State Ready\nEndState\nFunction Use()\n Int Hidden = 1\n Int Conditional = Hidden\nEndFunction\n";
    let (hidden, _) = language(text, "Hidden");
    assert!(hidden.documentation.unwrap().contains("Creation Kit"));
    let (conditional, _) = language(text, "Conditional");
    assert!(
        conditional
            .documentation
            .unwrap()
            .contains("condition system")
    );
    let (property, _) = language(text, "Auto Hidden");
    assert!(property.documentation.unwrap().contains("backing variable"));
    let (state, _) = language(text, "Auto State");
    assert!(state.documentation.unwrap().contains("initial state"));
    assert!(
        state
            .language
            .unwrap()
            .reference_url
            .ends_with("State_Reference")
    );
    for needle in ["Hidden =", "Conditional =", "Hidden\nEndFunction"] {
        assert!(language_hover(text, PapyrusDialect::Skyrim, text.find(needle).unwrap()).is_none());
    }
    let name = "ScriptName Hidden\nInt Property Conditional Auto\n";
    assert!(language_hover(name, PapyrusDialect::Skyrim, name.find("Hidden").unwrap()).is_none());
    assert!(
        language_hover(
            name,
            PapyrusDialect::Skyrim,
            name.find("Conditional").unwrap()
        )
        .is_none()
    );
}

#[test]
fn builtin_types_describe_representation_and_array_types() {
    let text = "ScriptName Example\nInt[] values\nFloat delay\nBool enabled\nString label\n";
    let (int, _) = language(text, "Int");
    assert_eq!(int.declaration, "Int[]");
    assert!(int.documentation.unwrap().contains("32-bit"));
    assert!(int.details.iter().any(|detail| detail.contains("None")));
    let (float, _) = language(text, "Float");
    assert!(float.documentation.unwrap().contains("single-precision"));
    let (boolean, _) = language(text, "Bool");
    assert!(boolean.documentation.unwrap().contains("False"));
    let (string, _) = language(text, "String");
    assert!(string.documentation.unwrap().contains("runtime casing"));
}

#[test]
fn literal_values_include_defaults_hexadecimal_signed_numbers_and_strings() {
    let text = "ScriptName Example\nInt Property Count = 0x2A Auto\nFunction Run(Int value = -2147483648, String label = \"雪\\n🦊\")\n Float delay = -1.25\n Bool yes = True\n Bool no = False\n Example target = None\n Int bits = 0xFFFFFFFF\n String empty = \"\"\nEndFunction\n";
    for (needle, ty, value) in [
        ("0x2A", "Int", "42"),
        ("2147483648", "Int", "-2147483648"),
        ("1.25", "Float", "-1.25"),
        ("True", "Bool", "True"),
        ("False", "Bool", "False"),
        ("None", "None", "None"),
        ("0xFFFFFFFF", "Int", "-1"),
        ("\"雪", "String", "\"雪\\n🦊\""),
        ("\"\"", "String", "\"\""),
    ] {
        let (item, range) = language(text, needle);
        assert_eq!(item.declaration, ty);
        assert_eq!(item.details, [format!("Value of literal: {value}")]);
        if needle == "2147483648" || needle == "1.25" {
            assert!(text[range.start..range.end].starts_with('-'));
        }
    }
}

#[test]
fn subtraction_does_not_change_the_right_literal_value() {
    let text = "ScriptName Example\nFunction Run()\n Int value = 8 - 3\nEndFunction\n";
    assert_eq!(language(text, "3").0.details, ["Value of literal: 3"]);
    let (operator, range) = language(text, "- 3");
    assert!(operator.documentation.unwrap().contains("Subtracts"));
    assert_eq!(&text[range.start..range.end], "-");
}

#[test]
fn invalid_literals_never_claim_a_decoded_value() {
    let text = "ScriptName Example\nFunction Run()\n Int big = 2147483648\n String invalid = \"\\q\"\n Float huge = 9999999999999999999999999999999999999999.0\nEndFunction\n";
    for needle in ["2147483648", "\"\\q", "999999"] {
        let (item, _) = language(text, needle);
        assert!(item.details[0].starts_with("Invalid "));
    }
}

#[test]
fn operators_and_delimiters_explain_their_own_token() {
    let text = "ScriptName Example\nFunction Run()\n Bool result = True && !False\n Int value = -4\n value += 2\nEndFunction\n";
    let (logical, range) = language(text, "&&");
    assert!(logical.documentation.unwrap().contains("Short-circuits"));
    assert_eq!(&text[range.start..range.end], "&&");
    let (negative, _) = language(text, "-4");
    assert!(negative.documentation.unwrap().contains("negates"));
    let (update, _) = language(text, "+=");
    assert!(update.documentation.unwrap().contains("assigns"));
    assert_eq!(language(text, "(").0.declaration, "()");
}

#[test]
fn trivia_and_other_dialects_keywords_do_not_trigger_language_help() {
    let text =
        "; If Int True\n;/ While Float /;\n{String None}\n\"unterminated\n Struct Var Is Guard\n";
    for needle in [
        "If",
        "While",
        "String",
        "\"unterminated",
        "Struct",
        "Var",
        "Is",
        "Guard",
        "\n",
        " ",
    ] {
        assert!(
            language_hover(text, PapyrusDialect::Skyrim, text.find(needle).unwrap()).is_none(),
            "{needle}"
        );
    }
    assert!(language_hover(text, PapyrusDialect::Skyrim, text.len()).is_none());
    assert!(language_hover(text, PapyrusDialect::Skyrim, usize::MAX).is_none());
    let unicode = "ScriptName Example\nString label = \"雪\"\n";
    assert!(
        language_hover(
            unicode,
            PapyrusDialect::Skyrim,
            unicode.find('雪').unwrap() + 1
        )
        .is_none()
    );
}

#[test]
fn semantic_hover_retains_self_parent_types_and_intrinsic_length_signature() {
    let base = "ScriptName Base\nFunction Run()\nEndFunction\n";
    let text = "ScriptName Example Extends Base\nFunction Run()\n Example target = Self\n Parent.Run()\n Int[] values = New Int[2]\n Int size = values.Length\nEndFunction\n";
    let view = project(&[("Base", base), ("Example", text)]);
    let file = file(&view, "Example");
    for (needle, declaration) in [
        ("Self", "Example Self"),
        ("Parent.Run", "Base Parent"),
        ("Length", "Int Property Length"),
    ] {
        let item = crate::hover(&view, file, text.find(needle).unwrap()).unwrap();
        assert_eq!(item.declaration, declaration);
        assert!(item.documentation.is_some());
        assert!(item.language.is_some());
        assert!(item.symbol.is_some());
    }
    let call = crate::hover(&view, file, text.find("Run()").unwrap()).unwrap();
    assert!(call.language.is_none());
    assert!(call.declaration.contains("Function Run()"));
}

#[test]
fn semantic_hover_uses_precise_ranges_and_ignores_trivia_inside_expressions() {
    let text = "ScriptName Example\nFunction Run()\n Int value = 2 ; Int\n Int other = value + 3\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let item = crate::hover(&view, file, text.find("+ 3").unwrap()).unwrap();
    let range = item.span.unwrap().range;
    assert_eq!(&text[range.start..range.end], "+");
    for byte in [
        text.find(" +").unwrap(),
        text.find("; Int").unwrap(),
        text.len(),
    ] {
        assert!(crate::hover(&view, file, byte).is_none());
    }
    let named = crate::hover(&view, file, text.find("value +").unwrap()).unwrap();
    assert!(named.language.is_none());
    assert!(named.declaration.contains("Int value"));
}
