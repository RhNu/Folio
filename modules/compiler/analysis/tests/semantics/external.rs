//! External declaration namespaces, contracts, and immutable view replacement.

use folio_analysis::AnalysisHost;
use folio_diagnostics::Severity;
use folio_format_declarations::decode;
use folio_hir::{ExpressionKind, Statement, Symbol, Type};
use folio_source::FileId;

use super::source;

fn declarations() -> folio_format_declarations::DeclarationBundle {
    decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Quest","members":[{"name":"Start","kind":"function","return_type":"Bool"}]},{"name":"Actor","members":[{"name":"GetLevel","kind":"function","return_type":"Int"}]},{"name":"Debug","members":[{"name":"Notification","kind":"function","parameters":[{"name":"message","ty":"String"}],"global":true}]}]}"#).expect("synthetic test input is valid")
}

#[test]
fn external_property_and_function_can_share_a_name() {
    let bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"Check","kind":"property","ty":"Bool","access":{"kind":"auto"}},{"name":"Check","kind":"function","return_type":"Bool"}]}]}"#).expect("synthetic test input is valid");
    let mut host = AnalysisHost::new();
    host.set_external_declarations(vec![bundle]);
    source(
        &mut host,
        98,
        "ScriptName User Extends Base\nFunction Probe()\nBool value = Check\nCheck()\nEndFunction\n",
    );
    let diagnostics = host
        .view()
        .diagnostics(FileId(98))
        .expect("synthetic test input is valid");
    assert!(
        diagnostics.iter().all(
            |item| item.code != "semantic.not-callable" && item.code != "semantic.unknown-name"
        )
    );
}

#[test]
fn rejects_assignment_to_external_read_only_property() {
    let bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"Value","kind":"property","ty":"Int","access":{"kind":"auto-read-only"}}]}]}"#).expect("synthetic test input is valid");
    let mut host = AnalysisHost::new();
    host.set_external_declarations(vec![bundle]);
    source(
        &mut host,
        99,
        "ScriptName User Extends Base\nFunction Change()\nValue = 3\nEndFunction\n",
    );
    let diagnostics = host
        .view()
        .diagnostics(FileId(99))
        .expect("synthetic test input is valid");
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code == "semantic.read-only-property")
    );
}

#[test]
fn binds_inheritance_external_calls_and_local_definition() {
    let mut host = AnalysisHost::new();
    host.set_external_declarations(vec![declarations()]);
    let text = "Scriptname Arena extends Quest\nActor Property PlayerRef Auto\nInt Function Reward(Int bonus)\nInt level = PlayerRef.GetLevel()\nStart()\nDebug.Notification(\"ready\")\nReturn level + bonus\nEndFunction\n";
    source(&mut host, 1, text);
    let view = host.view();
    assert!(
        view.diagnostics(FileId(1))
            .expect("synthetic test input is valid")
            .is_empty(),
        "{:?}",
        view.diagnostics(FileId(1))
    );
    let level_use = text
        .rfind("level +")
        .expect("synthetic test input is valid");
    let level_definition = text.find("level =").expect("synthetic test input is valid");
    assert_eq!(view.type_at(FileId(1), level_use), Some(Type::Int));
    assert_eq!(
        view.definition(FileId(1), level_use)
            .expect("synthetic test input is valid")
            .range
            .start,
        level_definition
    );
    let script = view.hir(FileId(1)).expect("synthetic test input is valid");
    assert_eq!(script.calls.len(), 3);
}

#[test]
fn unknown_callables_allow_explicit_calls_but_reject_unknown_defaults_and_overrides() {
    let bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"GetValue","kind":"unknown-callable","parameters":[{"name":"count","ty":"Int","default":{"kind":"unknown"}}],"return_type":"Int"}]}]}"#).expect("synthetic test input is valid");
    let mut host = AnalysisHost::new();
    host.set_external_declarations(vec![bundle]);
    host.set_fill_missing_arguments(true);
    source(
        &mut host,
        80,
        "ScriptName Child Extends Base\nFunction Probe()\nInt value = GetValue(1)\nEndFunction\n",
    );
    source(
        &mut host,
        81,
        "ScriptName Missing Extends Base\nFunction Probe()\nInt value = GetValue()\nEndFunction\n",
    );
    source(
        &mut host,
        82,
        "ScriptName Override Extends Base\nInt Function GetValue(Int count)\nReturn count\nEndFunction\n",
    );
    let view = host.view();
    assert!(
        view.diagnostics(FileId(80))
            .expect("synthetic test input is valid")
            .iter()
            .all(|item| item.severity != Severity::Error)
    );
    assert!(
        view.diagnostics(FileId(81))
            .expect("synthetic test input is valid")
            .iter()
            .any(|item| item.code == "semantic.default-unavailable")
    );
    assert!(
        view.diagnostics(FileId(82))
            .expect("synthetic test input is valid")
            .iter()
            .any(|item| item.code == "semantic.override-ambiguous")
    );
}

#[test]
fn persisted_parameter_defaults_are_checked_individually() {
    use folio_format_declarations::{DeclarationFormat, encode};
    let declaration = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"Target","kind":"function","parameters":[{"name":"required","ty":"Int"},{"name":"known","ty":"Int","default":{"kind":"literal","value":"7"}},{"name":"unknown","ty":"Int","default":{"kind":"unknown"}}]}]}]}"#).expect("synthetic test input is valid");
    for format in [DeclarationFormat::Json, DeclarationFormat::Binary] {
        let mut host = AnalysisHost::new();
        host.set_external_declarations(vec![
            decode(&encode(&declaration, format).expect("synthetic test input is valid"))
                .expect("synthetic test input is valid"),
        ]);
        host.set_fill_missing_arguments(true);
        source(
            &mut host,
            90,
            "ScriptName Known Extends Base\nFunction Probe()\nTarget(unknown = 2)\nEndFunction\n",
        );
        source(
            &mut host,
            91,
            "ScriptName Unknown Extends Base\nFunction Probe()\nTarget(required = 1)\nEndFunction\n",
        );
        let view = host.view();
        let known = view
            .diagnostics(FileId(90))
            .expect("synthetic test input is valid");
        assert_eq!(known.len(), 1, "{known:?}");
        assert_eq!(known[0].code, "semantic.argument-defaulted");
        let unknown = view
            .diagnostics(FileId(91))
            .expect("synthetic test input is valid");
        assert_eq!(unknown.len(), 1, "{unknown:?}");
        assert_eq!(unknown[0].code, "semantic.default-unavailable");
        let script = view.hir(FileId(90)).expect("synthetic test input is valid");
        let Statement::Expression(call) = &script.bodies[0].statements[0] else {
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
                Some((Type::Int, "0".into())),
                Some((Type::Int, "7".into())),
                None
            ]
        );
    }
}

#[test]
fn compiler_state_intrinsics_take_precedence_over_conflicting_declarations_signatures() {
    let mut host = AnalysisHost::new();
    let conflicting_declarations = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"GetState","kind":"function","parameters":[{"name":"wrong","ty":"Int"}],"return_type":"Int"},{"name":"GotoState","kind":"function","return_type":"Int"}]}]}"#).expect("synthetic test input is valid");
    host.set_external_declarations(vec![conflicting_declarations]);
    source(
        &mut host,
        12,
        "Scriptname StateConsumer extends Base\nBase Property Other Auto\nString Function Probe()\nString own = GetState()\nString qualified = Self.GetState()\nString remote = Other.GetState()\nGotoState(\"Ready\")\nOther.GotoState(\"Idle\")\nReturn remote\nEndFunction\n",
    );
    let view = host.view();
    assert!(
        view.diagnostics(FileId(12))
            .expect("synthetic test input is valid")
            .is_empty(),
        "{:?}",
        view.diagnostics(FileId(12))
    );
    let script = view.hir(FileId(12)).expect("synthetic test input is valid");
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
fn external_replacement_preserves_old_view_and_exposes_declaration_errors() {
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
            .expect("synthetic test input is valid")
            .iter()
            .any(|item| item.code == "semantic.unknown-name")
    );
    let bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Api","parent":"Absent","members":[{"name":"Fetch","kind":"function","return_type":"Int","global":true}]}]}"#).expect("synthetic test input is valid");
    host.set_external_declarations(vec![bundle]);
    let current = host.view();
    assert_eq!(
        current
            .diagnostics(FileId(9))
            .expect("synthetic test input is valid"),
        [] as [folio_diagnostics::Diagnostic; 0]
    );
    assert!(
        current
            .project_diagnostics()
            .iter()
            .any(|item| item.code == "semantic.unknown-parent")
    );
    assert!(
        missing
            .diagnostics(FileId(9))
            .expect("synthetic test input is valid")
            .iter()
            .any(|item| item.code == "semantic.unknown-name")
    );
    assert!(current.generation() > missing.generation());
}

#[test]
fn invalid_external_types_are_reported_without_a_source_span() {
    let mut host = AnalysisHost::new();
    let bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Api","members":[{"name":"Fetch","kind":"function","return_type":"MissingType"}]}]}"#).expect("synthetic test input is valid");
    host.set_external_declarations(vec![bundle]);
    let diagnostics = host.view().project_diagnostics();
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code == "semantic.unknown-type" && item.primary.is_none())
    );
}
