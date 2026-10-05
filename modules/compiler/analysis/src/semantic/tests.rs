use super::*;
use folio_format_declarations::decode;

mod arguments;
mod integers;

fn analyze_sources(sources: &[&str]) -> (Vec<Diagnostic>, Vec<Script>) {
    let mut host = crate::AnalysisHost::new();
    for (index, text) in sources.iter().enumerate() {
        host.upsert(
            FileId(index as u32),
            folio_source::Revision(1),
            std::sync::Arc::from(*text),
            folio_papyrus::PapyrusDialect::Skyrim,
        )
        .unwrap();
    }
    let view = host.view();
    let mut diagnostics = Vec::new();
    let mut scripts = Vec::new();
    for index in 0..sources.len() {
        let file = FileId(index as u32);
        diagnostics.extend(view.diagnostics(file).unwrap());
        scripts.push((*view.hir(file).unwrap()).clone());
    }
    (diagnostics, scripts)
}

#[test]
fn contextual_string_conversions_and_explicit_scalar_casts_are_typed() {
    let (diagnostics, scripts) = analyze_sources(&[
        "ScriptName Conversions\nFunction Take(String text) Native\nString Function Run()\n String text = 1\n text = True\n Take(Self)\n Int integer = True As Int\n Float real = \"1.5\" As Float\n String[] values\n text = values\n Return integer\nEndFunction\n",
    ]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(scripts[0].bodies.iter().flat_map(|body| &body.statements).any(|statement| matches!(statement, Statement::Variable { value: Some(value), .. } if value.ty == Type::Int && value.conversion == Some(Type::String))));
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName Invalid\nBool Function Run()\n Return 1 == None\nEndFunction\n",
    ]);
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code == "semantic.operator-type")
    );
}

#[test]
fn object_casts_follow_ancestry_and_arrays_remain_invariant() {
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName Base\n",
        "ScriptName Child Extends Base\n",
        "ScriptName Sibling Extends Base\n",
        "ScriptName Runner\nFunction Run(Child child, Base base, Sibling sibling, Child[] children)\n Base up = child As Base\n Child down = base As Child\n Sibling wrong = child As Sibling\n Base[] wrongArray = children As Base[]\nEndFunction\n",
    ]);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.invalid-cast")
            .count(),
        2,
        "{diagnostics:?}"
    );
}

#[test]
fn sibling_locals_keep_distinct_identity_and_do_not_escape() {
    let (diagnostics, scripts) = analyze_sources(&[
        "ScriptName Scopes\nFunction Run()\n If True\n  Int item = 1\n Else\n  Int item = 2\n EndIf\n item = 3\nEndFunction\n",
    ]);
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code == "semantic.unknown-name")
    );
    assert!(
        !diagnostics
            .iter()
            .any(|item| item.code == "semantic.duplicate-local")
    );
    let symbols: Vec<_> = scripts[0]
        .declarations
        .iter()
        .filter_map(|fact| matches!(fact.symbol, Symbol::Local { .. }).then_some(&fact.symbol))
        .collect();
    assert_eq!(symbols.len(), 2);
    assert_ne!(symbols[0], symbols[1]);
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName Conflicts\nInt item\nFunction Run()\n Int item\n Int nested\n If True\n  Int nested\n EndIf\nEndFunction\n",
    ]);
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code == "semantic.local-member-conflict")
    );
    assert!(
        diagnostics
            .iter()
            .any(|item| item.code == "semantic.duplicate-local")
    );
}

#[test]
fn unused_parameter_defaults_are_checked() {
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName Defaults\nFunction Wrong(Int amount = True, Int required) Native\nFunction Right(Float amount = 1, Defaults object = None) Native\n",
    ]);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.parameter-default")
            .count(),
        1,
        "{diagnostics:?}"
    );
}

#[test]
fn property_reads_and_writes_require_their_own_accessors() {
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName Access\nInt stored\nInt Property Readable\n Int Function Get()\n  Return stored\n EndFunction\nEndProperty\nInt Property Writable\n Function Set(Int value)\n  stored = value\n EndFunction\nEndProperty\nFunction Run()\n Int allowed = Readable\n Writable = 1\n Readable = 1\n Int denied = Writable\n Writable += 1\nEndFunction\n",
    ]);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.read-only-property")
            .count(),
        1,
        "{diagnostics:?}"
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.write-only-property")
            .count(),
        2,
        "{diagnostics:?}"
    );
}

#[test]
fn inherited_properties_and_named_state_contracts_are_enforced() {
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName Base\nInt Property Value Auto\nFunction Existing(Int amount) Native\n",
        "ScriptName Child Extends Base\nInt Property Value Auto\nState Busy\n Function Existing(Int amount)\n EndFunction\n Function Orphan()\n EndFunction\n Event OnBeginState()\n EndEvent\nEndState\nState Other\n Function Orphan()\n EndFunction\nEndState\n",
    ]);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.property-override")
            .count(),
        1,
        "{diagnostics:?}"
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.state-contract")
            .count(),
        2,
        "{diagnostics:?}"
    );
}

#[test]
fn parent_only_serves_as_a_callable_receiver() {
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName Base\nFunction Work() Native\nInt Property Value Auto\n",
        "ScriptName Child Extends Base\nChild Function Run()\n Parent.Work()\n Int value = Parent.Value\n Return Parent\nEndFunction\n",
    ]);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.parent-context")
            .count(),
        2,
        "{diagnostics:?}"
    );
}

#[test]
fn intrinsic_named_arguments_use_ck_parameter_names() {
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName Intrinsics\nFunction Run(Int[] values)\n Int forward = values.Find(akElement = 1, aiStartIndex = 0)\n Int reverse = values.RFind(akElement = 1, aiStartIndex = -1)\n GotoState(asNewState = \"\")\nEndFunction\n",
    ]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn cast_type_ignores_continuation_trivia() {
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName ContinuedCast\nFloat Function Run()\n Return 1 As \\ ; note\n Float\nEndFunction\n",
    ]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn declaration_only_parents_supply_state_contracts_and_property_permissions() {
    let bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"Value","kind":"property","ty":"Int","access":{"kind":"manual","readable":false,"writable":true}},{"name":"Work","kind":"function","parameters":[{"name":"amount","ty":"Int"}]}]}]}"#).unwrap();
    let mut host = crate::AnalysisHost::new();
    let file = FileId(0);
    host.upsert(file, folio_source::Revision(1), std::sync::Arc::from("ScriptName Child Extends Base\nState Busy\n Function Work(Int amount)\n  Value = amount\n  Int invalidRead = Value\n EndFunction\nEndState\n"), folio_papyrus::PapyrusDialect::Skyrim).unwrap();
    host.set_external_declarations(vec![bundle]);
    let diagnostics = host.view().diagnostics(file).unwrap();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "semantic.write-only-property");
}

#[test]
fn named_state_signatures_cannot_change_empty_state_parameters() {
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName Mismatch\nFunction Work(Int amount) Native\nState Busy\n Function Work(Float amount)\n EndFunction\nEndState\n",
    ]);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.state-contract")
            .count(),
        1,
        "{diagnostics:?}"
    );
}

#[test]
fn named_state_oninit_requires_an_empty_state_declaration() {
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName InitContract\nState Busy\n Event OnInit()\n EndEvent\n Event OnBeginState()\n EndEvent\n Event OnEndState()\n EndEvent\nEndState\n",
    ]);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.state-contract")
            .count(),
        1,
        "{diagnostics:?}"
    );
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName InitContract\nEvent OnInit()\nEndEvent\nState Busy\n Event OnInit()\n EndEvent\nEndState\n",
    ]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn unused_overflowing_defaults_and_initializers_are_rejected() {
    let (diagnostics, _) = analyze_sources(&[
        "ScriptName Overflow\nInt large = 2147483648\nFunction Wrong(Int amount = 2147483648) Native\nFunction Right(Int amount = -2147483648) Native\n",
    ]);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.parameter-default")
            .count(),
        1,
        "{diagnostics:?}"
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.initializer-type")
            .count(),
        1,
        "{diagnostics:?}"
    );
}

#[test]
fn external_default_literals_are_validated_before_any_call() {
    let mut bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"Integer","kind":"function","parameters":[{"name":"amount","ty":"Int","default":{"kind":"literal","value":"1"}}]},{"name":"Text","kind":"function","parameters":[{"name":"value","ty":"String","default":{"kind":"literal","value":"\"ok\""}}]}]}]}"#).unwrap();
    // In-memory selected APIs can bypass carrier decoding, so the semantic boundary
    // independently rejects values that cannot be represented in the target.
    for (member, literal) in bundle.scripts[0]
        .members
        .iter_mut()
        .zip(["2147483648", "\"bad\\q\""])
    {
        if let folio_format_declarations::MemberData::Function { parameters, .. } = &mut member.data
        {
            parameters[0].default = ParameterDefault::Literal(literal.into());
        }
    }
    let mut host = crate::AnalysisHost::new();
    host.set_external_declarations(vec![bundle]);
    let diagnostics = host.view().project_diagnostics();
    assert_eq!(
        diagnostics
            .iter()
            .filter(|item| item.code == "semantic.parameter-default-type")
            .count(),
        2,
        "{diagnostics:?}"
    );
}

#[test]
fn external_initializers_check_type_range_and_string_validity() {
    let mut bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"Integer","kind":"variable","ty":"Int","initial_literal":"1"},{"name":"WrongType","kind":"property","ty":"Int","access":{"kind":"auto"},"initial_literal":"1"},{"name":"Text","kind":"property","ty":"String","access":{"kind":"auto-read-only"},"initial_literal":"\"ok\""}]}]}"#).unwrap();
    for (member, literal) in
        bundle.scripts[0]
            .members
            .iter_mut()
            .zip(["2147483648", "True", "\"bad\\q\""])
    {
        match &mut member.data {
            folio_format_declarations::MemberData::Variable {
                initial_literal, ..
            }
            | folio_format_declarations::MemberData::Property {
                initial_literal, ..
            } => *initial_literal = Some(literal.into()),
            _ => unreachable!(),
        }
    }
    let mut host = crate::AnalysisHost::new();
    host.set_external_declarations(vec![bundle]);
    let diagnostics = host.view().project_diagnostics();
    assert_eq!(diagnostics.len(), 3, "{diagnostics:?}");
    assert!(
        diagnostics
            .iter()
            .all(|item| item.code == "semantic.initializer-type")
    );
}

#[test]
fn external_initializers_accept_reference_none_and_int_to_float_without_requiring_unknown_values() {
    let bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"Instance","kind":"variable","ty":"Base","initial_literal":"None"},{"name":"Values","kind":"property","ty":"Int[]","access":{"kind":"auto"},"initial_literal":"None"},{"name":"Amount","kind":"variable","ty":"Float","initial_literal":"1"},{"name":"UnknownReadOnly","kind":"property","ty":"Int","access":{"kind":"auto-read-only"}}]}]}"#).unwrap();
    let mut host = crate::AnalysisHost::new();
    host.set_external_declarations(vec![bundle]);
    let diagnostics = host.view().project_diagnostics();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn completion_filters_locals_outside_their_lexical_block() {
    let text = "ScriptName Completion\nFunction Run()\n If True\n  Int hidden\n EndIf\n Int visible\nEndFunction\n";
    let mut host = crate::AnalysisHost::new();
    let file = FileId(0);
    host.upsert(
        file,
        folio_source::Revision(1),
        std::sync::Arc::from(text),
        folio_papyrus::PapyrusDialect::Skyrim,
    )
    .unwrap();
    let view = host.view();
    let candidates = view
        .completion_candidates(
            file,
            text.find("EndFunction").unwrap(),
            None,
            false,
            "",
            &|| false,
        )
        .unwrap();
    assert!(!candidates.iter().any(
        |candidate| matches!(&candidate.symbol, Symbol::Local { name, .. } if name == "hidden")
    ));
    assert!(candidates.iter().any(
        |candidate| matches!(&candidate.symbol, Symbol::Local { name, .. } if name == "visible")
    ));
}

#[test]
fn external_declarations_preserve_states_variables_and_property_access() {
    let bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"count","kind":"variable","ty":"Int"},{"name":"Value","kind":"property","ty":"Int","access":{"kind":"manual","readable":true,"writable":false}}],"states":[{"name":"Busy","auto":true,"members":[{"name":"Pulse","kind":"function","return_type":"Int"}]}]}]}"#).unwrap();
    let world = script_from_external(&bundle.scripts[0]);
    assert_eq!(world.variables["count"].kind, MemberKind::Variable);
    assert!(world.members["value"].read_only);
    assert_eq!(world.states["busy"]["pulse"].ty, Type::Int);
}

#[test]
fn external_model_keeps_property_and_callable_namespaces() {
    let bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"Check","kind":"property","ty":"Bool","access":{"kind":"auto"}},{"name":"Check","kind":"function","return_type":"Bool"}]}]}"#).unwrap();
    let external = script_from_external(&bundle.scripts[0]);
    assert_eq!(external.members["check"].kind, MemberKind::Property);
    assert_eq!(
        external.callable_overloads["check"].kind,
        MemberKind::Function
    );
}
