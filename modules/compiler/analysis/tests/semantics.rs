//! Public parsing, binding, and type-analysis contracts over in-memory inputs.
use std::sync::Arc;

use folio_analysis::AnalysisHost;
use folio_diagnostics::Severity;
use folio_hir::{ExpressionKind, MemberKind as HirMemberKind, Statement, Symbol, Type};
use folio_papyrus::PapyrusDialect;
use folio_source::{FileId, Revision};

fn source(host: &mut AnalysisHost, id: u32, text: &str) {
    host.upsert(
        FileId(id),
        Revision(1),
        Arc::from(text),
        PapyrusDialect::Skyrim,
    )
    .expect("synthetic test input is valid");
}

#[path = "semantics/external.rs"]
mod external;

#[test]
fn reports_local_errors_without_losing_sibling_facts() {
    let mut host = AnalysisHost::new();
    let text = "Scriptname Sample\nInt Function Broken()\nReturn Missing + 1\nEndFunction\nInt Function Good()\nReturn 2\nEndFunction\n";
    source(&mut host, 2, text);
    let view = host.view();
    let diagnostics = view
        .diagnostics(FileId(2))
        .expect("synthetic test input is valid");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.unknown-name")
            .count(),
        1
    );
    assert_eq!(
        view.type_at(
            FileId(2),
            text.find('2').expect("synthetic test input is valid")
        ),
        Some(Type::Int)
    );
    assert_eq!(
        view.hir(FileId(2))
            .expect("synthetic test input is valid")
            .bodies
            .len(),
        2
    );
}

#[test]
fn string_addition_accepts_numeric_operands_and_records_conversions() {
    let mut host = AnalysisHost::new();
    source(
        &mut host,
        20,
        "ScriptName Probe\nString Function Mix(Int count, Float ratio)\nReturn \"n=\" + count + ratio\nEndFunction\nString Function Reverse()\nReturn 1.5 + \"x\"\nEndFunction\nString Function Compound()\nString value = \"x\"\nvalue += 2\nReturn value\nEndFunction\n",
    );
    let view = host.view();
    assert_eq!(
        view.diagnostics(FileId(20))
            .expect("synthetic test input is valid"),
        [] as [folio_diagnostics::Diagnostic; 0]
    );
    let script = view.hir(FileId(20)).expect("synthetic test input is valid");
    let Statement::Return {
        value: Some(value), ..
    } = &script.bodies[0].statements[0]
    else {
        panic!("expected return");
    };
    let ExpressionKind::Binary { right, .. } = &value.kind else {
        panic!("expected addition");
    };
    assert_eq!(value.ty, Type::String);
    assert_eq!(right.conversion, Some(Type::String));
    let Statement::Return {
        value: Some(value), ..
    } = &script.bodies[1].statements[0]
    else {
        panic!("expected return");
    };
    let ExpressionKind::Binary { left, .. } = &value.kind else {
        panic!("expected addition");
    };
    assert_eq!(left.conversion, Some(Type::String));
    let Statement::Assignment { value, .. } = &script.bodies[2].statements[1] else {
        panic!("expected compound assignment");
    };
    assert_eq!(value.conversion, Some(Type::String));
    source(
        &mut host,
        22,
        "ScriptName Bad\nString Function Invalid()\nReturn \"x\" + True\nEndFunction\n",
    );
    assert!(
        host.view()
            .diagnostics(FileId(22))
            .expect("synthetic test input is valid")
            .iter()
            .any(|item| item.code == "semantic.operator-type")
    );
}

#[test]
fn required_call_padding_is_opt_in_and_preserves_named_binding() {
    let mut host = AnalysisHost::new();
    source(
        &mut host,
        21,
        "ScriptName Probe\nFunction Target(Bool flag, Int count, Float ratio, String label, Probe peer, Int[] values)\nEndFunction\nFunction Use()\nTarget(ratio = 2.5)\nEndFunction\n",
    );
    let strict = host.view();
    assert_eq!(
        strict
            .diagnostics(FileId(21))
            .expect("synthetic test input is valid")
            .iter()
            .filter(|item| item.code == "semantic.argument-count")
            .count(),
        5
    );
    host.set_fill_missing_arguments(true);
    let compatible = host.view();
    let issues = compatible
        .diagnostics(FileId(21))
        .expect("synthetic test input is valid");
    assert_eq!(issues.len(), 5, "{issues:?}");
    assert!(issues.iter().all(|item| {
        item.code == "semantic.argument-defaulted" && item.severity == Severity::Warning
    }));
    let script = compatible
        .hir(FileId(21))
        .expect("synthetic test input is valid");
    let Statement::Expression(call) = &script.bodies[1].statements[0] else {
        panic!("expected call");
    };
    let ExpressionKind::Call {
        argument_ordinals,
        parameter_defaults,
        ..
    } = &call.kind
    else {
        panic!("expected call");
    };
    assert_eq!(argument_ordinals, &[2]);
    assert_eq!(
        parameter_defaults,
        &[
            Some((Type::Bool, "false".into())),
            Some((Type::Int, "0".into())),
            None,
            Some((Type::String, "\"\"".into())),
            Some((Type::Script("Probe".into()), "None".into())),
            Some((Type::Array(Box::new(Type::Int)), "None".into())),
        ]
    );
    assert_eq!(
        strict
            .diagnostics(FileId(21))
            .expect("synthetic test input is valid")
            .iter()
            .filter(|item| item.code == "semantic.argument-count")
            .count(),
        5
    );
}

#[test]
fn catches_inheritance_cycle_and_unknown_custom_flag() {
    let mut host = AnalysisHost::new();
    source(
        &mut host,
        3,
        "Scriptname A extends B\nInt Property Score Auto Special\n",
    );
    source(&mut host, 4, "Scriptname B extends A\n");
    let view = host.view();
    let diagnostics = view
        .diagnostics(FileId(3))
        .expect("synthetic test input is valid");
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code == "semantic.inheritance-cycle")
    );
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code == "semantic.unknown-flag")
    );
    host.set_user_flags(vec!["Special".into()]);
    let updated = host.view();
    assert!(
        !updated
            .diagnostics(FileId(3))
            .expect("synthetic test input is valid")
            .iter()
            .any(|item| item.code == "semantic.unknown-flag")
    );
    assert!(
        view.diagnostics(FileId(3))
            .expect("synthetic test input is valid")
            .iter()
            .any(|item| item.code == "semantic.unknown-flag")
    );
}

#[test]
fn preserves_compiler_intrinsics_and_elseif_structure() {
    let mut host = AnalysisHost::new();
    let text = "Scriptname Probe\nFunction Go()\nInt[] values = New Int[2]\nInt index = values.Find(1)\nIf index == 0\nGotoState(\"Ready\")\nElseIf index == 1\nGotoState(\"Idle\")\nEndIf\nEndFunction\nState Ready\nEvent OnBeginState()\nString current = GetState()\nEndEvent\nEndState\n";
    source(&mut host, 9, text);
    let view = host.view();
    assert!(
        view.diagnostics(FileId(9))
            .expect("synthetic test input is valid")
            .is_empty(),
        "{:?}",
        view.diagnostics(FileId(9))
    );
    let script = view.hir(FileId(9)).expect("synthetic test input is valid");
    assert!(script.calls.iter().any(|call| matches!(&call.target, Some(Symbol::Intrinsic { name }) if name.eq_ignore_ascii_case("Find"))));
    assert!(script.calls.iter().any(|call| matches!(&call.target, Some(Symbol::Intrinsic { name }) if name.eq_ignore_ascii_case("GotoState"))));
    let go = script
        .bodies
        .iter()
        .find(|body| matches!(&body.symbol, Symbol::Member { name, .. } if name == "Go"))
        .expect("synthetic test input is valid");
    assert!(
        go.statements.iter().any(
            |statement| matches!(statement, Statement::If { else_if, .. } if else_if.len() == 1)
        )
    );
}

#[test]
fn coerces_skyrim_truthy_values_at_boolean_uses() {
    let mut host = AnalysisHost::new();
    let text = "Scriptname Truth\nBool Function Probe(Int count, Float ratio, String label, Truth value, Int[] items)\nIf count\nElseIf ratio\nEndIf\nWhile label\nReturn False\nEndWhile\nIf value && items\nReturn !value\nEndIf\nBool fromNone = None\nReturn fromNone\nEndFunction\n";
    source(&mut host, 14, text);
    let view = host.view();
    assert!(
        view.diagnostics(FileId(14))
            .expect("synthetic test input is valid")
            .is_empty(),
        "{:?}",
        view.diagnostics(FileId(14))
    );
    let script = view.hir(FileId(14)).expect("synthetic test input is valid");
    let body = &script.bodies[0];
    let Statement::If {
        condition, else_if, ..
    } = &body.statements[0]
    else {
        panic!("expected If");
    };
    assert_eq!(condition.ty, Type::Int);
    assert_eq!(condition.conversion, Some(Type::Bool));
    assert_eq!(else_if[0].0.ty, Type::Float);
    assert_eq!(else_if[0].0.conversion, Some(Type::Bool));
    let Statement::While { condition, .. } = &body.statements[1] else {
        panic!("expected While");
    };
    assert_eq!(condition.ty, Type::String);
    assert_eq!(condition.conversion, Some(Type::Bool));
    let Statement::If {
        condition,
        then_branch,
        ..
    } = &body.statements[2]
    else {
        panic!("expected If");
    };
    let ExpressionKind::Binary { left, right, .. } = &condition.kind else {
        panic!("expected logical expression");
    };
    assert_eq!(left.conversion, Some(Type::Bool));
    assert_eq!(right.conversion, Some(Type::Bool));
    let Statement::Return {
        value: Some(value), ..
    } = &then_branch[0]
    else {
        panic!("expected Return");
    };
    let ExpressionKind::Unary { operand, .. } = &value.kind else {
        panic!("expected unary expression");
    };
    assert_eq!(operand.conversion, Some(Type::Bool));
    let Statement::Variable {
        value: Some(value), ..
    } = &body.statements[3]
    else {
        panic!("expected variable");
    };
    assert_eq!(value.ty, Type::None);
    assert_eq!(value.conversion, Some(Type::Bool));
}

#[test]
fn bool_coercion_does_not_allow_primitive_none_comparisons() {
    let mut host = AnalysisHost::new();
    source(
        &mut host,
        15,
        "Scriptname Truth\nFunction Probe(Int count, Bool flag, String label)\nIf count == None\nEndIf\nIf flag == None\nEndIf\nIf label == None\nEndIf\nEndFunction\n",
    );
    let diagnostics = host
        .view()
        .diagnostics(FileId(15))
        .expect("synthetic test input is valid");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.operator-type")
            .count(),
        3
    );
}

#[test]
fn preserves_cross_script_member_kind_for_codegen() {
    let mut host = AnalysisHost::new();
    source(
        &mut host,
        10,
        "Scriptname Provider\nInt Function Value()\nReturn 1\nEndFunction\n",
    );
    source(
        &mut host,
        11,
        "Scriptname Consumer\nProvider Property Ref Auto\nInt Function Use()\nReturn Ref.Value\nEndFunction\n",
    );
    let view = host.view();
    assert_eq!(
        view.diagnostics(FileId(11))
            .expect("synthetic test input is valid"),
        [] as [folio_diagnostics::Diagnostic; 0]
    );
    let script = view.hir(FileId(11)).expect("synthetic test input is valid");
    assert!(script.referenced_members.iter().any(|member| {
            matches!(&member.symbol, Symbol::Member { script, name } if script == "Provider" && name == "Value")
                && matches!(member.kind, HirMemberKind::Function { .. })
        }));
}

#[test]
fn callable_local_identities_distinguish_accessors_and_states() {
    let mut host = AnalysisHost::new();
    source(
        &mut host,
        13,
        "Scriptname Collisions\nInt Property First\nFunction Set(Int firstValue)\nInt local = firstValue\nEndFunction\nEndProperty\nString Property Second\nFunction Set(String secondValue)\nString local = secondValue\nEndFunction\nEndProperty\nState One\nInt Function Echo(Int value)\nInt local = value\nReturn local\nEndFunction\nEndState\nState Two\nInt Function Echo(Int value)\nInt local = value\nReturn local\nEndFunction\nEndState\n",
    );
    let view = host.view();
    assert!(
        view.diagnostics(FileId(13))
            .expect("synthetic test input is valid")
            .is_empty(),
        "{:?}",
        view.diagnostics(FileId(13))
    );
    let script = view.hir(FileId(13)).expect("synthetic test input is valid");
    let setters = script
        .bodies
        .iter()
        .filter(
            |body| matches!(&body.symbol, Symbol::PropertyAccessor { name, .. } if name == "Set"),
        )
        .collect::<Vec<_>>();
    assert_eq!(setters.len(), 2);
    assert_eq!(setters[0].parameters.len(), 1);
    assert_eq!(setters[1].parameters.len(), 1);
    assert_ne!(setters[0].parameters[0].ty, setters[1].parameters[0].ty);
    let local_owners = script
        .declarations
        .iter()
        .filter_map(|declaration| match &declaration.symbol {
            Symbol::Local { owner, name, .. } if name == "local" => Some(owner.as_ref()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(local_owners.len(), 4);
    assert_ne!(local_owners[0], local_owners[1]);
    assert_ne!(local_owners[2], local_owners[3]);
}

#[test]
fn analyzes_state_accessors_named_arguments_arrays_and_casts() {
    let mut host = AnalysisHost::new();
    let text = "Scriptname Features\nInt Property Score\nInt Function Get()\nReturn 1\nEndFunction\nFunction Set(Int value)\nEndFunction\nEndProperty\nInt Function Sum(Int left, Int right = 2)\nReturn left + right\nEndFunction\nState Armed\nFunction Signal()\nInt[] values = new Int[2]\nInt amount = 1\namount += values[0]\nScore = Sum(right = 3, left = amount)\nScore = 2.5 as Int\nEndFunction\nFunction Dispatch()\nSignal()\nEndFunction\nEndState\n";
    source(&mut host, 8, text);
    let view = host.view();
    let diagnostics = view
        .diagnostics(FileId(8))
        .expect("synthetic test input is valid");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let script = view.hir(FileId(8)).expect("synthetic test input is valid");
    assert_eq!(script.bodies.len(), 5);
    assert!(script.calls.iter().any(|call| call.arguments == [1, 0]));
    assert!(
        script
            .expressions
            .iter()
            .any(|fact| matches!(fact.kind, ExpressionKind::Cast { .. }) && fact.ty == Type::Int)
    );
}

#[test]
fn duplicate_script_does_not_replace_first_provider() {
    let mut host = AnalysisHost::new();
    source(
        &mut host,
        10,
        "Scriptname Shared\nInt Function First()\nReturn 1\nEndFunction\n",
    );
    source(
        &mut host,
        11,
        "Scriptname shared\nInt Function Second()\nReturn 2\nEndFunction\n",
    );
    let view = host.view();
    assert!(
        view.diagnostics(FileId(11))
            .expect("synthetic test input is valid")
            .iter()
            .any(|item| item.code == "semantic.duplicate-script")
    );
    assert!(
        view.hir(FileId(10))
            .expect("synthetic test input is valid")
            .declarations
            .iter()
            .any(|item| matches!(&item.symbol, Symbol::Member { name, .. } if name == "First"))
    );
    assert!(
        !view
            .hir(FileId(10))
            .expect("synthetic test input is valid")
            .declarations
            .iter()
            .any(|item| matches!(&item.symbol, Symbol::Member { name, .. } if name == "Second"))
    );
}
