use super::*;

/// Exercise the same call contract through source and decoded dependency declarations.
fn host_with_calls(calls: &str, external: bool) -> crate::AnalysisHost {
    let mut host = crate::AnalysisHost::new();
    if external {
        host.set_external_declarations(vec![decode(br#"{"format":"folio-declarations","schema":2,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Api","members":[{"name":"Travel","kind":"function","parameters":[{"name":"destination","ty":"Int","default":{"kind":"literal","value":"-1"}},{"name":"driver","ty":"Api"}]}]}]}"#).unwrap()]);
    } else {
        host.upsert(
            FileId(1),
            folio_source::Revision(1),
            std::sync::Arc::from(
                "ScriptName Api\nFunction Travel(Int destination = -1, Api driver) Native\n",
            ),
            folio_papyrus::PapyrusDialect::Skyrim,
        )
        .unwrap();
    }
    host.upsert(
        FileId(0),
        folio_source::Revision(1),
        std::sync::Arc::from(format!(
            "ScriptName Demo Extends Api\nFunction Run(Api driver)\n{calls}\nEndFunction\n"
        )),
        folio_papyrus::PapyrusDialect::Skyrim,
    )
    .unwrap();
    host
}

#[test]
fn nontrailing_defaults_bind_named_and_positional_calls_by_parameter_slot() {
    for external in [false, true] {
        let host = host_with_calls(
            "Travel(driver = driver)\nTravel(-1, driver)\nTravel(driver = driver, destination = 5)\nTravel(5, driver = driver)",
            external,
        );
        let view = host.view();
        assert!(view.project_diagnostics().is_empty());
        let issues = view.diagnostics(FileId(0)).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        if !external {
            assert!(view.diagnostics(FileId(1)).unwrap().is_empty());
        }
        let script = view.hir(FileId(0)).unwrap();
        let calls = &script.bodies[0].statements;
        assert_eq!(calls.len(), 4);
        for (statement, ordinals, defaults) in [
            (
                &calls[0],
                vec![1],
                vec![Some((Type::Int, "-1".into())), None],
            ),
            (&calls[1], vec![0, 1], vec![None, None]),
            (&calls[2], vec![1, 0], vec![None, None]),
            (&calls[3], vec![0, 1], vec![None, None]),
        ] {
            let Statement::Expression(call) = statement else {
                panic!("expected call")
            };
            let ExpressionKind::Call {
                argument_ordinals,
                parameter_defaults,
                ..
            } = &call.kind
            else {
                panic!("expected call")
            };
            assert_eq!(argument_ordinals, &ordinals);
            assert_eq!(parameter_defaults, &defaults);
        }
    }
}

#[test]
fn nontrailing_defaults_do_not_hide_required_gaps_or_invalid_bindings() {
    for external in [false, true] {
        for (call, expected) in [
            ("Travel()", vec!["semantic.argument-count"]),
            ("Travel(5)", vec!["semantic.argument-count"]),
            (
                "Travel(driver)",
                vec!["semantic.argument-count", "semantic.argument-type"],
            ),
            ("Travel(driver = 5)", vec!["semantic.argument-type"]),
            (
                "Travel(driver = driver, driver = driver)",
                vec!["semantic.duplicate-argument"],
            ),
            (
                "Travel(driver = driver, missing = 5)",
                vec!["semantic.unknown-argument"],
            ),
        ] {
            let host = host_with_calls(call, external);
            let issues = host.view().diagnostics(FileId(0)).unwrap();
            let mut codes: Vec<_> = issues.iter().map(|issue| issue.code.as_str()).collect();
            codes.sort_unstable();
            assert_eq!(codes, expected, "{call}, external={external}: {issues:?}");
        }
    }
}
