use std::sync::Arc;

use folio_format_declarations::decode;
use folio_papyrus::PapyrusDialect;
use folio_source::{FileId, Revision};

use super::super::AnalysisHost;
use super::*;

fn source(host: &mut AnalysisHost, id: u32, text: &str) {
    host.upsert(
        FileId(id),
        Revision(1),
        Arc::from(text),
        PapyrusDialect::Skyrim,
    )
    .unwrap();
}

fn sdk() -> folio_format_declarations::DeclarationBundle {
    decode(br#"{"schema":1,"package":{"name":"qa","version":"1","source":"fixture","generator":"test"},"compatibility":{"target":"skyrim-se","abi":"papyrus-skyrim"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Quest","members":[{"name":"Start","kind":"function","ty":"Bool"}]},{"name":"Actor","members":[{"name":"GetLevel","kind":"function","ty":"Int"}]},{"name":"Debug","members":[{"name":"Notification","kind":"function","is_global":true,"parameters":[{"name":"message","ty":"String"}]}]}]}"#).unwrap()
}

#[test]
fn external_schema_two_preserves_states_variables_and_property_access() {
    let bundle = decode(br#"{"schema":2,"package":{"name":"sdk","version":"1","source":"fixture","generator":"test"},"compatibility":{"target":"skyrim-se","abi":"papyrus-skyrim"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Base","members":[{"name":"count","kind":"variable","ty":"Int"},{"name":"Value","kind":"property","ty":"Int","is_read_only":true,"is_readable":true}],"states":[{"name":"Busy","auto":true,"members":[{"name":"Pulse","kind":"function","ty":"Int"}]}]}]}"#).unwrap();
    let world = script_from_external(&bundle.scripts[0]);
    assert_eq!(world.variables["count"].kind, MemberKind::Variable);
    assert!(world.members["value"].read_only);
    assert_eq!(world.states["busy"]["pulse"].ty, Type::Int);
}

#[test]
fn external_property_and_function_can_share_a_name() {
    let bundle = decode(br#"{"schema":2,"package":{"name":"sdk","version":"1","source":"fixture","generator":"test"},"compatibility":{"target":"skyrim-se","abi":"papyrus-skyrim"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Base","members":[{"name":"Check","kind":"property","ty":"Bool","is_auto":true,"is_readable":true,"is_writable":true},{"name":"Check","kind":"function","ty":"Bool"}]}]}"#).unwrap();
    let external = script_from_external(&bundle.scripts[0]);
    assert_eq!(external.members["check"].kind, MemberKind::Property);
    assert_eq!(
        external.callable_overloads["check"].kind,
        MemberKind::Function
    );
    let mut host = AnalysisHost::new();
    host.set_external_declarations(vec![bundle]);
    source(
        &mut host,
        98,
        "ScriptName User Extends Base\nFunction Probe()\nBool value = Check\nCheck()\nEndFunction\n",
    );
    let diagnostics = host.view().diagnostics(FileId(98)).unwrap();
    assert!(
        diagnostics.iter().all(
            |item| item.code != "semantic.not-callable" && item.code != "semantic.unknown-name"
        )
    );
}

#[test]
fn rejects_assignment_to_external_read_only_property() {
    let bundle = decode(br#"{"schema":2,"package":{"name":"sdk","version":"1","source":"fixture","generator":"test"},"compatibility":{"target":"skyrim-se","abi":"papyrus-skyrim"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Base","members":[{"name":"Value","kind":"property","ty":"Int","is_auto":true,"is_read_only":true,"is_readable":true}]}]}"#).unwrap();
    let mut host = AnalysisHost::new();
    host.set_external_declarations(vec![bundle]);
    source(
        &mut host,
        99,
        "ScriptName User Extends Base\nFunction Change()\nValue = 3\nEndFunction\n",
    );
    let diagnostics = host.view().diagnostics(FileId(99)).unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code == "semantic.read-only-property")
    );
}

#[test]
fn binds_inheritance_external_calls_and_local_definition() {
    let mut host = AnalysisHost::new();
    host.set_external_declarations(vec![sdk()]);
    let text = "Scriptname Arena extends Quest\nActor Property PlayerRef Auto\nInt Function Reward(Int bonus)\nInt level = PlayerRef.GetLevel()\nStart()\nDebug.Notification(\"ready\")\nReturn level + bonus\nEndFunction\n";
    source(&mut host, 1, text);
    let view = host.view();
    assert!(
        view.diagnostics(FileId(1)).unwrap().is_empty(),
        "{:?}",
        view.diagnostics(FileId(1))
    );
    let level_use = text.rfind("level +").unwrap();
    let level_definition = text.find("level =").unwrap();
    assert_eq!(view.type_at(FileId(1), level_use), Some(Type::Int));
    assert_eq!(
        view.definition(FileId(1), level_use).unwrap().range.start,
        level_definition
    );
    let script = view.hir(FileId(1)).unwrap();
    assert_eq!(script.calls.len(), 3);
}

#[test]
fn reports_local_errors_without_losing_sibling_facts() {
    let mut host = AnalysisHost::new();
    let text = "Scriptname Sample\nInt Function Broken()\nReturn Missing + 1\nEndFunction\nInt Function Good()\nReturn 2\nEndFunction\n";
    source(&mut host, 2, text);
    let view = host.view();
    let diagnostics = view.diagnostics(FileId(2)).unwrap();
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.unknown-name")
            .count(),
        1
    );
    assert_eq!(
        view.type_at(FileId(2), text.find('2').unwrap()),
        Some(Type::Int)
    );
    assert_eq!(view.hir(FileId(2)).unwrap().bodies.len(), 2);
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
    assert!(view.diagnostics(FileId(20)).unwrap().is_empty());
    let script = view.hir(FileId(20)).unwrap();
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
            .unwrap()
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
            .unwrap()
            .iter()
            .filter(|item| item.code == "semantic.argument-count")
            .count(),
        5
    );
    host.set_fill_missing_arguments(true);
    let compatible = host.view();
    let issues = compatible.diagnostics(FileId(21)).unwrap();
    assert_eq!(issues.len(), 5, "{issues:?}");
    assert!(issues.iter().all(|item| {
        item.code == "semantic.argument-defaulted" && item.severity == Severity::Warning
    }));
    let script = compatible.hir(FileId(21)).unwrap();
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
            .unwrap()
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
    let diagnostics = view.diagnostics(FileId(3)).unwrap();
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
            .unwrap()
            .iter()
            .any(|item| item.code == "semantic.unknown-flag")
    );
    assert!(
        view.diagnostics(FileId(3))
            .unwrap()
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
        view.diagnostics(FileId(9)).unwrap().is_empty(),
        "{:?}",
        view.diagnostics(FileId(9))
    );
    let script = view.hir(FileId(9)).unwrap();
    assert!(script.calls.iter().any(|call| matches!(&call.target, Some(Symbol::Intrinsic { name }) if name.eq_ignore_ascii_case("Find"))));
    assert!(script.calls.iter().any(|call| matches!(&call.target, Some(Symbol::Intrinsic { name }) if name.eq_ignore_ascii_case("GotoState"))));
    let go = script
        .bodies
        .iter()
        .find(|body| matches!(&body.symbol, Symbol::Member { name, .. } if name == "Go"))
        .unwrap();
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
        view.diagnostics(FileId(14)).unwrap().is_empty(),
        "{:?}",
        view.diagnostics(FileId(14))
    );
    let script = view.hir(FileId(14)).unwrap();
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
    let diagnostics = host.view().diagnostics(FileId(15)).unwrap();
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.operator-type")
            .count(),
        3
    );
}

#[test]
fn compiler_state_intrinsics_take_precedence_over_conflicting_sdk_signatures() {
    let mut host = AnalysisHost::new();
    let conflicting_sdk = decode(br#"{"schema":1,"package":{"name":"sdk","version":"1","source":"fixture","generator":"test"},"compatibility":{"target":"skyrim-se","abi":"papyrus-skyrim"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Base","members":[{"name":"GetState","kind":"function","ty":"Int","parameters":[{"name":"wrong","ty":"Int"}]},{"name":"GotoState","kind":"function","ty":"Int"}]}]}"#).unwrap();
    host.set_external_declarations(vec![conflicting_sdk]);
    source(
        &mut host,
        12,
        "Scriptname StateConsumer extends Base\nBase Property Other Auto\nString Function Probe()\nString own = GetState()\nString qualified = Self.GetState()\nString remote = Other.GetState()\nGotoState(\"Ready\")\nOther.GotoState(\"Idle\")\nReturn remote\nEndFunction\n",
    );
    let view = host.view();
    assert!(
        view.diagnostics(FileId(12)).unwrap().is_empty(),
        "{:?}",
        view.diagnostics(FileId(12))
    );
    let script = view.hir(FileId(12)).unwrap();
    assert_eq!(script.calls.len(), 5);
    assert!(script.calls.iter().all(|call| {
            matches!(&call.target, Some(Symbol::Intrinsic { name }) if name.eq_ignore_ascii_case("GetState") || name.eq_ignore_ascii_case("GotoState"))
        }));
    assert_eq!(
        script
            .calls
            .iter()
            .filter(|call| call.result == Type::String)
            .count(),
        3
    );
    assert_eq!(
        script
            .calls
            .iter()
            .filter(|call| call.result == Type::Void)
            .count(),
        2
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
    assert!(view.diagnostics(FileId(11)).unwrap().is_empty());
    let script = view.hir(FileId(11)).unwrap();
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
        view.diagnostics(FileId(13)).unwrap().is_empty(),
        "{:?}",
        view.diagnostics(FileId(13))
    );
    let script = view.hir(FileId(13)).unwrap();
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
            Symbol::Local { owner, name } if name == "local" => Some(owner.as_ref()),
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
    let diagnostics = view.diagnostics(FileId(8)).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let script = view.hir(FileId(8)).unwrap();
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
fn external_replacement_preserves_old_view_and_exposes_sdk_errors() {
    let mut host = AnalysisHost::new();
    source(
        &mut host,
        9,
        "Scriptname Client\nInt Function Value()\nReturn Api.Fetch()\nEndFunction\n",
    );
    let missing = host.view();
    assert!(
        missing
            .diagnostics(FileId(9))
            .unwrap()
            .iter()
            .any(|item| item.code == "semantic.unknown-name")
    );
    let bundle = decode(br#"{"schema":1,"package":{"name":"qa","version":"1","source":"fixture","generator":"test"},"compatibility":{"target":"skyrim-se","abi":"papyrus-skyrim"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Api","parent":"Absent","members":[{"name":"Fetch","kind":"function","ty":"Int","is_global":true}]}]}"#).unwrap();
    host.set_external_declarations(vec![bundle]);
    let current = host.view();
    assert!(current.diagnostics(FileId(9)).unwrap().is_empty());
    assert!(
        current
            .project_diagnostics()
            .iter()
            .any(|item| item.code == "semantic.unknown-parent")
    );
    assert!(
        missing
            .diagnostics(FileId(9))
            .unwrap()
            .iter()
            .any(|item| item.code == "semantic.unknown-name")
    );
    assert!(current.generation() > missing.generation());
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
            .unwrap()
            .iter()
            .any(|item| item.code == "semantic.duplicate-script")
    );
    assert!(
        view.hir(FileId(10))
            .unwrap()
            .declarations
            .iter()
            .any(|item| matches!(&item.symbol, Symbol::Member { name, .. } if name == "First"))
    );
    assert!(
        !view
            .hir(FileId(10))
            .unwrap()
            .declarations
            .iter()
            .any(|item| matches!(&item.symbol, Symbol::Member { name, .. } if name == "Second"))
    );
}

#[test]
fn invalid_external_types_are_reported_without_a_source_span() {
    let mut host = AnalysisHost::new();
    let bundle = decode(br#"{"schema":1,"package":{"name":"qa","version":"1","source":"fixture","generator":"test"},"compatibility":{"target":"skyrim-se","abi":"papyrus-skyrim"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Api","members":[{"name":"Fetch","kind":"function","ty":"MissingType"}]}]}"#).unwrap();
    host.set_external_declarations(vec![bundle]);
    let diagnostics = host.view().project_diagnostics();
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code == "semantic.unknown-type" && item.primary.is_none())
    );
}
