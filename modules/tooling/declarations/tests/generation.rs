//! Public crate behavior over in-memory inputs.
use folio_declaration_tools::{GenerationOptions, SourceInput, generate};
use folio_format_declarations::MemberKind;

#[test]
fn extracts_state_variable_and_property_access() {
    let text = "ScriptName Sample Extends Quest Hidden\nImport Utility\nInt count = 3\nInt Property Value = 3 AutoReadOnly\nAuto State Busy\nEvent OnUpdate(Int ticks = 1)\nEndEvent\nEndState\n";
    let bundle = generate(
        GenerationOptions { source: "fixture" },
        &[SourceInput {
            path: "Sample.psc",
            text,
        }],
    )
    .unwrap();
    let script = &bundle.scripts[0];
    assert_eq!(script.parent.as_deref(), Some("Quest"));
    assert_eq!(script.imports, ["Utility"]);
    assert!(script.states[0].auto);
    assert_eq!(
        script.states[0].members[0].parameters()[0]
            .default
            .literal(),
        Some("1")
    );
    assert!(
        script
            .members
            .iter()
            .any(|member| member.kind() == MemberKind::Variable && member.name == "count")
    );
    let property = script
        .members
        .iter()
        .find(|member| member.name == "Value")
        .unwrap();
    assert!(property.is_read_only() && property.is_readable() && !property.is_writable());
}

#[test]
fn content_order_does_not_change_output_and_bad_names_fail() {
    let first = SourceInput {
        path: "A.psc",
        text: "ScriptName A\n",
    };
    let second = SourceInput {
        path: "B.psc",
        text: "ScriptName B\n",
    };
    let options = || GenerationOptions { source: "fixture" };
    assert_eq!(
        generate(options(), &[first, second]).unwrap(),
        generate(options(), &[second, first]).unwrap()
    );
    assert!(
        generate(
            options(),
            &[SourceInput {
                path: "A.psc",
                text: "ScriptName Other\n"
            }]
        )
        .is_err()
    );
}

#[test]
fn merges_reopened_states_and_keeps_variable_function_namespaces() {
    let text = "ScriptName Sample\nBool busy\nState Ready\nEndState\nState Ready\nFunction Wait()\nEndFunction\nEndState\nFunction Busy()\nEndFunction\n";
    let bundle = generate(
        GenerationOptions { source: "fixture" },
        &[SourceInput {
            path: "Sample.psc",
            text,
        }],
    )
    .unwrap();
    let script = &bundle.scripts[0];
    assert_eq!(script.states.len(), 1);
    assert_eq!(script.states[0].members[0].name, "Wait");
    assert_eq!(
        script
            .members
            .iter()
            .filter(|member| member.name.eq_ignore_ascii_case("busy"))
            .count(),
        2
    );
}

#[test]
fn records_only_complete_literal_initializers() {
    let text = "ScriptName Sample\nInt plain = 7\nInt negative = -2\n";
    let bundle = generate(
        GenerationOptions { source: "fixture" },
        &[SourceInput {
            path: "Sample.psc",
            text,
        }],
    )
    .unwrap();
    let members = &bundle.scripts[0].members;
    assert_eq!(members[0].initial_literal(), Some("7"));
    assert_eq!(members[1].initial_literal(), Some("-2"));
}
