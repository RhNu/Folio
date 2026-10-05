//! Public crate behavior over in-memory inputs.
use folio_papyrus::*;
use folio_source::TextRange;

#[test]
fn keeps_every_byte_while_recovering_later_declarations() {
    let source = "Scriptname Snow extends ObjectReference Hidden\r\nFunction Broken(\r\nReturn \"雪\"\r\nFunction Good(Int[] values, String label = \"🦊\") Global\r\nEndFunction\r\n";
    let parsed = parse(source, PapyrusDialect::Skyrim);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert_ne!(parsed.errors, [] as [folio_papyrus::SyntaxError; 0]);
    let found = declarations(&parsed)
        .into_iter()
        .map(|item| item.declaration)
        .collect::<Vec<_>>();
    assert_eq!(
        found[0],
        Declaration::Script {
            name: "Snow".into(),
            parent: Some("ObjectReference".into()),
            flags: vec!["hidden".into()]
        }
    );
    assert_eq!(
        found[1],
        Declaration::Function {
            name: "Good".into(),
            return_type: None,
            parameters: vec![
                Parameter {
                    ty: "Int[]".into(),
                    name: "values".into(),
                    default: None
                },
                Parameter {
                    ty: "String".into(),
                    name: "label".into(),
                    default: Some("\"🦊\"".into())
                }
            ],
            modifiers: vec!["global".into()],
        }
    );
}

#[test]
fn body_edit_does_not_change_signature_facts() {
    let first = "Scriptname Sample\nInt Function Value(Int x = 2)\nReturn x + 1\nEndFunction\nFunction Other() Native\n";
    let second = "Scriptname Sample\nInt Function Value(Int x = 2)\nReturn x + 100000\nEndFunction\nFunction Other() Native\n";
    let summary = |text| {
        declarations(&parse(text, PapyrusDialect::Skyrim))
            .into_iter()
            .map(|item| item.declaration)
            .collect::<Vec<_>>()
    };
    assert_eq!(summary(first), summary(second));
    assert_eq!(
        summary(first)[1],
        Declaration::Function {
            name: "Value".into(),
            return_type: Some("Int".into()),
            parameters: vec![Parameter {
                ty: "Int".into(),
                name: "x".into(),
                default: Some("2".into())
            }],
            modifiers: vec![]
        }
    );
}

#[test]
fn unclosed_tokens_and_multiline_comments_remain_lossless() {
    let source = "Scriptname Fox\n;/ 🦊\n comment /;\nFunction F() Native\n\"unterminated 🦊\n";
    let parsed = parse(source, PapyrusDialect::Skyrim);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert!(declarations(&parsed).iter().any(
        |item| matches!(&item.declaration, Declaration::Function { name, .. } if name == "F")
    ));
    assert!(
        parsed
            .errors
            .iter()
            .any(|error| error.kind == SyntaxErrorKind::UnclosedString)
    );
    let damaged = "Scriptname Fox\n{未关闭\n";
    let parsed = parse(damaged, PapyrusDialect::Skyrim);
    assert_eq!(parsed.syntax().text().to_string(), damaged);
    assert!(
        parsed
            .errors
            .iter()
            .any(|error| error.kind == SyntaxErrorKind::UnclosedComment)
    );
}

#[test]
fn expression_operators_keep_assignment_distinct_from_comparison() {
    let valid = parse(
        "Scriptname S\nFunction F()\nReturn a || b && c == 2\nEndFunction\n",
        PapyrusDialect::Skyrim,
    );
    assert!(valid.errors.is_empty(), "{:?}", valid.errors);
    let expression = valid
        .syntax()
        .descendants()
        .find(|node| node.kind() == SyntaxKind::ReturnStmt)
        .and_then(|node| {
            node.children()
                .find(|child| child.kind() == SyntaxKind::BinaryExpr)
        })
        .unwrap();
    assert!(
        expression
            .children_with_tokens()
            .any(|element| element.kind() == SyntaxKind::OrOr)
    );
    assert!(expression.children().any(|child| {
        child.kind() == SyntaxKind::BinaryExpr
            && child
                .children_with_tokens()
                .any(|element| element.kind() == SyntaxKind::AndAnd)
    }));
    let invalid = parse(
        "Scriptname S\nFunction F()\nReturn a = b\nEndFunction\n",
        PapyrusDialect::Skyrim,
    );
    assert_ne!(invalid.errors, [] as [folio_papyrus::SyntaxError; 0]);
    assert!(
        invalid
            .syntax()
            .descendants()
            .any(|node| node.kind() == SyntaxKind::Error)
    );
}

#[test]
fn members_indexes_calls_and_literals_have_syntax_nodes() {
    let parsed = parse(
        "Scriptname S\nFunction F()\nReturn obj.items[0].Get(true, None)\nEndFunction\n",
        PapyrusDialect::Skyrim,
    );
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let syntax = parsed.syntax();
    assert!(
        syntax
            .descendants()
            .any(|node| node.kind() == SyntaxKind::MemberExpr)
    );
    assert!(
        syntax
            .descendants()
            .any(|node| node.kind() == SyntaxKind::IndexExpr)
    );
    assert!(
        syntax
            .descendants()
            .any(|node| node.kind() == SyntaxKind::CallExpr)
    );
    assert_eq!(
        syntax
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::LiteralExpr)
            .count(),
        3
    );
}

#[test]
fn local_variable_and_control_flow_are_retained() {
    let source = "Scriptname S\nFunction F()\nInt count = 1\nReturn count\nEndFunction\n";
    let parsed = parse(source, PapyrusDialect::Skyrim);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert!(
        parsed
            .syntax()
            .descendants()
            .any(|node| node.kind() == SyntaxKind::ReturnStmt)
    );
    assert!(
        parsed
            .syntax()
            .descendants()
            .any(|node| node.kind() == SyntaxKind::VariableDecl)
    );
    assert_eq!(declarations(&parsed).len(), 2);

    let damaged = "Scriptname S\nFunction F()\nIf True\nReturn 1\nEndIf\nEndFunction\nFunction Good() Native\n";
    let parsed = parse(damaged, PapyrusDialect::Skyrim);
    assert_eq!(parsed.syntax().text().to_string(), damaged);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert!(
        parsed
            .syntax()
            .descendants()
            .any(|node| node.kind() == SyntaxKind::IfStmt)
    );
    assert!(declarations(&parsed).iter().any(
        |item| matches!(&item.declaration, Declaration::Function { name, .. } if name == "Good")
    ));
}

#[test]
fn nested_declarations_arrays_and_control_flow_keep_structure() {
    let source = "Scriptname Frost extends Quest MyFlag\nImport Utility\nInt[] Property Scores Auto Hidden\nAuto State Active\nEvent OnUpdate(Int tick)\nInt[] values = new Int[3]\nIf tick > 0\nvalues[0] = tick\nElseIf tick == 0\nWhile tick < 3\ntick = tick + 1\nEndWhile\nElse\nReturn\nEndIf\nEndEvent\nEndState\n";
    let parsed = parse(source, PapyrusDialect::Skyrim);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let kinds = parsed
        .syntax()
        .descendants()
        .map(|node| node.kind())
        .collect::<Vec<_>>();
    for kind in [
        SyntaxKind::ImportDecl,
        SyntaxKind::PropertyDecl,
        SyntaxKind::StateDecl,
        SyntaxKind::EventDecl,
        SyntaxKind::NewArrayExpr,
        SyntaxKind::IfStmt,
        SyntaxKind::ElseIfClause,
        SyntaxKind::ElseClause,
        SyntaxKind::WhileStmt,
        SyntaxKind::AssignmentStmt,
    ] {
        assert!(kinds.contains(&kind), "missing {kind:?}");
    }
    let facts = declarations(&parsed)
        .into_iter()
        .map(|item| item.declaration)
        .collect::<Vec<_>>();
    assert!(facts.contains(&Declaration::Import {
        name: "Utility".into()
    }));
    assert!(facts.contains(&Declaration::Property {
        name: "Scores".into(),
        ty: "Int[]".into(),
        flags: vec!["auto".into(), "hidden".into()]
    }));
    assert!(facts.contains(&Declaration::State {
        name: "Active".into(),
        flags: vec!["auto".into()]
    }));
    assert!(facts.iter().any(
        |fact| matches!(fact, Declaration::Event { name, parameters, .. }
            if name == "OnUpdate" && parameters.len() == 1 && parameters[0].ty == "Int")
    ));
}

#[test]
fn unterminated_nested_blocks_recover_next_members() {
    let source = "Scriptname S\nState Busy\nEvent OnBegin()\nIf True\nReturn\nEndEvent\nFunction Good() Native\nEndState\n";
    let parsed = parse(source, PapyrusDialect::Skyrim);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert!(
        parsed
            .errors
            .iter()
            .any(|error| error.kind == SyntaxErrorKind::MissingEndIf)
    );
    assert!(
        declarations(&parsed)
            .iter()
            .any(|item| matches!(&item.declaration,
            Declaration::Function { name, .. } if name == "Good"))
    );

    let missing_function_end =
        "Scriptname S\nState Busy\nFunction Broken()\nReturn\nEndState\nEvent OnUpdate() Native\n";
    let parsed = parse(missing_function_end, PapyrusDialect::Skyrim);
    assert_eq!(parsed.syntax().text().to_string(), missing_function_end);
    assert!(
        parsed
            .errors
            .iter()
            .any(|error| error.kind == SyntaxErrorKind::MissingEndFunction)
    );
    assert!(
        !parsed
            .errors
            .iter()
            .any(|error| error.kind == SyntaxErrorKind::MissingEndEvent)
    );
    assert!(
        declarations(&parsed)
            .iter()
            .any(|item| matches!(&item.declaration,
            Declaration::Event { name, .. } if name == "OnUpdate"))
    );
}

#[test]
fn property_initializers_compound_assignment_and_named_calls_parse() {
    let source = "Scriptname S CustomFlag\nInt Property Limit = 4 Auto Conditional\nFunction Adjust()\nInt total = 0\ntotal += Limit\ntotal = total as Int\nUtility.Set(value = total)\nEndFunction\n";
    let parsed = parse(source, PapyrusDialect::Skyrim);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let syntax = parsed.syntax();
    assert!(
        syntax
            .descendants_with_tokens()
            .any(|part| part.kind() == SyntaxKind::PlusEq)
    );
    assert!(
        syntax
            .descendants()
            .any(|node| node.kind() == SyntaxKind::NamedArgument)
    );
    assert!(declarations(&parsed).iter().any(|item| matches!(&item.declaration,
            Declaration::Property { name, flags, .. } if name == "Limit" && flags == &["auto", "conditional"])));
}

#[test]
fn auto_state_marker_and_hex_default_remain_in_declaration_facts() {
    let parsed = parse(
        "ScriptName S\nAuto State Ready\nEndState\nInt Function F(Int mask = 0xFF) Native\n",
        PapyrusDialect::Skyrim,
    );
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let facts = declarations(&parsed);
    assert!(facts.iter().any(|item| matches!(&item.declaration, Declaration::State { name, flags } if name == "Ready" && flags == &["auto"])));
    assert!(facts.iter().any(|item| matches!(&item.declaration, Declaration::Function { parameters, .. } if parameters[0].default.as_deref() == Some("0xFF"))));
}

#[test]
fn void_return_at_eof_with_trivia_does_not_expect_expression() {
    let source = "Scriptname S\nFunction F()\nReturn   ; final comment";
    let parsed = parse(source, PapyrusDialect::Skyrim);
    assert_eq!(parsed.syntax().text().to_string(), source);
    assert!(
        parsed
            .errors
            .iter()
            .any(|error| error.kind == SyntaxErrorKind::MissingEndFunction)
    );
    assert!(
        !parsed
            .errors
            .iter()
            .any(|error| error.kind == SyntaxErrorKind::ExpectedExpression)
    );
    assert!(parsed.syntax().descendants_with_tokens().any(|element| {
        element.kind() == SyntaxKind::Missing
            && element.text_range().start() == element.text_range().end()
    }));
}

#[test]
fn unclosed_unicode_string_range_stops_before_crlf() {
    let parsed = parse("\"🦊\r\n", PapyrusDialect::Skyrim);
    assert!(parsed.errors.iter().any(|error| {
        error.kind == SyntaxErrorKind::UnclosedString
            && error.range == TextRange { start: 0, end: 5 }
    }));
}
