use super::*;
use folio_project_model::{
    DeclarationLocation, LoadedCarrier, LoadedPackage, LoadedSdk, Manifest, Metadata, PackageId,
    ScriptProvider, ScriptSelection, SelectionReason, SourceFile, SourceId,
};
use folio_project_resolve::io::LoadedSourceInput;

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
fn loaded_text_and_dialect_are_projected_without_another_reader() {
    let manifest = Manifest {
        source: "folio.toml".into(),
        fields: BTreeMap::new(),
        name: "sample".into(),
        version: "0.1.0".into(),
        source_path: folio_project_model::LocatedString {
            value: "Source/Scripts".into(),
            span: folio_project_model::SourceSpan {
                source: "fixture".into(),
                start: 0,
                end: 0,
            },
        },
        output_path: folio_project_model::LocatedString {
            value: "Scripts".into(),
            span: folio_project_model::SourceSpan {
                source: "fixture".into(),
                start: 0,
                end: 0,
            },
        },
        language: "papyrus".into(),
        dialect: "skyrim".into(),
        extensions: vec!["psc".into()],
        user_flags: vec![],
        fill_missing_arguments: false,
        lint_rules: Default::default(),
        target: "skyrim-se".into(),
        profile: "dev".into(),
        debug_info: true,
        experimental_pex_dependencies: false,
        emit: vec![],
        dependencies: vec![],
    };
    let project = LoadedProject {
        root_key: "manifest".into(),
        packages: vec![LoadedPackage {
            source_key: "manifest".into(),
            source_id: SourceId::Project,
            carrier: LoadedCarrier::Manifest(manifest),
            source_files: vec![SourceFile {
                path: "src/Sky.psc".into(),
                display_path: "src/Sky.psc".into(),
                script_candidate: "Sky".into(),
            }],
            links: vec![],
        }],
        declaration_bundles: BTreeMap::new(),
        source_inputs: vec![LoadedSourceInput {
            package_key: "manifest".into(),
            canonical_path: PathBuf::from("project/src/Sky.psc"),
            display_path: "src/Sky.psc".into(),
            script_candidate: "Sky".into(),
            text: Arc::from("Scriptname Sky\n"),
        }],
    };
    let sources = sources_from_loaded(&project).unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].text.as_ref(), "Scriptname Sky\n");
    assert_eq!(sources[0].dialect, PapyrusDialect::Skyrim);
    let mut service = ProjectAnalysis::new();
    let metadata =
        folio_project_resolve::graph::resolve(&project.root_key, &project.packages).unwrap();
    let view = service.sync_project(&project, &metadata).unwrap();
    assert!(view.issues.is_empty());
    assert_eq!(view.analysis.text(FileId(0)), Some("Scriptname Sky\n"));
}

#[test]
fn selected_inputs_exclude_shadowed_sdk_script_and_keep_flags() {
    let manifest = Manifest {
        source: "folio.toml".into(),
        fields: BTreeMap::new(),
        name: "sample".into(),
        version: "0.1.0".into(),
        source_path: folio_project_model::LocatedString {
            value: "Source/Scripts".into(),
            span: folio_project_model::SourceSpan {
                source: "fixture".into(),
                start: 0,
                end: 0,
            },
        },
        output_path: folio_project_model::LocatedString {
            value: "Scripts".into(),
            span: folio_project_model::SourceSpan {
                source: "fixture".into(),
                start: 0,
                end: 0,
            },
        },
        language: "papyrus".into(),
        dialect: "skyrim".into(),
        extensions: vec!["psc".into()],
        user_flags: vec!["Custom".into()],
        fill_missing_arguments: false,
        lint_rules: Default::default(),
        target: "skyrim-se".into(),
        profile: "dev".into(),
        debug_info: true,
        experimental_pex_dependencies: false,
        emit: vec!["pex".into()],
        dependencies: vec![],
    };
    let root_id = PackageId {
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        source: SourceId::Project,
    };
    let sdk_source = SourceId::DeclarationSdk {
        path: "sdk.json".into(),
        digest: "test".into(),
    };
    let sdk_id = PackageId {
        name: "sdk".into(),
        version: "1".into(),
        source: sdk_source.clone(),
    };
    let sdk = folio_format_declarations::decode(br#"{"schema":1,"package":{"name":"sdk","version":"1","source":"self-authored","generator":"test"},"compatibility":{"target":"skyrim-se","abi":"papyrus-skyrim"},"naming":{"language":"papyrus","case_sensitive":false},"scripts":[{"name":"Sky"},{"name":"Actor"}]}"#).unwrap();
    let root_provider = ScriptProvider {
        script: "Sky".into(),
        package: root_id.clone(),
        definition: None,
        declaration: None,
        source_path: Some("src/Sky.psc".into()),
    };
    let sdk_provider = |name: &str, script_index| ScriptProvider {
        script: name.into(),
        package: sdk_id.clone(),
        definition: None,
        declaration: Some(DeclarationLocation {
            carrier_path: "sdk.json".into(),
            script_index,
            source_path: None,
            line: None,
            column: None,
        }),
        source_path: None,
    };
    let metadata = Metadata {
        schema: 4,
        root: root_id,
        target: "skyrim-se".into(),
        profile: "dev".into(),
        fill_missing_arguments: false,
        debug_info: true,
        user_flags: vec!["Custom".into()],
        source: "Source/Scripts".into(),
        output: "Scripts".into(),
        packages: vec![],
        dependencies: vec![],
        scripts: vec![
            ScriptSelection {
                script: "sky".into(),
                selected: root_provider.clone(),
                providers: vec![sdk_provider("Sky", 0), root_provider],
                reason: SelectionReason::DependencyOrder,
            },
            ScriptSelection {
                script: "actor".into(),
                selected: sdk_provider("Actor", 1),
                providers: vec![sdk_provider("Actor", 1)],
                reason: SelectionReason::SoleProvider,
            },
        ],
        external_requirements: vec![],
    };
    let project = LoadedProject {
        root_key: "root".into(),
        packages: vec![
            LoadedPackage {
                source_key: "root".into(),
                source_id: SourceId::Project,
                carrier: LoadedCarrier::Manifest(manifest.clone()),
                source_files: vec![],
                links: vec![],
            },
            LoadedPackage {
                source_key: "sdk".into(),
                source_id: sdk_source,
                carrier: LoadedCarrier::Declarations {
                    kind: folio_project_model::DependencyKind::Sdk,
                    sdk: LoadedSdk {
                        name: "sdk".into(),
                        version: "1".into(),
                        target: "skyrim-se".into(),
                        abi: "papyrus-skyrim".into(),
                        scripts: vec![],
                    },
                },
                source_files: vec![],
                links: vec![],
            },
        ],
        declaration_bundles: BTreeMap::from([("sdk".into(), sdk)]),
        source_inputs: vec![LoadedSourceInput {
            package_key: "root".into(),
            canonical_path: PathBuf::from("src/Sky.psc"),
            display_path: "src/Sky.psc".into(),
            script_candidate: "Sky".into(),
            text: Arc::from("ScriptName Sky\n"),
        }],
    };
    let selected = selected_inputs(&project, &metadata).unwrap();
    assert_eq!(selected.sources.len(), 1);
    assert_eq!(selected.declarations.len(), 1);
    assert_eq!(selected.declarations[0].scripts.len(), 1);
    assert_eq!(selected.declarations[0].scripts[0].name, "Actor");
    assert_eq!(selected.user_flags, vec!["Custom"]);
}
