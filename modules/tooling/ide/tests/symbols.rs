//! Public crate behavior over in-memory inputs.
use folio_build::ProjectAnalysisView;
use folio_ide::*;
use folio_source::FileId;

use std::path::PathBuf;
use std::sync::Arc;

use folio_build::{ProjectAnalysis, ProjectSource};
use folio_papyrus::PapyrusDialect;

fn view(text: &str) -> (ProjectAnalysisView, FileId) {
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
        .unwrap();
    let file = *view.sources.keys().next().unwrap();
    (view, file)
}

#[test]
fn hover_and_signature_include_names_and_parameters() {
    let text = "Scriptname Example\nInt Function Sum(Int left, Int right)\n Return left + right\nEndFunction\nFunction Use()\n Int total = Sum(1, 2)\n Int other = Sum(right=2, left=1)\nEndFunction\n";
    let (view, file) = view(text);
    let declaration = text.find("Sum").unwrap();
    let hover = crate::hover(&view, file, declaration).unwrap();
    assert!(
        hover
            .content
            .contains("Int Function Sum(Int left, Int right)")
    );
    let reference = text.rfind("Sum(").unwrap();
    let target = source_declaration(&view, file, reference).unwrap();
    assert_eq!(target.range.start, declaration);
    assert!(
        crate::hover(&view, file, reference)
            .unwrap()
            .content
            .contains("Sum(Int left, Int right)")
    );
    let cursor = text.find("2)").unwrap();
    let help = signature_help(&view, file, cursor).unwrap();
    assert_eq!(help.active_parameter, 1);
    assert_eq!(help.parameters, ["Int left", "Int right"]);
    let named_cursor = text.rfind("1)").unwrap();
    let named_help = signature_help(&view, file, named_cursor).unwrap();
    assert_eq!(named_help.active_parameter, 0);
}

#[test]
fn signature_help_is_available_while_call_is_incomplete() {
    let text = "Scriptname Example\nFunction Sum(Int left, Int right) Native\nFunction Use()\n Sum(\n Sum(1,\nEndFunction\n";
    let (view, file) = view(text);
    let cursor = text.find("Sum(\n").unwrap() + 4;
    let help = signature_help(&view, file, cursor).unwrap();
    assert_eq!(help.parameters, ["Int left", "Int right"]);
    assert_eq!(help.active_parameter, 0);
    let comma_cursor = text.find("Sum(1,\n").unwrap() + 6;
    let next = signature_help(&view, file, comma_cursor).unwrap();
    assert_eq!(next.active_parameter, 1);
}

#[test]
fn semantic_tokens_and_outline_classify_declarations_and_references() {
    let text = "Scriptname Example\nInt Property Count Auto\nEvent OnInit(Int value)\n Int current = value\nEndEvent\nState Busy\n Event OnBeginState()\n EndEvent\nEndState\n";
    let (view, file) = view(text);
    let tokens = semantic_tokens(&view, file);
    let token_at = |name: &str| {
        let at = text.find(name).unwrap();
        tokens
            .iter()
            .find(|item| item.range.start == at)
            .copied()
            .unwrap()
    };
    assert_eq!(token_at("Example").kind, SemanticTokenKind::Class);
    assert_eq!(token_at("Count").kind, SemanticTokenKind::Property);
    assert_eq!(token_at("OnInit").kind, SemanticTokenKind::Event);
    assert_eq!(token_at("value").kind, SemanticTokenKind::Parameter);
    let reference = text.rfind("value").unwrap();
    assert_eq!(
        tokens
            .iter()
            .find(|item| item.range.start == reference)
            .unwrap()
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
        .unwrap();
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
        .unwrap();
    let child = view
        .sources
        .iter()
        .find(|(_, source)| source.script_candidate == "Child")
        .unwrap()
        .0;
    let parent = view
        .sources
        .iter()
        .find(|(_, source)| source.script_candidate == "Parent")
        .unwrap()
        .0;
    let inherited_type = source_declaration(&view, *child, root.find("Parent").unwrap()).unwrap();
    assert_eq!(inherited_type.file, *parent);
    assert_eq!(
        crate::hover(&view, *child, root.find("Parent").unwrap())
            .unwrap()
            .owner_script
            .as_deref(),
        Some("Parent")
    );
    let method = source_declaration(&view, *child, root.find("GetNumber").unwrap()).unwrap();
    assert_eq!(method.file, *parent);
    assert_eq!(
        &dependency[method.range.start..method.range.end],
        "GetNumber"
    );
}
