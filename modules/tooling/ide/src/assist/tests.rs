use super::*;
use crate::tests::{file, project};

#[test]
fn nontrailing_defaults_preserve_signature_positions_and_hints() {
    let text = "ScriptName Example\nFunction Travel(Int destination = -1, Example driver) Native\nFunction Use(Example vehicle)\n Travel(driver = vehicle)\n Travel(-1, vehicle)\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let help = crate::signature_help(
        &view,
        file,
        text.find("driver = vehicle").unwrap() + "driver = ".len(),
    )
    .unwrap();
    assert_eq!(help.parameters, ["Int destination = -1", "Example driver"]);
    assert_eq!(help.active_parameter, 1);
    let hints = inlay_hints(
        &view,
        file,
        TextRange {
            start: 0,
            end: text.len(),
        },
    );
    assert_eq!(
        hints
            .iter()
            .map(|hint| hint.label.as_str())
            .collect::<Vec<_>>(),
        ["destination:", "driver:"]
    );
}

#[test]
fn completion_uses_visible_locals_and_replaces_whole_identifier() {
    let text = "Scriptname Example\nFunction Use(Int amount)\n Int count = amount\n count = amount\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let byte = text.rfind("amount").unwrap() + 2;
    let items = completion(&view, file, byte, &|| false).unwrap();
    let item = items.iter().find(|item| item.label == "amount").unwrap();
    assert_eq!(
        &text[item.replacement.start..item.replacement.end],
        "amount"
    );
    assert_eq!(item.insert_text, "amount");
}

#[test]
fn completion_does_not_expose_other_callable_locals_or_comment_names() {
    let text = "Scriptname Example\nFunction A()\n Int secret = 1\nEndFunction\nFunction B()\n ; comment\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    assert!(
        !completion(&view, file, text.find(" ; comment").unwrap(), &|| false)
            .unwrap()
            .iter()
            .any(|item| item.label == "secret")
    );
    assert!(
        completion(&view, file, text.find("comment").unwrap() + 2, &|| false)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn hints_map_arguments_and_omit_explicit_or_obvious_names() {
    let text = "Scriptname Example\nFunction Work(Int left, Int right) Native\nFunction Use(Int left)\n Work(1, 2)\n Work(right=2, left=1)\n Work(left, 3)\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let hints = inlay_hints(
        &view,
        file,
        TextRange {
            start: 0,
            end: text.len(),
        },
    );
    assert_eq!(
        hints
            .iter()
            .map(|hint| hint.label.as_str())
            .collect::<Vec<_>>(),
        ["left:", "right:", "right:"]
    );
}

#[test]
fn member_completion_preserves_instance_and_global_receiver_distinction() {
    let api = "Scriptname Api\nInt Property Count Auto\nFunction Utility() Global\nEndFunction\nFunction Work()\nEndFunction\n";
    let user =
        "Scriptname User\nApi Property Owner Auto\nFunction Use()\n Owner.\n Api.\nEndFunction\n";
    let view = project(&[("Api", api), ("User", user)]);
    let file = file(&view, "User");
    let instance = completion(&view, file, user.find("Owner.\n").unwrap() + 6, &|| false).unwrap();
    assert!(instance.iter().any(|item| item.label == "Count"));
    assert!(instance.iter().any(|item| item.label == "Work"));
    assert!(!instance.iter().any(|item| item.label == "Utility"));
    let global = completion(&view, file, user.find("Api.\n").unwrap() + 4, &|| false).unwrap();
    assert!(global.iter().any(|item| item.label == "Utility"));
    assert!(!global.iter().any(|item| item.label == "Count"));
    assert!(!global.iter().any(|item| item.label == "Work"));
}

#[test]
fn intrinsic_completion_signature_and_hover_reuse_semantic_shapes() {
    let text = "Scriptname Example\nFunction Use(Int[] values)\n values.Find(1)\n values.RFind(2, 3)\n Int size = values.Length\n String stateName = GetState()\n GotoState(\"Busy\")\n values.\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let items = completion(&view, file, text.find("values.\n").unwrap() + 7, &|| false).unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["Find", "Length", "RFind"]
    );
    let unqualified = completion(&view, file, text.find(" GotoState").unwrap(), &|| false).unwrap();
    assert!(unqualified.iter().any(|item| item.label == "GetState"));
    assert!(unqualified.iter().any(|item| item.label == "GotoState"));
    let help = crate::signature_help(&view, file, text.find("Find(1)").unwrap() + 5).unwrap();
    assert_eq!(help.parameters, ["Int akElement", "Int aiStartIndex = 0"]);
    let reverse = crate::signature_help(&view, file, text.find("RFind(2").unwrap() + 9).unwrap();
    assert_eq!(reverse.active_parameter, 1);
    assert_eq!(
        reverse.parameters,
        ["Int akElement", "Int aiStartIndex = -1"]
    );
    let state =
        crate::signature_help(&view, file, text.find("GotoState(\"").unwrap() + 10).unwrap();
    assert_eq!(state.parameters, ["String asNewState"]);
    let hover = crate::hover(&view, file, text.find("Length").unwrap()).unwrap();
    assert_eq!(hover.declaration, "Int Property Length");
    assert_eq!(hover.details, ["Read-only array length"]);
    assert!(
        crate::hover(&view, file, text.find("Find(").unwrap())
            .unwrap()
            .declaration
            .contains("Int akElement")
    );
}

#[test]
fn inherited_imported_global_completion_uses_checker_lookup() {
    let base = "Scriptname Base\nFunction Utility() Global\nEndFunction\n";
    let child = "Scriptname Child Extends Base\n";
    let user = "Scriptname User\nImport Child\nFunction Use()\n Utility()\nEndFunction\n";
    let view = project(&[("Base", base), ("Child", child), ("User", user)]);
    let file = file(&view, "User");
    let items = completion(&view, file, user.find("Utility()").unwrap() + 2, &|| false).unwrap();
    assert!(items.iter().any(|item| item.label == "Utility"));
}

#[test]
fn completion_resolves_only_selected_declaration_and_documentation() {
    let text = "Scriptname Example\nInt Function Work(Int amount = 2) Native\n{ Updates the counter. }\nFunction Use()\n Wor\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let items = completion(&view, file, text.find(" Wor\n").unwrap() + 4, &|| false).unwrap();
    let item = items.iter().find(|item| item.label == "Work").unwrap();
    assert_eq!(item.kind, 3);
    assert_eq!(item.detail, "Int");
    assert_eq!(item.documentation, None);
    let hover = completion_hover(&view, item).unwrap();
    assert!(hover.declaration.contains("Int amount = 2"));
    assert!(
        hover
            .documentation
            .unwrap()
            .contains("Updates the counter.")
    );
}

#[test]
fn lazy_intrinsic_completion_retains_selected_array_element_type() {
    let text = "Scriptname Example\nFunction Use(String[] values)\n values.\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let items = completion(&view, file, text.find("values.\n").unwrap() + 7, &|| false).unwrap();
    let find = items.iter().find(|item| item.label == "Find").unwrap();
    assert_eq!(find.kind, 3);
    assert_eq!(find.documentation, None);
    assert!(
        completion_hover(&view, find)
            .unwrap()
            .declaration
            .contains("String akElement")
    );
    let length = items.iter().find(|item| item.label == "Length").unwrap();
    assert_eq!(length.kind, 10);
    assert_eq!(
        completion_hover(&view, length).unwrap().declaration,
        "Int Property Length"
    );
}

#[test]
fn completion_merges_keywords_in_order_and_retains_semantic_collisions() {
    let text = "ScriptName Example\nFunction Use()\n Int While = 1\n While = 2\nEndFunction\n";
    let view = project(&[
        ("Example", text),
        ("Zulu", "ScriptName Zulu\n"),
        ("Alpha", "ScriptName Alpha\n"),
    ]);
    let file = file(&view, "Example");
    let items = completion(&view, file, text.find(" While = 2").unwrap(), &|| false).unwrap();
    assert!(
        items
            .windows(2)
            .all(|pair| pair[0].label.to_ascii_lowercase() < pair[1].label.to_ascii_lowercase())
    );
    let item = items
        .iter()
        .find(|item| item.label.eq_ignore_ascii_case("While"))
        .unwrap();
    assert!(matches!(item.symbol, Some(Symbol::Local { .. })));
    assert!(items.iter().any(|item| item.label == "Auto State"));
    assert!(items.iter().any(|item| item.label == "Alpha"));
    assert!(items.iter().any(|item| item.label == "Zulu"));
}

#[test]
fn completion_filters_array_intrinsics_before_presentation() {
    let text = "ScriptName Example\nFunction Use(Int[] values)\n values.rF\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let items = completion(
        &view,
        file,
        text.find("values.rF").unwrap() + "values.rF".len(),
        &|| false,
    )
    .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, "RFind");
}

#[test]
fn completion_returns_cancellation_without_partial_results() {
    let text = "ScriptName Example\nFunction Use()\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    assert_eq!(
        completion(&view, file, text.find("EndFunction").unwrap(), &|| true),
        Err(folio_analysis::AnalysisCancelled)
    );
    let checks = std::cell::Cell::new(0);
    let cancelled = || {
        checks.set(checks.get() + 1);
        checks.get() >= 3
    };
    assert_eq!(
        completion(&view, file, text.find("EndFunction").unwrap(), &cancelled),
        Err(folio_analysis::AnalysisCancelled)
    );
    assert!(
        !completion(&view, file, text.find("EndFunction").unwrap(), &|| false)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn completion_suppresses_token_contents_and_accepts_their_end_boundaries() {
    let text =
        "ScriptName Example\nFunction Use()\n String text = \"value\"\n ; marker \nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    for content in ["\"value\"", "; marker "] {
        let start = text.find(content).unwrap();
        assert!(
            completion(&view, file, start, &|| false)
                .unwrap()
                .is_empty()
        );
        assert!(
            !completion(&view, file, start + content.len(), &|| false)
                .unwrap()
                .is_empty()
        );
    }
}
