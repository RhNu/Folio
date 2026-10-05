use super::*;
use crate::SyntaxNode;

fn parsed(source: &str) -> Parse {
    let result = parse(source, PapyrusDialect::Skyrim);
    assert_eq!(result.syntax().text().to_string(), source);
    result
}

#[test]
fn accepts_commented_continuations_and_complete_native_headers() {
    for source in [
        "ScriptName Sample\nFunction Run()\nInt x = 3 + 4 \\ ; note\n + 5\nEndFunction\n",
        "ScriptName Sample\nEvent OnInit() One Two Three Four Five Six Seven Native\nFunction Run() Native\n",
        "ScriptName Sample\nEvent OnInit() One Two Three \\ ; note\nFour Five Six Seven Native\nFunction Run() Native\n",
        "ScriptName Sample\nInt \\ ; header note\nFunction Run() Native\nInt[] \\ ; property note\nProperty Values Auto\n",
    ] {
        let result = parsed(source);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }
}

#[test]
fn rejects_reserved_names_in_every_declaration_position() {
    for body in [
        "ScriptName Length\n",
        "ScriptName Sample Extends Int\n",
        "ScriptName Sample\nImport While\n",
        "ScriptName Sample\nInt Length\n",
        "ScriptName Sample\nInt Property Parent Auto\n",
        "ScriptName Sample\nState True\nEndState\n",
        "ScriptName Sample\nFunction Return() Native\n",
        "ScriptName Sample\nEvent Self() Native\n",
        "ScriptName Sample\nFunction Run(Int as) Native\n",
        "ScriptName Sample\nFunction Run()\nInt lEnGtH\nEndFunction\n",
        "ScriptName Sample\nLength x\n",
    ] {
        assert!(!parsed(body).errors.is_empty(), "{body}");
    }
    let valid = parsed(
        "ScriptName Sample Hidden\nInt Conditional\nFunction Run(Int value)\nInt size = values.Length\nEndFunction\n",
    );
    assert!(valid.errors.is_empty(), "{:?}", valid.errors);
}

#[test]
fn rejects_missing_late_or_repeated_script_headers() {
    for source in [
        "Int x\n",
        "Int x\nScriptName Sample\n",
        "ScriptName Sample\nScriptName Other\n",
        "",
    ] {
        assert!(!parsed(source).errors.is_empty(), "{source}");
    }
    let result = parsed("; preamble\n;/ multiline\ncomment /;\nScriptName Sample\n");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn documentation_requires_an_eligible_header_on_the_previous_line() {
    for source in [
        "ScriptName Sample { inline }\n",
        "ScriptName Sample\n\n{ late }\n",
        "ScriptName Sample\nState Busy\n{ state }\nEndState\n",
        "ScriptName Sample\nInt x\n{ variable }\n",
        "ScriptName Sample\nFunction Run()\nInt x\n{ local }\nEndFunction\n",
        "ScriptName Sample\n{ docs } Int x\n",
        "ScriptName Sample\n{ docs }\n{ duplicate }\n",
    ] {
        assert!(!parsed(source).errors.is_empty(), "{source}");
    }
    let result = parsed(
        "ScriptName Sample\n{ script }\nInt Property Value Auto\n{ property }\nFunction Run()\n{ function\n  multiline }\nEndFunction\nEvent OnInit()\n{ event extension }\nEndEvent\nFunction NativeCall() Native\n{ native }\n",
    );
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

#[test]
fn cast_and_comparison_grouping_produces_reference_values() {
    for (expression, expected) in [
        ("1 As Float + 2.0", 3.0),
        ("-2 As Float + 3.0", 1.0),
        ("2 == 3 < 4", 1.0),
        ("1 + 2 * 3 As Float", 7.0),
    ] {
        let source =
            format!("ScriptName Sample\nFloat Function Run()\nReturn {expression}\nEndFunction\n");
        let result = parsed(&source);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let value = result
            .syntax()
            .descendants()
            .find(|node| node.kind() == SyntaxKind::ReturnStmt)
            .unwrap()
            .children()
            .next()
            .unwrap();
        assert_eq!(
            evaluate(&value).to_bits(),
            f64::to_bits(expected),
            "{expression}"
        );
    }
    let result = parsed("ScriptName Sample\nFunction Run()\nReturn value As Int[]\nEndFunction\n");
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}

// Independent evaluation makes parser grouping observable without copying binding powers.
fn evaluate(node: &SyntaxNode) -> f64 {
    let children = node.children().collect::<Vec<_>>();
    let operator = node
        .children_with_tokens()
        .filter_map(rowan::NodeOrToken::into_token)
        .find(|token| !token.kind().is_trivia())
        .map(|token| token.text().to_string())
        .unwrap_or_default();
    match node.kind() {
        SyntaxKind::LiteralExpr => node.text().to_string().trim().parse().unwrap(),
        SyntaxKind::UnaryExpr => -evaluate(&children[0]),
        SyntaxKind::ParenExpr => evaluate(&children[0]),
        SyntaxKind::BinaryExpr => {
            let left = evaluate(&children[0]);
            if operator.eq_ignore_ascii_case("as") {
                return left;
            }
            let right = evaluate(&children[1]);
            match operator.as_str() {
                "+" => left + right,
                "*" => left * right,
                "==" => f64::from(left.partial_cmp(&right) == Some(std::cmp::Ordering::Equal)),
                "<" => f64::from(left < right),
                _ => panic!("unexpected operator {operator}"),
            }
        },
        _ => panic!("unexpected expression {node:?}"),
    }
}
