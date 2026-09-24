use super::*;
use folio_project_model::{
    DependencySpec, LoadedLink, LoadedSdk, LocatedString, Manifest, SourceFile,
};

fn spot(source: &str) -> SourceSpan {
    SourceSpan {
        source: source.into(),
        start: 0,
        end: 1,
    }
}

fn value(source: &str, text: &str) -> LocatedString {
    LocatedString {
        value: text.into(),
        span: spot(source),
    }
}

fn dependency(source: &str, name: &str, kind: DependencyKind) -> DependencySpec {
    DependencySpec {
        name: value(source, name),
        kind,

        path: value(source, name),
    }
}

fn manifest(name: &str, dependencies: Vec<DependencySpec>) -> Manifest {
    Manifest {
        source: format!("{name}/folio.toml"),
        fields: BTreeMap::new(),
        name: name.into(),
        version: "1.0".into(),
        source_path: value(name, "Source/Scripts"),
        output_path: value(name, "Scripts"),
        language: "papyrus".into(),
        dialect: "skyrim".into(),
        extensions: vec!["psc".into()],
        user_flags: vec![],
        fill_missing_arguments: false,
        lint_rules: Default::default(),
        target: "skyrim-se".into(),
        profile: "dev".into(),
        debug_info: true,
        emit: vec!["pex".into()],
        dependencies,
    }
}

fn package(
    key: &str,
    dependencies: Vec<(DependencySpec, &str)>,
    scripts: &[&str],
) -> LoadedPackage {
    let links = dependencies
        .iter()
        .enumerate()
        .map(|(index, (_, target))| LoadedLink {
            dependency_index: index,
            source_key: (*target).into(),
        })
        .collect();
    let specs = dependencies.into_iter().map(|(spec, _)| spec).collect();
    LoadedPackage {
        source_key: key.into(),
        source_id: if key == "root" {
            SourceId::Project
        } else {
            SourceId::Local {
                path: format!("../{key}/folio.toml"),
            }
        },
        manifest: Some(manifest(key, specs)),
        sdk: None,
        source_files: scripts
            .iter()
            .map(|script| SourceFile {
                path: format!("src/{script}.psc"),
                display_path: format!("src/{script}.psc"),
                script_candidate: (*script).into(),
            })
            .collect(),
        links,
    }
}

fn sdk(target: &str, digest: &str) -> LoadedPackage {
    LoadedPackage {
        source_key: "sdk".into(),
        source_id: SourceId::DeclarationSdk {
            path: "sdk.json".into(),
            digest: digest.into(),
        },
        manifest: None,
        sdk: Some(LoadedSdk {
            name: "sdk".into(),
            version: "1".into(),
            target: target.into(),
            abi: "papyrus-skyrim".into(),
            scripts: Vec::new(),
        }),
        source_files: Vec::new(),
        links: Vec::new(),
    }
}

fn edge<'a>(owner: &str, name: &'a str, kind: DependencyKind) -> (DependencySpec, &'a str) {
    (dependency(owner, name, kind), name)
}

#[test]
fn later_dependency_and_root_source_win() {
    let base = vec![
        package(
            "root",
            vec![
                edge("root", "a", DependencyKind::Package),
                edge("root", "b", DependencyKind::Package),
            ],
            &["Shared"],
        ),
        package("a", vec![], &["shared"]),
        package("b", vec![], &["SHARED"]),
    ];
    let result = resolve("root", &base).unwrap();
    assert_eq!(result.scripts[0].selected.package.name, "root");
    assert_eq!(
        result.scripts[0]
            .providers
            .iter()
            .map(|p| p.package.name.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b", "root"]
    );
    let mut without_root = base;
    without_root[0].source_files.clear();
    assert_eq!(
        resolve("root", &without_root).unwrap().scripts[0]
            .selected
            .package
            .name,
        "b"
    );
    without_root[0]
        .manifest
        .as_mut()
        .unwrap()
        .dependencies
        .swap(0, 1);
    without_root[0].links.swap(0, 1);
    for link in &mut without_root[0].links {
        link.dependency_index = 1 - link.dependency_index;
    }
    assert_eq!(
        resolve("root", &without_root).unwrap().scripts[0]
            .selected
            .package
            .name,
        "a"
    );
}

#[test]
fn transitive_dependencies_follow_the_last_declared_path() {
    let packages = vec![
        package(
            "root",
            vec![
                edge("root", "a", DependencyKind::Package),
                edge("root", "b", DependencyKind::Package),
            ],
            &[],
        ),
        package(
            "a",
            vec![edge("a", "shared", DependencyKind::Package)],
            &["Actor"],
        ),
        package("b", vec![edge("b", "shared", DependencyKind::Package)], &[]),
        package("shared", vec![], &["actor"]),
    ];
    let result = resolve("root", &packages).unwrap();
    assert_eq!(result.scripts[0].selected.package.name, "shared");
    let mut reversed_inputs = packages;
    reversed_inputs.reverse();
    assert_eq!(
        resolve("root", &reversed_inputs).unwrap().scripts,
        result.scripts
    );
}

#[test]
fn later_builtin_declaration_shadows_earlier_one() {
    let sdk = |name: &str| LoadedPackage {
        source_key: name.into(),
        source_id: SourceId::DeclarationSdk {
            path: format!("{name}.json"),
            digest: name.into(),
        },
        manifest: None,
        sdk: Some(LoadedSdk {
            name: name.into(),
            version: "1".into(),
            target: "skyrim-se".into(),
            abi: "papyrus-skyrim".into(),
            scripts: vec![folio_project_model::DeclaredScript {
                name: "Actor".into(),
                location: folio_project_model::DeclarationLocation {
                    carrier_path: format!("{name}.json"),
                    script_index: 0,
                    source_path: None,
                    line: None,
                    column: None,
                },
            }],
        }),
        source_files: Vec::new(),
        links: Vec::new(),
    };
    let packages = vec![
        package(
            "root",
            vec![
                edge("root", "ck", DependencyKind::Builtin),
                edge("root", "skse", DependencyKind::Builtin),
            ],
            &[],
        ),
        sdk("ck"),
        sdk("skse"),
    ];
    let result = resolve("root", &packages).unwrap();
    assert_eq!(result.scripts[0].selected.package.name, "skse");
    assert_eq!(result.scripts[0].providers.len(), 2);
}

#[test]
fn rejects_a_duplicate_within_one_package_and_dependency_cycles() {
    let duplicated = vec![package("root", vec![], &["Actor", "actor"])];
    assert!(matches!(
        resolve("root", &duplicated).unwrap_err().kind,
        ResolveErrorKind::ScriptConflict(_)
    ));
    let cycle = vec![
        package(
            "root",
            vec![edge("root", "a", DependencyKind::Package)],
            &[],
        ),
        package("a", vec![edge("a", "root", DependencyKind::Package)], &[]),
    ];
    assert!(matches!(
        resolve("root", &cycle).unwrap_err().kind,
        ResolveErrorKind::DependencyCycle(_)
    ));
}

#[test]
fn rejects_sdk_target_mismatch() {
    let mismatch = vec![
        package("root", vec![edge("root", "sdk", DependencyKind::Sdk)], &[]),
        sdk("other-target", "abc"),
    ];
    assert!(matches!(
        resolve("root", &mismatch).unwrap_err().kind,
        ResolveErrorKind::TargetMismatch { .. }
    ));
    let mut wrong_abi = mismatch;
    wrong_abi[1].sdk.as_mut().unwrap().target = "skyrim-se".into();
    wrong_abi[1].sdk.as_mut().unwrap().abi = "another-abi".into();
    assert!(matches!(
        resolve("root", &wrong_abi).unwrap_err().kind,
        ResolveErrorKind::AbiMismatch { .. }
    ));
}

#[test]
fn declared_name_must_match_loaded_package_identity() {
    let mut inputs = vec![
        package(
            "root",
            vec![edge("root", "expected", DependencyKind::Package)],
            &[],
        ),
        package("actual", vec![], &[]),
    ];
    inputs[0].links[0].source_key = "actual".into();
    assert!(matches!(
        resolve("root", &inputs).unwrap_err().kind,
        ResolveErrorKind::DependencyIdentityMismatch { .. }
    ));
}
