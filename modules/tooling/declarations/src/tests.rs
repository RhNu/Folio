use super::*;

#[test]
fn declaration_generation_rejects_invalid_headers_and_constants() {
    for text in [
        "Scriptname Demo\nInt Property P\nString Function Get(Int x)\nEndFunction\nEndProperty\n",
        "Scriptname Demo\nInt Property P\nEndProperty\n",
        "Scriptname Demo\nFunction F(Int a = 1, Int b) Native\n",
        "Scriptname Demo\nFunction F(Int a = 2147483648) Native\n",
        "Scriptname Demo\nInt value = 1 + 2\n",
        "Scriptname Demo\nInt Property P AutoReadOnly\n",
    ] {
        assert!(
            generate(
                GenerationOptions { source: "fixture" },
                &[SourceInput {
                    path: "Demo.psc",
                    text,
                }]
            )
            .is_err(),
            "accepted invalid declaration: {text}"
        );
    }
}

#[test]
fn declaration_generation_normalizes_signed_constants_without_compiling_bodies() {
    let bundle = generate(GenerationOptions { source: "fixture" }, &[SourceInput {
        path: "Demo.psc",
        text: "Scriptname Demo\nInt value = - 0x10\nFunction F()\n MissingRuntimeApi()\nEndFunction\n",
    }]).unwrap();
    let MemberData::Variable {
        initial_literal, ..
    } = &bundle.scripts[0].members[0].data
    else {
        panic!("expected variable");
    };
    assert_eq!(initial_literal.as_deref(), Some("-0x10"));
}

#[path = "../tests/generation.rs"]
mod generation;
#[path = "../tests/pex.rs"]
mod pex;

#[test]
fn generation_keeps_docs_for_scripts_and_members() {
    let bundle = generate(GenerationOptions { source: "fixture" }, &[SourceInput {
        path: "Demo.psc",
        text: "ScriptName Demo\n{ Script docs }\nInt Property Value Auto\n{ Property docs }\nFunction Run() Native\n{ Callable docs }\nAuto State Busy\n; State section\nEvent OnInit()\n{ Event docs }\nEndEvent\nEndState\n",
    }]).unwrap();
    let script = &bundle.scripts[0];
    assert_eq!(script.documentation.as_deref(), Some("Script docs"));
    assert_eq!(
        script.members[0].documentation.as_deref(),
        Some("Property docs")
    );
    assert_eq!(
        script.members[1].documentation.as_deref(),
        Some("Callable docs")
    );
    assert_eq!(
        script.states[0].members[0].documentation.as_deref(),
        Some("Event docs")
    );
}
