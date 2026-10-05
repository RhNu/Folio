//! In-memory project resolution, projection, planning, and fingerprint contracts.
#[path = "common/support.rs"]
mod common;
use std::path::PathBuf;

use common as test_support;
use folio_build::{
    ProjectAnalysis, fingerprint,
    plan::{PlanError, project_plan},
    selected_inputs,
};
use folio_project_model::Metadata;

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
    assert_eq!(
        selected.user_flags,
        vec![folio_profiles::UserFlag::from("Custom")]
    );
    let mut service = ProjectAnalysis::new();
    let view = service.sync_project(&loaded, &metadata).unwrap();
    assert_eq!(view.diagnostics(), [] as [folio_diagnostics::Diagnostic; 0]);
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
    let bundle =
        std::sync::Arc::make_mut(loaded.declaration_bundles.get_mut("dependency:0").unwrap());
    bundle.scripts.reverse();
    bundle.origin.source = "another machine".into();
    let selected = selected_inputs(&loaded, &metadata).unwrap();
    assert_eq!(selected.declarations[0].scripts.len(), 2);
    assert_eq!(
        before,
        fingerprint::command_fingerprint(&loaded, &metadata, "compiler", &[])
    );
    std::sync::Arc::make_mut(loaded.declaration_bundles.get_mut("dependency:0").unwrap()).scripts
        [0]
    .parent = Some("NewParent".into());
    assert_ne!(
        before,
        fingerprint::command_fingerprint(&loaded, &metadata, "compiler", &[])
    );
}

#[test]
fn plan_selects_root_provenance_and_rejects_unsafe_output() {
    let loaded = test_support::fixture("Scriptname Sky", "Sky");
    let metadata = test_support::resolve(&loaded);
    let plan = project_plan(&loaded, &metadata).unwrap();
    assert_eq!(plan.units.len(), 1);
    assert_eq!(plan.units[0].scripts[0].artifact_path, "sky.pex");
    assert_eq!(plan.units[0].scripts[0].source_path, "src/Sky.psc");
    let loaded = test_support::fixture("Scriptname CON", "CON");
    assert!(matches!(
        project_plan(&loaded, &test_support::resolve(&loaded)),
        Err(PlanError::InvalidArtifact(_))
    ));
}

#[test]
fn declaration_dependencies_supply_api_and_root_owns_build_output() {
    let mut loaded = test_support::fixture("Scriptname Sky", "Sky");
    test_support::add_declarations(&mut loaded, "api", test_support::declarations());
    let plan = project_plan(&loaded, &test_support::resolve(&loaded)).unwrap();
    assert_eq!(plan.units.len(), 1);
    assert_eq!(plan.units[0].scripts.len(), 1);
    assert_eq!(plan.units[0].scripts[0].script, "sky");
}

#[test]
fn fingerprint_tracks_compilation_settings_and_contents() {
    let mut loaded = test_support::fixture("Scriptname Sky", "Sky");
    let metadata = test_support::resolve(&loaded);
    let key = crate::fingerprint::command_fingerprint(&loaded, &metadata, "compiler-a", &[]);
    let mut changed = metadata.clone();
    changed.output = "Build/Scripts".into();
    assert_eq!(
        key,
        crate::fingerprint::command_fingerprint(&loaded, &changed, "compiler-a", &[])
    );
    assert_eq!(
        project_plan(&loaded, &changed).unwrap().output,
        "Build/Scripts"
    );
    for mutate in [
        |metadata: &mut Metadata| metadata.profile = "release".into(),
        |metadata: &mut Metadata| metadata.user_flags.push("Custom".into()),
        |metadata: &mut Metadata| metadata.fill_missing_arguments = true,
        |metadata: &mut Metadata| metadata.debug_info = false,
    ] {
        let mut changed = metadata.clone();
        mutate(&mut changed);
        assert_ne!(
            key,
            crate::fingerprint::command_fingerprint(&loaded, &changed, "compiler-a", &[])
        );
    }
    assert_ne!(
        key,
        crate::fingerprint::command_fingerprint(&loaded, &metadata, "compiler-b", &[])
    );
    assert_ne!(
        key,
        crate::fingerprint::command_fingerprint(
            &loaded,
            &metadata,
            "compiler-a",
            &["new-rule".into()]
        )
    );
    loaded.source_inputs[0].text = std::sync::Arc::from("Scriptname Sky\n");
    assert_ne!(
        key,
        crate::fingerprint::command_fingerprint(&loaded, &metadata, "compiler-a", &[])
    );
}

#[test]
fn fingerprint_tracks_dependency_precedence() {
    let mut loaded = test_support::fixture("Scriptname Sky", "Sky");
    test_support::add_declarations(&mut loaded, "first", test_support::declarations());
    test_support::add_declarations(&mut loaded, "last", test_support::declarations());
    let before = crate::fingerprint::command_fingerprint(
        &loaded,
        &test_support::resolve(&loaded),
        "compiler",
        &[],
    );
    let mut reversed = test_support::fixture("Scriptname Sky", "Sky");
    test_support::add_declarations(&mut reversed, "last", test_support::declarations());
    test_support::add_declarations(&mut reversed, "first", test_support::declarations());
    assert_ne!(
        before,
        crate::fingerprint::command_fingerprint(
            &reversed,
            &test_support::resolve(&reversed),
            "compiler",
            &[]
        )
    );
}
