use super::*;

#[test]
fn source_mask_initializers_and_defaults_accept_full_width_hex() {
    let (diagnostics, scripts) = analyze_sources(&[
        "ScriptName Masks\nInt Property HighBit = 0x80000000 AutoReadOnly\nInt allBits = 0xFFFFFFFF\nFunction Take(Int mask = 0xFFFFFFFF) Native\nFunction Run()\nTake()\nEndFunction\n",
    ]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let call = scripts[0]
        .bodies
        .iter()
        .flat_map(|body| &body.statements)
        .find_map(|statement| {
            if let Statement::Expression(call) = statement {
                Some(call)
            } else {
                None
            }
        })
        .expect("expected call");
    let ExpressionKind::Call {
        parameter_defaults, ..
    } = &call.kind
    else {
        panic!("expected call");
    };
    assert_eq!(
        parameter_defaults,
        &[Some((Type::Int, "0xFFFFFFFF".into()))]
    );
}

#[test]
fn external_mask_initializers_and_defaults_preserve_their_declared_values() {
    let bundle = decode(br#"{"format":"folio-declarations","schema":2,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Masks","members":[{"name":"HighBit","kind":"property","ty":"Int","access":{"kind":"auto-read-only"},"initial_literal":"0x80000000"},{"name":"Take","kind":"function","parameters":[{"name":"mask","ty":"Int","default":{"kind":"literal","value":"0xFFFFFFFF"}}]}]}]}"#).unwrap();
    let mut host = crate::AnalysisHost::new();
    host.set_external_declarations(vec![bundle]);
    host.upsert(FileId(0), folio_source::Revision(1), std::sync::Arc::from(
        "ScriptName UseMasks Extends Masks\nFunction Run()\nTake()\nInt value = HighBit\nEndFunction\n"
    ), folio_papyrus::PapyrusDialect::Skyrim).unwrap();
    let view = host.view();
    assert_eq!(
        view.project_diagnostics(),
        [] as [folio_diagnostics::Diagnostic; 0]
    );
    let issues = view.diagnostics(FileId(0)).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    let script = view.hir(FileId(0)).unwrap();
    let Statement::Expression(call) = &script.bodies[0].statements[0] else {
        panic!("expected call");
    };
    let ExpressionKind::Call {
        parameter_defaults, ..
    } = &call.kind
    else {
        panic!("expected call");
    };
    assert_eq!(
        parameter_defaults,
        &[Some((Type::Int, "0xFFFFFFFF".into()))]
    );
}
