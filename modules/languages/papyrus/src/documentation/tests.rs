use super::*;
use crate::{PapyrusDialect, parse};

fn declaration(text: &str, kind: SyntaxKind) -> SyntaxNode {
    parse(text, PapyrusDialect::Skyrim)
        .syntax()
        .descendants()
        .find(|node| node.kind() == kind)
        .unwrap()
}

#[test]
fn binds_inline_and_following_docs_to_their_declaration() {
    for source in [
        "ScriptName Demo { Script docs }\n",
        "ScriptName Demo\n{ Script docs }\nFunction Run() Native\n",
    ] {
        assert_eq!(
            declaration_documentation(&declaration(source, SyntaxKind::ScriptDecl)).as_deref(),
            Some("Script docs")
        );
    }
    for (source, kind, expected) in [
        (
            "Function Run()\n{ Call docs }\nReturn\nEndFunction\n",
            SyntaxKind::FunctionDecl,
            "Call docs",
        ),
        (
            "Event OnInit()\n{ Event docs }\nEndEvent\n",
            SyntaxKind::EventDecl,
            "Event docs",
        ),
        (
            "Int Property Value Auto\n{ Property docs }\n",
            SyntaxKind::PropertyDecl,
            "Property docs",
        ),
        (
            "State Busy\n{ State docs }\nEndState\n",
            SyntaxKind::StateDecl,
            "State docs",
        ),
        (
            "Function Run() Native\n{ Native docs }\n",
            SyntaxKind::FunctionDecl,
            "Native docs",
        ),
        (
            "Int count\n{ Variable docs }\n",
            SyntaxKind::VariableDecl,
            "Variable docs",
        ),
    ] {
        assert_eq!(
            declaration_documentation(&declaration(source, kind)).as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn excludes_ordinary_comments_empty_docs_and_later_body_docs() {
    for source in [
        "Function Run()\n; ordinary\n;/ block /;\nEndFunction\n",
        "Function Run()\n{ }\nEndFunction\n",
        "Function Run()\nReturn\n{ Later docs }\nEndFunction\n",
        "Function Run() Native\nFunction Other() Native\n{ Other docs }\n",
    ] {
        assert_eq!(
            declaration_documentation(&declaration(source, SyntaxKind::FunctionDecl)),
            None
        );
    }
}

#[test]
fn headers_preserve_defaults_modifiers_and_string_whitespace() {
    let source = "String[] Function Run(String label = \"two  words\", \\\n Int count = -2) Global Native Hidden { docs }\n";
    assert_eq!(
        declaration_header(&declaration(source, SyntaxKind::FunctionDecl)),
        "String[] Function Run(String label = \"two  words\", Int count = -2) Global Native Hidden"
    );
}

#[test]
fn inline_parameter_comments_keep_word_boundaries_without_becoming_callable_docs() {
    let source = "Function Run(Int{ parameter note }count) Native\n";
    let node = declaration(source, SyntaxKind::FunctionDecl);
    assert_eq!(declaration_header(&node), "Function Run(Int count) Native");
    assert_eq!(declaration_documentation(&node), None);
}

#[test]
fn manual_property_headers_describe_accessors_without_bodies() {
    let source = "Int Property Value\n{ docs }\nInt Function Get()\nReturn 1\nEndFunction\nFunction Set(Int value)\nEndFunction\nEndProperty\n";
    assert_eq!(
        declaration_header(&declaration(source, SyntaxKind::PropertyDecl)),
        "Int Property Value\n    Int Function Get()\n    EndFunction\n    Function Set(Int value)\n    EndFunction\nEndProperty"
    );
}
