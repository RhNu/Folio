//! Public crate behavior over in-memory inputs.
use std::{path::PathBuf, sync::Arc};

use folio_build::{ProjectAnalysis, ProjectSource};
use folio_ide::*;
use folio_papyrus::PapyrusDialect;
use folio_source::FileId;

fn view(text: &str) -> (IdeSnapshot, FileId) {
    let mut project = ProjectAnalysis::new();
    let view = project
        .sync_sources([ProjectSource {
            package_key: "root".into(),
            canonical_path: PathBuf::from("Example.psc"),
            display_path: "Example.psc".into(),
            script_candidate: "Example".into(),
            dialect: PapyrusDialect::Skyrim,
            text: Arc::from(text),
        }])
        .expect("validated semantic index");
    let file = *view
        .sources
        .keys()
        .next()
        .expect("validated semantic index");
    (view.into(), file)
}

#[test]
fn hover_and_signature_include_names_and_parameters() {
    let text = "Scriptname Example\nInt Function Sum(Int left, Int right)\n Return left + right\nEndFunction\nFunction Use()\n Int total = Sum(1, 2)\n Int other = Sum(right=2, left=1)\nEndFunction\n";
    let (view, file) = view(text);
    let declaration = text.find("Sum").expect("validated semantic index");
    let hover = crate::hover(&view, file, declaration).expect("validated semantic index");
    assert!(
        hover
            .content
            .contains("Int Function Sum(Int left, Int right)")
    );
    let reference = text.rfind("Sum(").expect("validated semantic index");
    let target = source_declaration(&view, file, reference).expect("validated semantic index");
    assert_eq!(target.range.start, declaration);
    assert!(
        crate::hover(&view, file, reference)
            .expect("validated semantic index")
            .content
            .contains("Sum(Int left, Int right)")
    );
    let cursor = text.find("2)").expect("validated semantic index");
    let help = signature_help(&view, file, cursor).expect("validated semantic index");
    assert_eq!(help.active_parameter, 1);
    assert_eq!(help.parameters, ["Int left", "Int right"]);
    let named_cursor = text.rfind("1)").expect("validated semantic index");
    let named_help = signature_help(&view, file, named_cursor).expect("validated semantic index");
    assert_eq!(named_help.active_parameter, 0);
}

#[test]
fn signature_help_is_available_while_call_is_incomplete() {
    let text = "Scriptname Example\nFunction Sum(Int left, Int right) Native\nFunction Use()\n Sum(\n Sum(1,\nEndFunction\n";
    let (view, file) = view(text);
    let cursor = text.find("Sum(\n").expect("validated semantic index") + 4;
    let help = signature_help(&view, file, cursor).expect("validated semantic index");
    assert_eq!(help.parameters, ["Int left", "Int right"]);
    assert_eq!(help.active_parameter, 0);
    let comma_cursor = text.find("Sum(1,\n").expect("validated semantic index") + 6;
    let next = signature_help(&view, file, comma_cursor).expect("validated semantic index");
    assert_eq!(next.active_parameter, 1);
}

#[test]
fn semantic_tokens_and_outline_classify_declarations_and_references() {
    let text = "Scriptname Example\nInt Property Count Auto\nEvent OnInit(Int value)\n Int current = value\nEndEvent\nState Busy\n Event OnBeginState()\n EndEvent\nEndState\n";
    let (view, file) = view(text);
    let tokens = semantic_tokens(&view, file);
    let token_at = |name: &str| {
        let at = text.find(name).expect("validated semantic index");
        tokens
            .iter()
            .find(|item| item.range.start == at)
            .copied()
            .expect("validated semantic index")
    };
    assert_eq!(token_at("Example").kind, SemanticTokenKind::Class);
    assert_eq!(token_at("Count").kind, SemanticTokenKind::Property);
    assert_eq!(token_at("OnInit").kind, SemanticTokenKind::Event);
    assert_eq!(token_at("value").kind, SemanticTokenKind::Parameter);
    let reference = text.rfind("value").expect("validated semantic index");
    assert_eq!(
        tokens
            .iter()
            .find(|item| item.range.start == reference)
            .expect("validated semantic index")
            .kind,
        SemanticTokenKind::Parameter
    );
    let outline = document_symbols(&view, file);
    assert!(
        outline[0]
            .children
            .iter()
            .any(|item| item.name == "OnInit" && item.kind == 24)
    );
    assert!(
        outline[0]
            .children
            .iter()
            .any(|item| item.name == "Count" && item.kind == 7)
    );
    let busy = outline[0]
        .children
        .iter()
        .find(|item| item.name == "Busy")
        .expect("validated semantic index");
    assert!(busy.children.iter().any(|item| item.name == "OnBeginState"));
}

#[test]
fn declaration_navigation_reaches_selected_dependency_source() {
    let dependency = "Scriptname Parent\nInt Function GetNumber() Native\n";
    let root =
        "Scriptname Child Extends Parent\nFunction Use()\n Int number = GetNumber()\nEndFunction\n";
    let mut project = ProjectAnalysis::new();
    let view = project
        .sync_sources([
            ProjectSource {
                package_key: "dependency".into(),
                canonical_path: PathBuf::from("Parent.psc"),
                display_path: "Parent.psc".into(),
                script_candidate: "Parent".into(),
                dialect: PapyrusDialect::Skyrim,
                text: Arc::from(dependency),
            },
            ProjectSource {
                package_key: "root".into(),
                canonical_path: PathBuf::from("Child.psc"),
                display_path: "Child.psc".into(),
                script_candidate: "Child".into(),
                dialect: PapyrusDialect::Skyrim,
                text: Arc::from(root),
            },
        ])
        .expect("validated semantic index");
    let view = IdeSnapshot::from(view);
    let child = view
        .sources
        .iter()
        .find(|(_, source)| source.script_candidate == "Child")
        .expect("validated semantic index")
        .0;
    let parent = view
        .sources
        .iter()
        .find(|(_, source)| source.script_candidate == "Parent")
        .expect("validated semantic index")
        .0;
    let inherited_type = source_declaration(
        &view,
        *child,
        root.find("Parent").expect("validated semantic index"),
    )
    .expect("validated semantic index");
    assert_eq!(inherited_type.file, *parent);
    assert_eq!(
        crate::hover(
            &view,
            *child,
            root.find("Parent").expect("validated semantic index")
        )
        .expect("validated semantic index")
        .owner_script
        .as_deref(),
        Some("Parent")
    );
    let method = source_declaration(
        &view,
        *child,
        root.find("GetNumber").expect("validated semantic index"),
    )
    .expect("validated semantic index");
    assert_eq!(method.file, *parent);
    assert_eq!(
        &dependency[method.range.start..method.range.end],
        "GetNumber"
    );
}
