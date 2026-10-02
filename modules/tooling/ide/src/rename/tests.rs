use super::*;
use crate::tests::{file, project};

#[test]
fn local_rename_preserves_bindings_and_ignores_comments_and_strings() {
    let text = "Scriptname Example\nFunction Use()\n Int value = 1\n value = 2\n ; value\n String text = \"value\"\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let edits = rename(&view, file, text.find("value").unwrap(), "result").unwrap();
    assert_eq!(edits.len(), 2);
    assert!(
        edits
            .iter()
            .all(|edit| &text[edit.span.range.start..edit.span.range.end] == "value")
    );
}

#[test]
fn sibling_blocks_can_reuse_the_renamed_local_name() {
    let text = "ScriptName Example\nFunction Use()\n If True\n  Int first = 1\n  first = 2\n Else\n  Int result = 3\n  result = 4\n EndIf\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let edits = rename(&view, file, text.find("first").unwrap(), "result").unwrap();
    assert_eq!(edits.len(), 2);
    assert!(
        edits
            .iter()
            .all(|edit| &text[edit.span.range.start..edit.span.range.end] == "first")
    );
}

#[test]
fn renaming_one_sibling_binding_leaves_the_other_binding_unchanged() {
    let text = "ScriptName Example\nFunction Use()\n If True\n  Int value = 1\n  value = 2\n Else\n  Int value = 3\n  value = 4\n EndIf\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let edits = rename(&view, file, text.find("value").unwrap(), "renamedValue").unwrap();
    assert_eq!(edits.len(), 2);
    assert!(
        edits
            .iter()
            .all(|edit| edit.span.range.end < text.find("Else").unwrap())
    );
}

#[test]
fn member_rename_remaps_following_local_declaration_identity() {
    let text = "ScriptName Example\nFunction Work()\n Int value = 1\n value = 2\nEndFunction\nFunction Use()\n Work()\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let edits = rename(&view, file, text.find("Work").unwrap(), "PerformWork").unwrap();
    assert_eq!(edits.len(), 2);
    assert!(
        edits
            .iter()
            .all(|edit| &text[edit.span.range.start..edit.span.range.end] == "Work")
    );
}

#[test]
fn reanalysis_rejects_capture_even_without_a_declaration_collision() {
    let text = "ScriptName Example\nInt Property Count Auto\nFunction Use()\n Int value = 1\n Int result = Count\n value = result\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    assert_eq!(
        rename(&view, file, text.find("value").unwrap(), "Count"),
        Err(RenameError::UnverifiableEdit)
    );
}

#[test]
fn project_member_rename_updates_cross_script_uses() {
    let base = "Scriptname Base\nInt Property Count Auto\n";
    let user = "Scriptname User\nBase Property Owner Auto\nFunction Use()\n Owner.Count = 1\nEndFunction\n";
    let view = project(&[("Base", base), ("User", user)]);
    let file = file(&view, "Base");
    let edits = rename(&view, file, base.find("Count").unwrap(), "Total").unwrap();
    assert_eq!(edits.len(), 2);
    assert_ne!(edits[0].span.file, edits[1].span.file);
}

#[test]
fn parameter_rename_updates_named_calls() {
    let text = "Scriptname Example\nFunction Work(Int amount)\n Int result = amount\nEndFunction\nFunction Use()\n Work(amount=1)\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    assert_eq!(
        rename(&view, file, text.find("amount").unwrap(), "quantity")
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn collisions_keywords_and_incomplete_analysis_are_rejected() {
    let text = "Scriptname Example\nFunction Use(Int other)\n Int value = 1\n value = other\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let at = text.find("value").unwrap();
    assert_eq!(
        rename(&view, file, at, "other"),
        Err(RenameError::Collision)
    );
    assert_eq!(rename(&view, file, at, "If"), Err(RenameError::InvalidName));
    assert_eq!(
        rename(&view, file, at, "Length"),
        Err(RenameError::InvalidName)
    );
    let invalid = "Scriptname Broken\nFunction Use()\n missing()\nEndFunction\n";
    let broken = project(&[("Broken", invalid)]);
    assert_eq!(
        prepare_rename(
            &broken,
            self::file(&broken, "Broken"),
            invalid.find("Use").unwrap()
        ),
        Err(RenameError::IncompleteAnalysis)
    );
}

#[test]
fn native_events_scripts_and_overrides_are_rejected() {
    let base = "Scriptname Base\nFunction Work()\nEndFunction\n";
    let child = "Scriptname Child Extends Base\nFunction Work()\nEndFunction\nEvent OnInit()\nEndEvent\nFunction NativeWork() Native\n";
    let view = project(&[("Base", base), ("Child", child)]);
    let file = file(&view, "Child");
    for word in ["Child", "Work", "OnInit", "NativeWork"] {
        assert_eq!(
            prepare_rename(&view, file, child.find(word).unwrap()),
            Err(RenameError::UnsupportedSymbol)
        );
    }
}

#[test]
fn manual_property_rename_preserves_accessor_and_parameter_owners() {
    let text = "Scriptname Example\nInt stored\nInt Property Count\n Int Function Get()\n  Return stored\n EndFunction\n Function Set(Int value)\n  stored = value\n EndFunction\nEndProperty\nFunction Use()\n Count = 1\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    assert_eq!(
        rename(&view, file, text.find("Count").unwrap(), "Total")
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn warning_only_analysis_allows_independently_verified_local_rename() {
    let text = "Scriptname Example\nFunction Work(Int amount) Native\nFunction Use()\n Int value = 1\n Work()\n value = 2\nEndFunction\n";
    let mut view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let mut host = AnalysisHost::new();
    host.set_fill_missing_arguments(true);
    host.upsert(
        file,
        Revision(1),
        text.into(),
        view.analysis.dialect(file).unwrap(),
    )
    .unwrap();
    view.analysis = host.view();
    let diagnostics = view.analysis.diagnostics(file).unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == folio_diagnostics::Severity::Warning)
    );
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == folio_diagnostics::Severity::Error)
    );
    assert_eq!(
        rename(&view, file, text.find("value").unwrap(), "result")
            .unwrap()
            .len(),
        2
    );
}
