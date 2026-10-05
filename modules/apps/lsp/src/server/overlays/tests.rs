use folio_project_model::{LoadedDependency, LoadedRoot};
use folio_project_resolve::io::{FolioHome, WatchPlan};

use super::*;
use crate::server::presentation;

fn project() -> LoadedProject {
    let manifest=folio_project_resolve::manifest::parse("folio.toml",
        "[package]\nname='demo'\nversion='0.1.0'\n[languages.papyrus]\ndialect='skyrim'\nextensions=['psc']\n[build]\ntarget='skyrim-se'\nprofile='dev'\nemit=['pex']\n[[dependencies]]\nname='api'\nkind='psc'\npath='../api'\nencoding='windows1252'\n").unwrap();
    let source = "ScriptName External\nString Function Run() Native\n{Original documentation.}\n";
    let bundle = generate(
        GenerationOptions {
            source: "dependency/0",
        },
        &[SourceInput {
            path: "External.psc",
            text: source,
        }],
    )
    .unwrap();
    let specification = &manifest.dependencies[0];
    let dependency = LoadedDependency {
        source_key: "dependency:0".into(),
        source_id: SourceId::Dependency {
            index: 0,
            kind: DependencyKind::Psc,
            path: "../api".into(),
            digest: semantic_digest(&bundle),
        },
        kind: DependencyKind::Psc,
        name: "api".into(),
        declared_path: "../api".into(),
        canonical_path: PathBuf::from("D:/synthetic/api"),
        declaration: specification.path.span.clone(),
        profile: bundle.profile.clone(),
        scripts: bundle
            .scripts
            .iter()
            .map(|script| DeclaredScript {
                name: script.name.clone(),
                location: DeclarationLocation {
                    carrier_path: "../api".into(),
                    script_name: script.name.to_ascii_lowercase(),
                    source_path: Some("External.psc".into()),
                    line: Some(1),
                    column: Some(1),
                },
            })
            .collect(),
    };
    LoadedProject {
        root_key: "root".into(),
        root: LoadedRoot {
            source_key: "root".into(),
            source_id: SourceId::Project,
            manifest,
            source_files: Vec::new(),
        },
        dependencies: vec![dependency],
        declaration_bundles: BTreeMap::from([("dependency:0".into(), Arc::new(bundle))]),
        source_inputs: Vec::new(),
        input_snapshots: vec![InputSnapshot::File {
            path: PathBuf::from("D:/synthetic/api/External.psc"),
            bytes: Arc::from(source.as_bytes()),
        }],
        watch_plan: WatchPlan::default(),
        folio_home: FolioHome {
            path: PathBuf::from("D:/synthetic/home"),
        },
    }
}

#[test]
fn dependency_buffers_update_docs_and_navigation_without_compiling_bodies() {
    let mut loaded = project();
    let original_id = loaded.dependencies[0].source_id.clone();
    let path = PathBuf::from("D:/synthetic/api/External.psc");
    let text = Arc::<str>::from(
        "; unsaved\nScriptName External\nString Function Run() Native\n{New café documentation.}\n",
    );
    let buffers = BTreeMap::from([(path.clone(), Arc::clone(&text))]);
    assert!(apply_dependency_overlays(&mut loaded, &buffers).unwrap());
    assert_eq!(loaded.dependencies[0].source_id, original_id);
    assert_eq!(loaded.dependencies[0].scripts[0].location.line, Some(2));
    assert_eq!(
        loaded.declaration_bundles["dependency:0"].scripts[0].members[0]
            .documentation
            .as_deref(),
        Some("New café documentation.")
    );
    assert!(loaded.source_inputs.is_empty());
    let metadata = folio_project_resolve::resolve(&loaded.root, &loaded.dependencies).unwrap();
    let (selected_path, selected_text) =
        presentation::external_source(&metadata, &loaded, "External", &buffers).unwrap();
    assert_eq!(selected_path, path);
    assert_eq!(selected_text, text.as_ref());
}

#[test]
fn dependency_signature_edits_change_provider_identity_and_reject_wrong_filenames() {
    let mut loaded = project();
    let original_id = loaded.dependencies[0].source_id.clone();
    let path = PathBuf::from("D:/synthetic/api/External.psc");
    assert!(
        apply_dependency_overlays(
            &mut loaded,
            &BTreeMap::from([(
                path.clone(),
                Arc::from("ScriptName External\nInt Function Run(Int count = 3) Native\n")
            )])
        )
        .unwrap()
    );
    assert_ne!(loaded.dependencies[0].source_id, original_id);
    assert!(
        apply_dependency_overlays(
            &mut loaded,
            &BTreeMap::from([(path, Arc::from("ScriptName Wrong\n"))])
        )
        .is_err()
    );
}
