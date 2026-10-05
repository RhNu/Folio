use std::{cell::Cell, sync::Arc};

use super::*;

fn view(sources: &[&str]) -> AnalysisView {
    let mut host = crate::AnalysisHost::new();
    for (index, text) in sources.iter().enumerate() {
        host.upsert(
            FileId(u32::try_from(index).expect("test fixture fits u32")),
            folio_source::Revision(1),
            Arc::from(*text),
            folio_papyrus::PapyrusDialect::Skyrim,
        )
        .unwrap();
    }
    host.view()
}

#[test]
fn ordered_prefix_range_handles_empty_missing_and_neighboring_keys() {
    let entries = BTreeMap::from([
        ("alpha".into(), 1),
        ("alphabet".into(), 2),
        ("alpine".into(), 3),
        ("beta".into(), 4),
    ]);
    assert_eq!(
        prefix_entries(&entries, "alpha")
            .map(|(_, value)| *value)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(prefix_entries(&entries, "").count(), 4);
    assert_eq!(prefix_entries(&entries, "azure").count(), 0);
    assert_eq!(prefix_entries(&entries, "zulu").count(), 0);
}

#[test]
fn prefix_selects_scripts_case_insensitively_without_losing_empty_prefix_results() {
    let text = "ScriptName User\nFunction Run()\nEndFunction\n";
    let view = view(&[
        text,
        "ScriptName Alpha\n",
        "ScriptName Alpine\n",
        "ScriptName Beta\n",
    ]);
    let at = text.find("EndFunction").unwrap();
    let candidates = view
        .completion_candidates(FileId(0), at, None, false, "aLP", &|| false)
        .unwrap();
    assert_eq!(
        candidates
            .iter()
            .map(|item| symbol_name(&item.symbol))
            .collect::<Vec<_>>(),
        ["Alpha", "Alpine"]
    );
    let all = view
        .completion_candidates(FileId(0), at, None, false, "", &|| false)
        .unwrap();
    assert_eq!(
        all.iter()
            .filter_map(|item| match &item.symbol {
                Symbol::Script(name) => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        ["Alpha", "Alpine", "Beta", "User"]
    );
}

#[test]
fn prefixed_names_keep_local_and_state_precedence_over_inherited_members() {
    let text = "ScriptName User Extends Base\nInt Function Compute() Native\nState Busy\n Int Function Compute() Native\n Function Run()\n  Int Compute = 1\n  Compute = 2\n EndFunction\nEndState\n";
    let view = view(&[text, "ScriptName Base\nInt Function Compute() Native\n"]);
    let state = view
        .completion_candidates(
            FileId(0),
            text.find("Int Compute =").unwrap(),
            None,
            false,
            "cOM",
            &|| false,
        )
        .unwrap();
    assert!(
        matches!(&state[0].symbol, Symbol::StateMember { script, state, name } if script == "User" && state == "Busy" && name == "Compute")
    );
    let local = view
        .completion_candidates(
            FileId(0),
            text.find("Compute = 2").unwrap(),
            None,
            false,
            "cOM",
            &|| false,
        )
        .unwrap();
    assert_eq!(local.len(), 1);
    assert!(matches!(&local[0].symbol, Symbol::Local { name, .. } if name == "Compute"));
}

#[test]
fn prefixed_imports_preserve_ambiguity_and_inherited_global_resolution() {
    let text = "ScriptName User\nImport Child\nImport Other\nFunction Run()\nEndFunction\n";
    let view = view(&[
        text,
        "ScriptName Base\nFunction Utility() Global Native\nFunction Unique() Global Native\n",
        "ScriptName Child Extends Base\n",
        "ScriptName Other\nFunction Utility() Global Native\n",
    ]);
    let items = view
        .completion_candidates(
            FileId(0),
            text.find("EndFunction").unwrap(),
            None,
            false,
            "u",
            &|| false,
        )
        .unwrap();
    assert!(
        !items
            .iter()
            .any(|item| symbol_name(&item.symbol) == "Utility")
    );
    assert!(items.iter().any(|item| matches!(&item.symbol, Symbol::Member { script, name } if script == "Base" && name == "Unique")));
}

#[test]
fn candidate_enumeration_cancels_after_warmup_and_can_retry() {
    let text = "ScriptName User\nFunction Run()\nEndFunction\n";
    let view = view(&[text, "ScriptName Alpha\n", "ScriptName Beta\n"]);
    view.try_warm_semantics(|| false).unwrap();
    let checks = Cell::new(0);
    let cancelled = || {
        checks.set(checks.get() + 1);
        checks.get() >= 3
    };
    assert_eq!(
        view.completion_candidates(
            FileId(0),
            text.find("EndFunction").unwrap(),
            None,
            false,
            "",
            &cancelled
        ),
        Err(AnalysisCancelled)
    );
    let retry = view
        .completion_candidates(
            FileId(0),
            text.find("EndFunction").unwrap(),
            None,
            false,
            "a",
            &|| false,
        )
        .unwrap();
    assert_eq!(retry.len(), 1);
    assert_eq!(symbol_name(&retry[0].symbol), "Alpha");
}

#[test]
fn ascii_prefix_matching_does_not_accept_unicode_case_folding() {
    assert!(matches_prefix("Kept", "kE"));
    assert!(!matches_prefix("\u{212a}ept", "ke"));
    assert!(!matches_prefix("Äpfel", "ä"));
    assert!(matches_prefix("Äpfel", "Ä"));
}

fn external_view(text: &str, declarations: &[u8]) -> AnalysisView {
    let mut host = crate::AnalysisHost::new();
    host.upsert(
        FileId(0),
        folio_source::Revision(1),
        Arc::from(text),
        folio_papyrus::PapyrusDialect::Skyrim,
    )
    .unwrap();
    host.set_external_declarations(vec![
        folio_format_declarations::decode(declarations).unwrap(),
    ]);
    host.view()
}

#[test]
fn filtered_unicode_member_still_shadows_an_ascii_script_name() {
    let text = "ScriptName User Extends Api\nFunction Run()\nEndFunction\n";
    let declarations = r#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Api","members":[{"name":"\u212aept","kind":"property","ty":"Int","access":{"kind":"auto"}}]},{"name":"Kept","members":[]}]}"#;
    let view = external_view(text, declarations.as_bytes());
    let at = text.find("EndFunction").unwrap();
    let filtered = view
        .completion_candidates(FileId(0), at, None, false, "k", &|| false)
        .unwrap();
    assert!(filtered.is_empty());
    let all = view
        .completion_candidates(FileId(0), at, None, false, "", &|| false)
        .unwrap();
    assert!(
        all.iter().any(
            |item| matches!(&item.symbol, Symbol::Member { name, .. } if name == "\u{212a}ept")
        )
    );
    assert!(
        !all.iter()
            .any(|item| matches!(&item.symbol, Symbol::Script(name) if name == "Kept"))
    );
}

#[test]
fn filtered_unicode_import_still_participates_in_ambiguity() {
    let text = "ScriptName User\nImport One\nImport Two\nFunction Run()\nEndFunction\n";
    let declarations = r#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"One","members":[{"name":"Kept","kind":"function","return_type":"Int","global":true}]},{"name":"Two","members":[{"name":"\u212aept","kind":"function","return_type":"Int","global":true}]}]}"#;
    let view = external_view(text, declarations.as_bytes());
    let filtered = view
        .completion_candidates(
            FileId(0),
            text.find("EndFunction").unwrap(),
            None,
            false,
            "k",
            &|| false,
        )
        .unwrap();
    assert!(filtered.is_empty());
}

#[test]
fn out_of_range_cursor_keeps_nonlocal_candidates_without_panicking() {
    let text = "ScriptName User\nFunction Run(Int argument)\nEndFunction\n";
    let view = view(&[text, "ScriptName Alpha\n"]);
    for at in [text.len() + 1, usize::MAX] {
        let items = view
            .completion_candidates(FileId(0), at, None, false, "a", &|| false)
            .unwrap();
        assert_eq!(items.len(), 1);
        assert!(matches!(&items[0].symbol, Symbol::Script(name) if name == "Alpha"));
    }
}

#[test]
fn callable_end_boundary_retains_parameter_visibility() {
    let text = "ScriptName User\nFunction Run(Int argument)\nEndFunction\n";
    let view = view(&[text]);
    let items = view
        .completion_candidates(
            FileId(0),
            text.find("EndFunction").unwrap() + "EndFunction".len(),
            None,
            false,
            "arg",
            &|| false,
        )
        .unwrap();
    assert_eq!(items.len(), 1);
    assert!(matches!(&items[0].symbol, Symbol::Parameter { name, .. } if name == "argument"));
}
