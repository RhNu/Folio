use super::*;
use crate::{PapyrusDialect, parse};

fn codes(source: &str, flags: &[UserFlag]) -> Vec<&'static str> {
    validate_declarations(&parse(source, PapyrusDialect::Skyrim), Some(flags))
        .iter()
        .map(|issue| issue.code)
        .collect()
}

#[test]
fn property_forms_and_accessors_are_validated_without_bodies() {
    for source in [
        "ScriptName A\nInt Property P AutoReadOnly\n",
        "ScriptName A\nInt Property P = 1 Auto AutoReadOnly\n",
        "ScriptName A\nInt Property P\nEndProperty\n",
        "ScriptName A\nInt Property P\nString Function Get(Int x)\nReturn \"x\"\nEndFunction\nEndProperty\n",
    ] {
        assert!(!codes(source, &[]).is_empty(), "{source}");
    }
    assert_eq!(
        codes("ScriptName A\nInt Property P = 1 AutoReadOnly\n", &[]),
        [] as [&str; 0]
    );
}

#[test]
fn declarations_require_literal_values() {
    assert!(codes("ScriptName A\nInt x = 1 + 2\n", &[]).contains(&"semantic.variable-initializer"));
    assert_eq!(
        codes("ScriptName A\nFunction F(Int x = -1) Native\n", &[]),
        [] as [&str; 0]
    );
}

#[test]
fn callable_defaults_can_precede_required_parameters() {
    for source in [
        "ScriptName A\nFunction F(Int x = -1, A target, Bool enabled = True, Int count) Native\n",
        "ScriptName A\nEvent E(Int x = -1, A target)\nEndEvent\n",
    ] {
        assert!(codes(source, &[]).is_empty(), "{source}");
    }
}

#[test]
fn declaration_literals_are_checked_before_any_calls() {
    for source in [
        "ScriptName A\nFunction F(Int x = True) Native\n",
        "ScriptName A\nFunction F(Int x = 2147483648) Native\n",
        "ScriptName A\nInt x = \"bad\"\n",
        "ScriptName A\nInt Property P = \"bad\" Auto\n",
        "ScriptName A\nString Property P = \"bad\\q\" AutoReadOnly\n",
    ] {
        assert!(!codes(source, &[]).is_empty(), "{source}");
    }
    assert_eq!(
        codes(
            "ScriptName A\nFloat x = 0x10\nInt minimum = -2147483648\nFunction F(Actor actor = None, Int[] values = None) Native\n",
            &[]
        ),
        [] as [&str; 0]
    );
}

#[test]
fn checks_flag_scopes_owner_and_duplicate_modifiers() {
    for source in [
        "ScriptName A\nEvent E() Global\nEndEvent\n",
        "ScriptName A\nInt x Conditional\n",
        "ScriptName A\nFunction F() Native Native\n",
        "ScriptName A\nFunction F()\nInt x Conditional\nEndFunction\n",
    ] {
        assert!(!codes(source, &[]).is_empty(), "{source}");
    }
    let flag = UserFlag {
        name: "Tag".into(),
        bit: Some(7),
        scopes: vec![FlagScope::Variable],
    };
    assert_eq!(
        codes(
            "ScriptName A Conditional\nInt Property P Auto Tag Conditional\n",
            std::slice::from_ref(&flag)
        ),
        [] as [&str; 0]
    );
    assert!(codes("ScriptName A Tag\n", &[flag]).contains(&"semantic.flag-scope"));
}
