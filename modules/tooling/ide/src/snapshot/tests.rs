use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::tests::{file, project};

#[test]
fn cancellation_during_cold_reference_build_does_not_publish_partial_counts() {
    let first =
        "Scriptname First\nInt Property Count Auto\nFunction Use()\n Count = 1\nEndFunction\n";
    let second = "Scriptname Second\nFirst Property Owner Auto\nFunction Use()\n Owner.Count = 2\nEndFunction\n";
    let view = project(&[("First", first), ("Second", second)]);
    let count = Symbol::Member {
        script: "first".into(),
        name: "COUNT".into(),
    };
    let checks = Arc::new(AtomicUsize::new(0));
    let cancelled = view.with_cancellation(Arc::new(move || {
        checks.fetch_add(1, Ordering::Relaxed) >= 6
    }));
    assert!(crate::references_of(&cancelled, &count, true).is_empty());
    assert_eq!(crate::references_of(&view, &count, true).len(), 3);
    assert_eq!(crate::references_of(&view.clone(), &count, false).len(), 2);
}

#[test]
fn each_new_snapshot_counts_only_its_own_inputs() {
    let old =
        "Scriptname Example\nInt Property Count Auto\nFunction Use()\n Count = 1\nEndFunction\n";
    let edited = "Scriptname Example\nInt Property Count Auto\nFunction Use()\n Count = Count + 1\nEndFunction\n";
    let before = project(&[("Example", old)]);
    let after = project(&[("Example", edited)]);
    let count = Symbol::Member {
        script: "Example".into(),
        name: "Count".into(),
    };
    assert_eq!(crate::references_of(&before, &count, false).len(), 1);
    assert_eq!(crate::references_of(&after, &count, false).len(), 2);
    assert_eq!(crate::references_of(&before, &count, false).len(), 1);
}

#[test]
fn warmed_indices_keep_sibling_local_scope_identity() {
    let text = "Scriptname Example\nFunction Use(Bool condition)\n If condition\n  Int value = 1\n  value = 2\n Else\n  Int value = 3\n  value = 4\n EndIf\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let first = crate::references(&view, file, text.find("value").unwrap(), true);
    let second = crate::references(&view, file, text.rfind("value").unwrap(), true);
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert!(first.iter().all(|span| !second.contains(span)));
    assert_eq!(
        crate::document_highlights(&view, file, text.find("value").unwrap()),
        first
    );
    assert_eq!(
        crate::document_highlights(&view, file, text.rfind("value").unwrap()),
        second
    );
    assert_eq!(
        crate::references(&view.clone(), file, text.find("value").unwrap(), true),
        first
    );
}

#[test]
fn cancelled_hierarchy_build_can_retry_and_preserves_transitive_overrides() {
    let base = "Scriptname Base\nFunction Work()\nEndFunction\n";
    let child = "Scriptname Child Extends Base\nFunction work()\nEndFunction\n";
    let leaf =
        "Scriptname Leaf Extends Child\nState Busy\n Function WORK()\n EndFunction\nEndState\n";
    let view = project(&[("Base", base), ("Child", child), ("Leaf", leaf)]);
    let checks = Arc::new(AtomicUsize::new(0));
    let cancelled = view.with_cancellation(Arc::new(move || {
        checks.fetch_add(1, Ordering::Relaxed) >= 2
    }));
    assert!(crate::implementation_symbols(&cancelled, &Symbol::Script("BASE".into())).is_empty());
    assert_eq!(
        crate::implementation_symbols(&view, &Symbol::Script("base".into())).len(),
        2
    );
    let target = Symbol::Member {
        script: "bAsE".into(),
        name: "wOrK".into(),
    };
    let actual = crate::implementation_symbols(&view, &target);
    assert_eq!(actual.len(), 2);
    assert!(actual.iter().any(|symbol| matches!(symbol, Symbol::StateMember { script, state, .. } if script == "Leaf" && state == "Busy")));
    assert_eq!(
        crate::implementation_symbols(&view.clone(), &target),
        actual
    );
}

#[test]
fn hierarchy_uses_the_selected_whole_script_and_root_source_precedence() {
    use folio_format_declarations::{DeclarationBundle, Member, MemberData, Origin, Script};
    let base = "Scriptname Base\nFunction Work()\nEndFunction\n";
    let root_child = "Scriptname RootChild Extends Base\n";
    let root = project(&[("Base", base), ("RootChild", root_child)]);
    let api = |name: &str, parent: Option<&str>, callable: bool| Script {
        name: name.into(),
        documentation: None,
        parent: parent.map(str::to_owned),
        is_native: false,
        flags: Vec::new(),
        imports: Vec::new(),
        members: if callable {
            vec![Member {
                name: "work".into(),
                documentation: None,
                flags: Vec::new(),
                data: MemberData::Function {
                    return_type: None,
                    global: false,
                    native: true,
                    parameters: Vec::new(),
                },
            }]
        } else {
            Vec::new()
        },
        states: Vec::new(),
        source: None,
    };
    let mut host = folio_analysis::AnalysisHost::new();
    // The analysis API receives already selected project inputs; its first
    // indexed duplicate supplies the authoritative external unit.
    host.set_external_declarations(vec![DeclarationBundle {
        format: folio_format_declarations::FORMAT.into(),
        schema: folio_format_declarations::SCHEMA_VERSION,
        profile: folio_format_declarations::PROFILE.into(),
        origin: Origin {
            source: "Synthetic".into(),
            input_digest: None,
        },
        scripts: vec![
            api("Child", Some("Base"), true),
            api("CHILD", None, false),
            api("RootChild", None, true),
        ],
    }]);
    for file in root.analysis.file_ids() {
        host.upsert(
            file,
            folio_source::Revision(1),
            root.analysis.text(file).unwrap().into(),
            root.analysis.dialect(file).unwrap(),
        )
        .unwrap();
    }
    let view = IdeSnapshot::from(folio_build::ProjectAnalysisView {
        analysis: host.view(),
        sources: root.sources.clone(),
        issues: Vec::new(),
    });
    assert_eq!(
        crate::implementation_symbols(&view, &Symbol::Script("BASE".into())),
        vec![
            Symbol::Script("Child".into()),
            Symbol::Script("RootChild".into())
        ]
    );
    assert_eq!(
        crate::implementation_symbols(
            &view,
            &Symbol::Member {
                script: "Base".into(),
                name: "Work".into()
            }
        ),
        vec![Symbol::Member {
            script: "Child".into(),
            name: "work".into()
        }]
    );
}
