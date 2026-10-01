use super::*;

#[path = "../tests/generation.rs"]
mod generation;
#[path = "../tests/pex.rs"]
mod pex;

#[test]
fn generation_keeps_docs_for_scripts_states_and_members() {
    let bundle = generate(GenerationOptions { source: "fixture" }, &[SourceInput {
        path: "Demo.psc",
        text: "ScriptName Demo\n{ Script docs }\nInt Property Value Auto\n{ Property docs }\nFunction Run() Native\n{ Callable docs }\nAuto State Busy\n{ State docs }\nEvent OnInit()\n{ Event docs }\nEndEvent\nEndState\n",
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
        script.states[0].documentation.as_deref(),
        Some("State docs")
    );
    assert_eq!(
        script.states[0].members[0].documentation.as_deref(),
        Some("Event docs")
    );
}
