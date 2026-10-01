use super::*;

#[test]
fn inverse_declaration_navigation_preserves_state_and_type_owner() {
    let text = "Scriptname Child Extends Base\nBase Property Owner Auto\nState Busy\n Function Work()\n EndFunction\nEndState\n";
    assert_eq!(
        external_declaration_symbol(text, text.find("Work").unwrap()),
        Some(Symbol::StateMember {
            script: "Child".into(),
            state: "Busy".into(),
            name: "Work".into()
        })
    );
    assert_eq!(
        external_declaration_symbol(text, text.find("Base").unwrap()),
        Some(Symbol::Script("Base".into()))
    );
    assert_eq!(
        external_declaration_symbol(text, text.find("Owner").unwrap()),
        Some(Symbol::Member {
            script: "Child".into(),
            name: "Owner".into()
        })
    );
}

#[test]
fn locates_inherited_member_in_its_state_and_ignores_body_locals() {
    let text = "ScriptName Base\nFunction Pulse()\n Int value\nEndFunction\nState Busy\n Function Pulse() Native\nEndState\n";
    let symbol = Symbol::StateMember {
        script: "base".into(),
        state: "busy".into(),
        name: "pulse".into(),
    };
    let range = external_declaration_range(text, &symbol).unwrap();
    assert_eq!(&text[range.start..range.end], "Pulse");
    assert_eq!(text[..range.start].lines().count(), 6);
    assert!(
        external_declaration_range(
            text,
            &Symbol::Member {
                script: "Base".into(),
                name: "value".into()
            }
        )
        .is_none()
    );
}

#[test]
fn locates_script_property_and_variable_name_spans() {
    let text = "ScriptName Base\nInt value\nInt Property Count Auto\n";
    for symbol in [
        Symbol::Script("base".into()),
        Symbol::Member {
            script: "Base".into(),
            name: "value".into(),
        },
        Symbol::Member {
            script: "Base".into(),
            name: "count".into(),
        },
    ] {
        let range = external_declaration_range(text, &symbol).unwrap();
        let expected = match &symbol {
            Symbol::Script(_) => "Base",
            Symbol::Member { name, .. } => name,
            _ => unreachable!(),
        };
        assert!(text[range.start..range.end].eq_ignore_ascii_case(expected));
    }
}

#[test]
fn refuses_wrong_script_and_ambiguous_member() {
    let text = "ScriptName Base\nInt Property Value Auto\nInt Function Value() Native\n";
    assert!(external_declaration_range(text, &Symbol::Script("Other".into())).is_none());
    assert!(
        external_declaration_range(
            text,
            &Symbol::Member {
                script: "Base".into(),
                name: "Value".into()
            }
        )
        .is_none()
    );
}
