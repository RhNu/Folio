//! Minimal in-memory project inputs shared by pure build tests.
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use folio_format_declarations::DeclarationBundle;
use folio_project_model::{
    DeclarationLocation, DeclaredScript, DependencyKind, DependencySpec, LoadedDependency,
    LoadedRoot, LocatedString, SourceEncoding, SourceFile, SourceId, SourceSpan,
};
use folio_project_resolve::{LoadedProject, io::LoadedSourceInput};

pub(crate) fn fixture(text: &str, name: &str) -> LoadedProject {
    let manifest = folio_project_resolve::manifest::parse(
        "folio.toml",
        r#"
[package]
name = "app"
version = "1"
[paths]
source = "src"
[languages.papyrus]
dialect = "skyrim"
extensions = ["psc"]
[build]
target = "skyrim-se"
profile = "dev"
emit = ["pex"]
"#,
    )
    .expect("test fixture operation succeeds");
    let path = format!("src/{name}.psc");
    LoadedProject {
        folio_home: folio_project_resolve::io::FolioHome {
            path: PathBuf::from("C:/user/.folio"),
        },
        root_key: "root".into(),
        root: LoadedRoot {
            source_key: "root".into(),
            source_id: SourceId::Project,
            manifest,
            source_files: vec![SourceFile {
                path: path.clone(),
                display_path: path.clone(),
                script_candidate: name.into(),
            }],
        },
        dependencies: vec![],
        declaration_bundles: BTreeMap::new(),
        source_inputs: vec![LoadedSourceInput {
            package_key: "root".into(),
            canonical_path: PathBuf::from(format!("C:/workspace/{path}")),
            display_path: path,
            script_candidate: name.into(),
            text: Arc::from(text),
        }],
        input_snapshots: vec![],
        watch_plan: folio_project_resolve::io::WatchPlan::default(),
    }
}

pub(crate) fn add_declarations(project: &mut LoadedProject, name: &str, bundle: DeclarationBundle) {
    let index = project.dependencies.len();
    let path = format!("{name}.json");
    let key = format!("dependency:{index}");
    let span = SourceSpan {
        source: "folio.toml".into(),
        start: 0,
        end: 1,
    };
    let source_id = SourceId::Dependency {
        index,
        kind: DependencyKind::Decl,
        path: path.clone(),
        digest: folio_format_declarations::semantic_digest(&bundle),
    };
    project.root.manifest.dependencies.push(DependencySpec {
        name: LocatedString {
            value: name.into(),
            span: span.clone(),
        },
        kind: DependencyKind::Decl,
        path: LocatedString {
            value: path.clone(),
            span: span.clone(),
        },
        encoding: SourceEncoding::Utf8,
    });
    project.dependencies.push(LoadedDependency {
        source_key: key.clone(),
        source_id,
        kind: DependencyKind::Decl,
        name: name.into(),
        declared_path: path.clone(),
        canonical_path: PathBuf::from(format!("E:/repo/{path}")),
        declaration: span,
        profile: bundle.profile.clone(),
        scripts: bundle
            .scripts
            .iter()
            .map(|script| DeclaredScript {
                name: script.name.clone(),
                location: DeclarationLocation {
                    carrier_path: path.clone(),
                    script_name: script.name.clone(),
                    source_path: None,
                    line: None,
                    column: None,
                },
            })
            .collect(),
    });
    project.declaration_bundles.insert(key, Arc::new(bundle));
}

pub(crate) fn declarations() -> DeclarationBundle {
    folio_format_declarations::decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"self-authored"},"scripts":[{"name":"Sky"},{"name":"Actor"}]}"#).expect("test fixture operation succeeds")
}

pub(crate) fn resolve(project: &LoadedProject) -> folio_project_model::Metadata {
    folio_project_resolve::graph::resolve(&project.root, &project.dependencies)
        .expect("test fixture operation succeeds")
}
