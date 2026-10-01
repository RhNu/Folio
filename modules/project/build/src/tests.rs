use super::*;
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
fn selected_inputs_preserve_root_body_and_select_dependency_api() {
    let mut loaded = test_support::fixture("Scriptname Sky\nFunction Run() Native\n", "Sky");
    loaded.root.manifest.user_flags.push("Custom".into());
    test_support::add_declarations(&mut loaded, "api", test_support::declarations());
    let metadata = test_support::resolve(&loaded);
    let selected = selected_inputs(&loaded, &metadata).unwrap();
    assert_eq!(selected.sources.len(), 1);
    assert_eq!(
        selected.sources[0].text.as_ref(),
        "Scriptname Sky\nFunction Run() Native\n"
    );
    assert_eq!(selected.declarations.len(), 1);
    assert_eq!(selected.declarations[0].scripts.len(), 1);
    assert_eq!(selected.declarations[0].scripts[0].name, "Actor");
    assert_eq!(selected.user_flags, vec!["Custom"]);
    let mut service = ProjectAnalysis::new();
    let view = service.sync_project(&loaded, &metadata).unwrap();
    assert!(view.diagnostics().is_empty());
}

#[test]
fn repeated_carrier_occurrences_keep_the_last_alias() {
    let mut loaded = test_support::fixture("Scriptname Root\n", "Root");
    test_support::add_declarations(&mut loaded, "first", test_support::declarations());
    test_support::add_declarations(&mut loaded, "last", test_support::declarations());
    let metadata = test_support::resolve(&loaded);
    let selected = selected_inputs(&loaded, &metadata).unwrap();
    assert_eq!(selected.declarations.len(), 1);
    let actor = metadata
        .scripts
        .iter()
        .find(|item| item.script == "actor")
        .unwrap();
    assert_eq!(actor.selected.package.name, "last");
    assert_eq!(actor.providers.len(), 2);
}

#[test]
fn fingerprint_ignores_provenance_host_paths_and_declaration_order() {
    let mut loaded = test_support::fixture("Scriptname Root\n", "Root");
    test_support::add_declarations(&mut loaded, "api", test_support::declarations());
    let metadata = test_support::resolve(&loaded);
    let before = fingerprint::command_fingerprint(&loaded, &metadata, "compiler", &[]);
    loaded.source_inputs[0].canonical_path = PathBuf::from("F:/moved/src/Root.psc");
    loaded.dependencies[0].canonical_path = PathBuf::from("C:/elsewhere/api.fdecl");
    let bundle = loaded.declaration_bundles.get_mut("dependency:0").unwrap();
    bundle.scripts.reverse();
    bundle.origin.source = "another machine".into();
    let selected = selected_inputs(&loaded, &metadata).unwrap();
    assert_eq!(selected.declarations[0].scripts.len(), 2);
    assert_eq!(
        before,
        fingerprint::command_fingerprint(&loaded, &metadata, "compiler", &[])
    );
    loaded
        .declaration_bundles
        .get_mut("dependency:0")
        .unwrap()
        .scripts[0]
        .parent = Some("NewParent".into());
    assert_ne!(
        before,
        fingerprint::command_fingerprint(&loaded, &metadata, "compiler", &[])
    );
}
