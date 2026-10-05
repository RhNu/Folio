use super::*;

#[path = "../tests/common/support.rs"]
pub(super) mod support;

fn source(path: &str, text: &str) -> ProjectSource {
    ProjectSource {
        package_key: "package".into(),
        canonical_path: PathBuf::from(path),
        display_path: path.into(),
        script_candidate: Path::new(path)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        dialect: PapyrusDialect::Skyrim,
        text: Arc::from(text),
    }
}

#[test]
fn stable_file_identity_and_header_feedback_use_shared_analysis() {
    let mut project = ProjectAnalysis::new();
    let first = project
        .sync_sources([source("Sky.psc", "Scriptname Sky\nFunction F() Native\n")])
        .unwrap();
    let file = first.file_for("package", Path::new("Sky.psc")).unwrap();
    assert!(first.issues.is_empty());
    let second = project
        .sync_sources([source("Sky.psc", "Scriptname Other\nFunction F() Native\n")])
        .unwrap();
    assert_eq!(second.file_for("package", Path::new("Sky.psc")), Some(file));
    assert_eq!(second.analysis.revision(file), Some(Revision(2)));
    assert_eq!(
        second.issues,
        vec![SourceIssue::NameMismatch {
            file,
            candidate: "Sky".into(),
            declared: "Other".into()
        }]
    );
    assert_eq!(first.analysis.revision(file), Some(Revision(1)));
    let empty = project.sync_sources([]).unwrap();
    assert!(empty.analysis.parse(file).is_none());
    let readded = project
        .sync_sources([source("Sky.psc", "Scriptname Sky\n")])
        .unwrap();
    assert_eq!(
        readded.file_for("package", Path::new("Sky.psc")),
        Some(file)
    );
    assert_eq!(readded.analysis.revision(file), Some(Revision(4)));
}

#[test]
fn duplicate_project_source_keeps_previous_generation() {
    let mut project = ProjectAnalysis::new();
    let first = project
        .sync_sources([source("One.psc", "Scriptname One\n")])
        .unwrap();
    let failure = project.sync_sources([
        source("Two.psc", "Scriptname Two\n"),
        source("Two.psc", "Scriptname Two\n"),
    ]);
    assert!(matches!(
        failure,
        Err(ProjectionError::DuplicateSource { .. })
    ));
    let same = project
        .sync_sources([source("One.psc", "Scriptname One\n")])
        .unwrap();
    assert_eq!(same.analysis.generation(), first.analysis.generation());
}

#[test]
fn cached_projection_keeps_source_edits_and_language_policy_current() {
    let mut loaded = support::fixture("Scriptname Root\n", "Root");
    support::add_declarations(&mut loaded, "api", support::declarations());
    let mut metadata = support::resolve(&loaded);
    let mut service = ProjectAnalysis::new();
    let first = service.sync_project(&loaded, &metadata).unwrap();
    let file = first.analysis.file_ids().next().unwrap();
    loaded.source_inputs[0].text = Arc::from("Scriptname Other\n");
    loaded.root.manifest.user_flags.push("Custom".into());
    metadata.fill_missing_arguments = true;
    let edited = service.sync_project(&loaded, &metadata).unwrap();
    assert!(
        matches!(edited.issues.as_slice(), [SourceIssue::NameMismatch { declared, .. }] if declared == "Other")
    );
    assert_eq!(edited.analysis.revision(file), Some(Revision(2)));
    assert!(edited.analysis.fill_missing_arguments());
    assert_eq!(
        edited.analysis.user_flags(),
        &[folio_profiles::UserFlag::from("Custom")]
    );
    assert_eq!(first.analysis.text(file), Some("Scriptname Root\n"));
}

#[test]
fn projection_refreshes_replaced_declaration_content_and_root_selection() {
    let mut loaded = support::fixture("Scriptname Root\n", "Root");
    support::add_declarations(&mut loaded, "api", support::declarations());
    let metadata = support::resolve(&loaded);
    let mut service = ProjectAnalysis::new();
    let first = service.sync_project(&loaded, &metadata).unwrap();
    // Documentation can change without changing semantic provider identity.
    let bundle = Arc::make_mut(loaded.declaration_bundles.get_mut("dependency:0").unwrap());
    bundle
        .scripts
        .iter_mut()
        .find(|script| script.name == "Actor")
        .unwrap()
        .documentation = Some("Updated API".into());
    let updated = service.sync_project(&loaded, &metadata).unwrap();
    let actor = updated
        .analysis
        .external_declarations()
        .iter()
        .flat_map(|bundle| &bundle.scripts)
        .find(|script| script.name == "Actor")
        .unwrap();
    assert_eq!(actor.documentation.as_deref(), Some("Updated API"));
    assert!(
        first
            .analysis
            .external_declarations()
            .iter()
            .flat_map(|bundle| &bundle.scripts)
            .find(|script| script.name == "Actor")
            .unwrap()
            .documentation
            .is_none()
    );
    loaded.root.source_files[0].script_candidate = "Actor".into();
    loaded.source_inputs[0].script_candidate = "Actor".into();
    loaded.source_inputs[0].text = Arc::from("Scriptname Actor\n");
    let metadata = support::resolve(&loaded);
    let selected = service.sync_project(&loaded, &metadata).unwrap();
    assert!(
        selected
            .analysis
            .external_declarations()
            .iter()
            .flat_map(|bundle| &bundle.scripts)
            .all(|script| script.name != "Actor")
    );
    assert!(selected.diagnostics().is_empty());
}

#[test]
fn cached_projection_rejects_missing_selected_source_without_removing_previous_file() {
    let mut loaded = support::fixture("Scriptname Root\n", "Root");
    let metadata = support::resolve(&loaded);
    let mut service = ProjectAnalysis::new();
    let first = service.sync_project(&loaded, &metadata).unwrap();
    loaded.source_inputs.clear();
    assert!(matches!(
        service.sync_project(&loaded, &metadata),
        Err(ProjectionError::MissingSelectedProvider)
    ));
    assert_eq!(
        service.analysis.view().generation(),
        first.analysis.generation()
    );
}

#[test]
fn cached_projection_refreshes_changed_dependency_mapping_with_unchanged_metadata_and_bundles() {
    for change_identity in [false, true] {
        let mut loaded = support::fixture("Scriptname Root\n", "Root");
        support::add_declarations(&mut loaded, "api", support::declarations());
        let metadata = support::resolve(&loaded);
        let mut service = ProjectAnalysis::new();
        let first = service.sync_project(&loaded, &metadata).unwrap();
        assert!(first.analysis.external_script("Actor").is_some());
        if change_identity {
            let SourceId::Dependency { digest, .. } = &mut loaded.dependencies[0].source_id else {
                panic!("fixture dependency must have a declaration identity");
            };
            *digest = "changed-identity".into();
        } else {
            loaded.dependencies[0].source_key = "unmapped-dependency".into();
        }
        // A fresh projection cannot associate the stale provider selection with this mapping.
        let fresh = selected_inputs(&loaded, &metadata).unwrap();
        assert!(fresh.declarations.is_empty());
        let changed = service.sync_project(&loaded, &metadata).unwrap();
        assert!(changed.analysis.external_declarations().is_empty());
        assert!(first.analysis.external_script("Actor").is_some());
    }
}
