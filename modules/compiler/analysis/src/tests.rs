use super::*;

#[test]
fn unchanged_external_inputs_preserve_generation_and_changes_invalidate_semantics() {
    let mut host = AnalysisHost::new();
    let file = FileId(0);
    host.upsert(
        file,
        Revision(1),
        Arc::from("ScriptName Child Extends Base\n"),
        PapyrusDialect::Skyrim,
    )
    .unwrap();
    let unresolved = host.view();
    assert!(!unresolved.diagnostics(file).unwrap().is_empty());
    let bundle = folio_format_declarations::decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[]}]}"#).unwrap();
    host.set_external_declarations(vec![bundle.clone()]);
    let resolved = host.view();
    assert!(resolved.diagnostics(file).unwrap().is_empty());
    host.set_external_declarations(vec![bundle]);
    host.set_user_flags(vec![]);
    host.set_fill_missing_arguments(false);
    assert_eq!(host.view().generation(), resolved.generation());
    assert!(host.view().diagnostics(file).unwrap().is_empty());
    host.set_external_declarations(vec![]);
    assert!(host.view().generation() > resolved.generation());
    assert!(!host.view().diagnostics(file).unwrap().is_empty());
    assert!(resolved.diagnostics(file).unwrap().is_empty());
    assert!(!unresolved.diagnostics(file).unwrap().is_empty());
}

#[test]
fn flag_and_call_policy_changes_invalidate_warmed_views() {
    let mut host = AnalysisHost::new();
    let file = FileId(0);
    host.upsert(file, Revision(1), Arc::from("ScriptName Policy Custom\nFunction Required(Int value) Native\nFunction Run()\n Required()\nEndFunction\n"), PapyrusDialect::Skyrim).unwrap();
    let original = host.view();
    let errors = original.diagnostics(file).unwrap();
    assert!(
        errors
            .iter()
            .any(|diagnostic| diagnostic.severity == folio_diagnostics::Severity::Error)
    );
    host.set_user_flags(vec!["Custom".into()]);
    host.set_fill_missing_arguments(true);
    let allowed = host.view();
    assert!(
        allowed
            .diagnostics(file)
            .unwrap()
            .iter()
            .all(|diagnostic| diagnostic.severity != folio_diagnostics::Severity::Error)
    );
    host.set_user_flags(vec!["Custom".into()]);
    host.set_fill_missing_arguments(true);
    assert_eq!(host.view().generation(), allowed.generation());
    host.set_fill_missing_arguments(false);
    assert!(
        host.view()
            .diagnostics(file)
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic.severity == folio_diagnostics::Severity::Error)
    );
    assert_eq!(original.diagnostics(file).unwrap(), errors);
}

fn source(text: &str) -> Arc<str> {
    Arc::from(text)
}

#[test]
fn replacement_and_removal_leave_old_views_consistent() {
    let mut host = AnalysisHost::new();
    let file = FileId(7);
    host.upsert(
        file,
        Revision(1),
        source("Scriptname First\nFunction A() Native\n"),
        PapyrusDialect::Skyrim,
    )
    .unwrap();
    let old = host.view();
    host.upsert(
        file,
        Revision(2),
        source("Scriptname Second\nFunction B() Native\n"),
        PapyrusDialect::Skyrim,
    )
    .unwrap();
    let current = host.view();
    assert_eq!(
        old.text(file),
        Some("Scriptname First\nFunction A() Native\n")
    );
    assert_eq!(
        current.text(file),
        Some("Scriptname Second\nFunction B() Native\n")
    );
    assert_ne!(old.declarations(file), current.declarations(file));
    host.remove(file, Revision(3)).unwrap();
    assert!(host.view().parse(file).is_none());
    assert!(current.parse(file).is_some());
    assert_eq!(
        host.upsert(
            file,
            Revision(2),
            source("Scriptname Stale"),
            PapyrusDialect::Skyrim
        )
        .err(),
        Some(InputError::StaleRevision {
            file,
            current: Revision(3),
            proposed: Revision(2)
        })
    );
}

#[test]
fn invalid_batch_is_atomic_and_rejects_duplicate_ids() {
    let mut host = AnalysisHost::new();
    let a = FileId(1);
    let b = FileId(2);
    host.upsert(
        a,
        Revision(1),
        source("Scriptname A\n"),
        PapyrusDialect::Skyrim,
    )
    .unwrap();
    let before = host.view();
    let result = host.apply_batch([
        InputEdit::Upsert {
            file: a,
            revision: Revision(2),
            text: source("Scriptname Changed\n"),
            dialect: PapyrusDialect::Skyrim,
        },
        InputEdit::Remove {
            file: b,
            revision: Revision(1),
        },
    ]);
    assert_eq!(result, Err(InputError::UnknownFile(b)));
    assert_eq!(host.view().generation(), before.generation());
    assert_eq!(host.view().text(a), before.text(a));
    assert_eq!(
        host.apply_batch([
            InputEdit::Upsert {
                file: a,
                revision: Revision(2),
                text: source("Scriptname X"),
                dialect: PapyrusDialect::Skyrim
            },
            InputEdit::Remove {
                file: a,
                revision: Revision(3)
            },
        ]),
        Err(InputError::DuplicateFile(a))
    );
}

#[test]
fn body_offset_change_preserves_semantic_summaries() {
    let mut host = AnalysisHost::new();
    let file = FileId(3);
    host.upsert(
        file,
        Revision(1),
        source("Scriptname S\nFunction A()\nReturn 1\nEndFunction\nFunction B() Native\n"),
        PapyrusDialect::Skyrim,
    )
    .unwrap();
    let old = host.view();
    host.upsert(
        file,
        Revision(2),
        source("Scriptname S\nFunction A()\nReturn 123456789\nEndFunction\nFunction B() Native\n"),
        PapyrusDialect::Skyrim,
    )
    .unwrap();
    let new = host.view();
    assert_eq!(old.declarations(file), new.declarations(file));
    assert_ne!(
        old.located_declarations(file),
        new.located_declarations(file)
    );
}

#[test]
fn cancelled_semantic_warmup_does_not_publish_partial_facts() {
    use std::cell::Cell;

    let mut host = AnalysisHost::new();
    for id in 1..=3 {
        host.upsert(
            FileId(id),
            Revision(1),
            source(&format!(
                "Scriptname S{id}\nInt Function Value()\nReturn {id}\nEndFunction\n"
            )),
            PapyrusDialect::Skyrim,
        )
        .unwrap();
    }
    let view = host.view();
    let checks = Cell::new(0);
    let cancelled = || {
        checks.set(checks.get() + 1);
        checks.get() >= 4
    };
    assert_eq!(view.try_warm_semantics(cancelled), Err(AnalysisCancelled));
    assert!(view.semantic.get().is_none());
    assert_eq!(view.try_warm_semantics(|| false), Ok(()));
    assert!(view.semantic.get().is_some());
    for id in 1..=3 {
        assert!(view.diagnostics(FileId(id)).unwrap().is_empty());
    }
}
