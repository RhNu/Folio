use super::*;
use crate::tests::{file, project};

#[test]
fn declaration_and_case_insensitive_uses_share_identity() {
    let text = "Scriptname Example\nInt Property Count Auto\nFunction Use()\n Count = count + 1\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    let target = text.find("Count").unwrap();
    let refs = references(&view, file, target, true);
    assert_eq!(refs.len(), 3);
    assert_eq!(references(&view, file, target, false).len(), 2);
    assert!(matches!(
        symbol_at(&view, file, target).unwrap().symbol,
        Symbol::Member { .. }
    ));
}

#[test]
fn same_named_locals_in_different_bodies_do_not_mix() {
    let text = "Scriptname Example\nFunction A()\n Int value = 1\n value = 2\nEndFunction\nFunction B()\n Int value = 3\n value = 4\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    assert_eq!(
        references(&view, file, text.find("value").unwrap(), true).len(),
        2
    );
}

#[test]
fn types_and_inheritance_are_semantic_script_references() {
    let base = "Scriptname Base\nFunction Work()\nEndFunction\n";
    let child =
        "Scriptname Child Extends Base\nBase Property Owner Auto\nFunction Work()\nEndFunction\n";
    let view = project(&[("Base", base), ("Child", child)]);
    let base_file = file(&view, "Base");
    assert_eq!(
        references(&view, base_file, base.find("Base").unwrap(), false).len(),
        2
    );
    assert_eq!(
        implementations(&view, base_file, base.find("Base").unwrap()).len(),
        1
    );
    assert_eq!(
        implementations(&view, base_file, base.find("Work").unwrap()).len(),
        1
    );
}

#[test]
fn parameter_references_include_named_argument_labels() {
    let text = "Scriptname Example\nFunction Work(Int amount)\n Int result = amount\nEndFunction\nFunction Use()\n Work(amount=1)\nEndFunction\n";
    let view = project(&[("Example", text)]);
    let file = file(&view, "Example");
    assert_eq!(
        references(&view, file, text.find("amount").unwrap(), true).len(),
        3
    );
}

#[test]
fn script_occurrences_preserve_identifier_ranges_and_skip_unknown_types() {
    let target = "Scriptname Target\n";
    let text = "Scriptname Example Extends Target\nImport tArGeT\nTarget[] Property Owners Auto\nMissing Property Unknown Auto\nFunction Use(Target value)\n Target local = value\nEndFunction\n";
    let view = project(&[("Target", target), ("Example", text)]);
    let file = file(&view, "Example");
    let target_file = crate::tests::file(&view, "Target");
    let spans = references(&view, target_file, target.find("Target").unwrap(), false);
    assert_eq!(spans.len(), 5);
    assert!(spans.iter().all(|span| span.file == file
        && text[span.range.start..span.range.end].eq_ignore_ascii_case("Target")));
    for span in spans {
        assert_eq!(
            crate::source_declaration(&view, file, span.range.start),
            definition_of(&view, &Symbol::Script("target".into()))
        );
    }
    assert!(symbol_at(&view, file, text.find("Missing").unwrap()).is_none());
    assert!(symbol_at(&view, file, text.len()).is_none());
}

#[test]
fn selected_external_descendants_and_state_implementations_are_included() {
    use folio_format_declarations::{DeclarationBundle, Member, MemberData, Origin, Script, State};
    let text = "Scriptname Base\nFunction Work()\nEndFunction\nInt Property Count Auto\nFunction Utility() Global\nEndFunction\n";
    let mut view = project(&[("Base", text)]);
    let member = |name: &str, global| Member {
        name: name.into(),
        documentation: None,
        flags: Vec::new(),
        data: MemberData::Function {
            return_type: None,
            global,
            native: true,
            parameters: Vec::new(),
        },
    };
    let external = Script {
        name: "ExternalChild".into(),
        documentation: None,
        parent: Some("Base".into()),
        is_native: false,
        flags: Vec::new(),
        imports: Vec::new(),
        members: vec![
            member("Work", false),
            member("Count", false),
            member("Utility", false),
        ],
        states: vec![State {
            name: "Busy".into(),
            documentation: None,
            auto: false,
            members: vec![member("Work", false)],
        }],
        source: None,
    };
    let mut host = folio_analysis::AnalysisHost::new();
    host.set_external_declarations(vec![DeclarationBundle {
        format: folio_format_declarations::FORMAT.into(),
        schema: folio_format_declarations::SCHEMA_VERSION,
        profile: folio_format_declarations::PROFILE.into(),
        origin: Origin {
            source: "Synthetic".into(),
            input_digest: None,
        },
        scripts: vec![external],
    }]);
    for file in view.analysis.file_ids() {
        host.upsert(
            file,
            folio_source::Revision(1),
            view.analysis.text(file).unwrap().into(),
            view.analysis.dialect(file).unwrap(),
        )
        .unwrap();
    }
    view = IdeSnapshot::from(folio_build::ProjectAnalysisView {
        analysis: host.view(),
        sources: view.sources.clone(),
        issues: Vec::new(),
    });
    assert_eq!(
        implementation_symbols(&view, &Symbol::Script("Base".into())),
        vec![Symbol::Script("ExternalChild".into())]
    );
    assert_eq!(
        implementation_symbols(
            &view,
            &Symbol::Member {
                script: "Base".into(),
                name: "Work".into()
            }
        )
        .len(),
        2
    );
    for name in ["Count", "Utility"] {
        assert!(
            implementation_symbols(
                &view,
                &Symbol::Member {
                    script: "Base".into(),
                    name: name.into()
                }
            )
            .is_empty()
        );
    }
}
