//! Public crate behavior over in-memory inputs.
use std::collections::BTreeMap;

use folio_diagnostics::Severity;
use folio_hir::{Body, ExpressionFact, ExpressionKind, Script, Statement, Type};
use folio_lint::{LintConfig, PREFER_TRUTHY_NONE_CHECK, lint_script};
use folio_source::{FileId, SourceSpan, TextRange};

fn at(start: usize) -> SourceSpan {
    SourceSpan {
        file: FileId(3),
        range: TextRange {
            start,
            end: start + 5,
        },
    }
}

fn value(ty: Type, kind: ExpressionKind, start: usize) -> ExpressionFact {
    ExpressionFact {
        span: at(start),
        ty,
        binding: None,
        conversion: None,
        kind,
    }
}

fn comparison(ty: Type, reversed: bool, start: usize) -> ExpressionFact {
    let reference = value(ty, ExpressionKind::Literal("value".into()), start);
    let none = value(
        Type::None,
        ExpressionKind::Literal("None".into()),
        start + 1,
    );
    let (left, right) = if reversed {
        (none, reference)
    } else {
        (reference, none)
    };
    value(
        Type::Bool,
        ExpressionKind::Binary {
            operator: "!=".into(),
            left: Box::new(left),
            right: Box::new(right),
        },
        start,
    )
}

fn script(conditions: Vec<ExpressionFact>) -> Script {
    Script {
        bodies: vec![Body {
            symbol: folio_hir::Symbol::Script("S".into()),
            return_type: Type::Void,
            parameters: vec![],
            statements: conditions
                .into_iter()
                .map(|condition| Statement::If {
                    span: condition.span,
                    condition,
                    then_branch: vec![],
                    else_if: vec![],
                    else_branch: vec![],
                })
                .collect(),
        }],
        ..Default::default()
    }
}

#[test]
fn warns_for_script_reference_on_either_side_of_none() {
    let input = script(vec![
        comparison(Type::Script("Actor".into()), false, 10),
        comparison(Type::Script("Actor".into()), true, 30),
        comparison(Type::Array(Box::new(Type::Int)), false, 50),
    ]);
    let diagnostics = lint_script(&input, &LintConfig::default());
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].primary, Some(at(10)));
    assert_eq!(diagnostics[1].primary, Some(at(30)));
    assert!(
        diagnostics
            .iter()
            .all(|item| item.code == PREFER_TRUTHY_NONE_CHECK)
    );
}

#[test]
fn config_disables_or_promotes_only_the_known_rule() {
    let input = script(vec![comparison(Type::Script("Actor".into()), false, 10)]);
    let disabled = LintConfig::from_rules(&BTreeMap::from([(
        PREFER_TRUTHY_NONE_CHECK.into(),
        "off".into(),
    )]))
    .unwrap();
    assert!(lint_script(&input, &disabled).is_empty());
    let promoted = LintConfig::from_rules(&BTreeMap::from([(
        PREFER_TRUTHY_NONE_CHECK.into(),
        "error".into(),
    )]))
    .unwrap();
    assert_eq!(lint_script(&input, &promoted)[0].severity, Severity::Error);
    assert!(LintConfig::from_rules(&BTreeMap::from([("unknown".into(), "off".into())])).is_err());
}

#[test]
fn visits_else_if_and_nested_while_but_skips_invalid_conditions() {
    let valid = comparison(Type::Script("Actor".into()), false, 20);
    let nested = comparison(Type::Script("Actor".into()), false, 40);
    let mut invalid = comparison(Type::Error, false, 60);
    invalid.ty = Type::Error;
    let input = Script {
        bodies: vec![Body {
            symbol: folio_hir::Symbol::Script("S".into()),
            return_type: Type::Void,
            parameters: vec![],
            statements: vec![Statement::If {
                span: at(0),
                condition: invalid,
                then_branch: vec![],
                else_if: vec![(
                    valid,
                    vec![Statement::While {
                        span: at(30),
                        condition: nested,
                        body: vec![],
                    }],
                )],
                else_branch: vec![],
            }],
        }],
        ..Default::default()
    };
    let diagnostics = lint_script(&input, &LintConfig::default());
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].primary, Some(at(20)));
    assert_eq!(diagnostics[1].primary, Some(at(40)));
}
