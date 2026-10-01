use super::*;
use folio_project_model::{DeclarationLocation, DeclaredScript, DependencyKind, SourceFile};
use std::path::PathBuf;

fn fixture(names: &[&str]) -> (LoadedRoot, Vec<LoadedDependency>) {
    let mut text = "[package]\nname = \"root\"\nversion = \"0.2.0\"\n[languages.papyrus]\ndialect = \"skyrim\"\nextensions = [\"psc\"]\n[build]\ntarget = \"skyrim-se\"\nprofile = \"dev\"\nemit = [\"pex\"]\n".to_owned();
    for name in names {
        text.push_str(&format!(
            "\n[[dependencies]]\nname = \"{name}\"\nkind = \"decl\"\npath = \"api.json\"\n"
        ));
    }
    let manifest = crate::manifest::parse("folio.toml", &text).unwrap();
    let dependencies = manifest
        .dependencies
        .iter()
        .enumerate()
        .map(|(index, specification)| LoadedDependency {
            source_key: format!("dependency:{index}"),
            source_id: SourceId::Dependency {
                index,
                kind: DependencyKind::Decl,
                path: "api.json".into(),
                digest: "same-api".into(),
            },
            kind: DependencyKind::Decl,
            name: specification.name.value.clone(),
            declared_path: "api.json".into(),
            canonical_path: PathBuf::from("/api.json"),
            declaration: specification.path.span.clone(),
            profile: "papyrus-skyrim".into(),
            scripts: vec![DeclaredScript {
                name: "Actor".into(),
                location: DeclarationLocation {
                    carrier_path: "api.json".into(),
                    script_name: "actor".into(),
                    source_path: None,
                    line: None,
                    column: None,
                },
            }],
        })
        .collect();
    (
        LoadedRoot {
            source_key: "root".into(),
            source_id: SourceId::Project,
            manifest,
            source_files: Vec::new(),
        },
        dependencies,
    )
}

#[test]
fn repeated_carrier_occurrences_keep_alias_and_order() {
    let (root, dependencies) = fixture(&["first", "second"]);
    let metadata = resolve(&root, &dependencies).unwrap();
    assert_eq!(metadata.scripts[0].selected.package.name, "second");
    assert_eq!(metadata.scripts[0].providers.len(), 2);
    assert!(
        metadata
            .dependencies
            .iter()
            .all(|edge| edge.to.version.is_none())
    );
    assert_ne!(
        metadata.dependencies[0].to.source,
        metadata.dependencies[1].to.source
    );
}

#[test]
fn root_script_has_highest_precedence() {
    let (mut root, dependencies) = fixture(&["api"]);
    root.source_files.push(SourceFile {
        path: "Actor.psc".into(),
        display_path: "Actor.psc".into(),
        script_candidate: "actor".into(),
    });
    let metadata = resolve(&root, &dependencies).unwrap();
    assert_eq!(
        metadata.scripts[0].selected.package.source,
        SourceId::Project
    );
}

#[test]
fn duplicates_inside_one_provider_are_conflicts() {
    let (root, mut dependencies) = fixture(&["api"]);
    let duplicate = dependencies[0].scripts[0].clone();
    dependencies[0].scripts.push(duplicate);
    assert!(matches!(
        resolve(&root, &dependencies).unwrap_err().kind,
        ResolveErrorKind::ScriptConflict(_)
    ));
}

#[test]
fn incompatible_profile_has_dependency_location() {
    let (root, mut dependencies) = fixture(&["api"]);
    dependencies[0].profile = "other".into();
    let error = resolve(&root, &dependencies).unwrap_err();
    assert!(matches!(
        error.kind,
        ResolveErrorKind::ProfileMismatch { .. }
    ));
    assert!(error.location.is_some());
}
