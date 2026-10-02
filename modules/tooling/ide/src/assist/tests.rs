use super::*;
use crate::tests::{file, project};

#[test]
fn completion_uses_visible_locals_and_replaces_whole_identifier() {
    let text = "Scriptname Example\nFunction Use(Int amount)\n Int count = amount\n count = amount\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let byte = text.rfind("amount").unwrap() + 2;
    let items = completion(&view, file, byte);
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
        !completion(&view, file, text.find(" ; comment").unwrap())
            .iter()
            .any(|item| item.label == "secret")
    );
    assert!(completion(&view, file, text.find("comment").unwrap() + 2).is_empty());
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
    let instance = completion(&view, file, user.find("Owner.\n").unwrap() + 6);
    assert!(instance.iter().any(|item| item.label == "Count"));
    assert!(instance.iter().any(|item| item.label == "Work"));
    assert!(!instance.iter().any(|item| item.label == "Utility"));
    let global = completion(&view, file, user.find("Api.\n").unwrap() + 4);
    assert!(global.iter().any(|item| item.label == "Utility"));
    assert!(!global.iter().any(|item| item.label == "Count"));
    assert!(!global.iter().any(|item| item.label == "Work"));
}

#[test]
fn intrinsic_completion_signature_and_hover_reuse_semantic_shapes() {
    let text = "Scriptname Example\nFunction Use(Int[] values)\n values.Find(1)\n values.RFind(2, 3)\n Int size = values.Length\n String stateName = GetState()\n GotoState(\"Busy\")\n values.\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let items = completion(&view, file, text.find("values.\n").unwrap() + 7);
    assert_eq!(
        items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["Find", "Length", "RFind"]
    );
    let unqualified = completion(&view, file, text.find(" GotoState").unwrap());
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
    let items = completion(&view, file, user.find("Utility()").unwrap() + 2);
    assert!(items.iter().any(|item| item.label == "Utility"));
}
