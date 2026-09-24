//! Deterministic project planning before semantic and target-specific work.

use std::collections::{BTreeMap, BTreeSet};

use folio_profiles::{TargetId, TargetProfile};
use folio_project_model::{Metadata, PackageId, SourceId};
use folio_project_resolve::LoadedProject;
use serde::Serialize;

/// A package, target and profile have one logical build identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BuildUnit {
    pub id: String,
    pub package: PackageId,
    pub target: String,
    pub profile: String,
    pub scripts: Vec<PlannedScript>,
}

/// A selected source provider, with its portable package-relative provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlannedScript {
    pub script: String,
    pub source_path: String,
    pub package: PackageId,
    pub artifact_path: String,
}

/// The resolved package graph projected into serial build tasks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectPlan {
    pub schema: u32,
    pub root: PackageId,
    pub output: String,
    pub units: Vec<BuildUnit>,
    pub external_requirements: Vec<folio_project_model::ExternalRequirement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    UnsupportedTarget(String),
    UnsupportedArtifact(String),
    TooManyUserFlags { count: usize, maximum: usize },
    MissingPackage(String),
    MissingSource { package: String, path: String },
    DuplicateArtifact(String),
    InvalidArtifact(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedTarget(target) => write!(f, "unsupported build target {target}"),
            Self::UnsupportedArtifact(kind) => write!(f, "unsupported artifact kind {kind}"),
            Self::TooManyUserFlags { count, maximum } => {
                write!(f, "{count} user flags exceed target limit of {maximum}")
            }
            Self::MissingPackage(package) => write!(f, "missing loaded package {package}"),
            Self::MissingSource { package, path } => {
                write!(f, "selected source {path} is missing from {package}")
            }
            Self::DuplicateArtifact(path) => write!(f, "duplicate artifact path {path}"),
            Self::InvalidArtifact(path) => {
                write!(f, "artifact name cannot be represented safely: {path}")
            }
        }
    }
}

impl std::error::Error for PlanError {}

/// Plan only resolver-selected scripts owned by the current workspace.
pub fn project_plan(
    project: &LoadedProject,
    metadata: &Metadata,
) -> Result<ProjectPlan, PlanError> {
    let target = TargetId::new(&metadata.target)
        .and_then(|id| TargetProfile::implemented(&id))
        .ok_or_else(|| PlanError::UnsupportedTarget(metadata.target.clone()))?;
    // Skyrim reserves bits 0 and 1 for Hidden and Conditional.
    let maximum_user_flags = usize::from(target.max_user_flag_bit).saturating_sub(1);
    if metadata.user_flags.len() > maximum_user_flags {
        return Err(PlanError::TooManyUserFlags {
            count: metadata.user_flags.len(),
            maximum: maximum_user_flags,
        });
    }
    let root = project
        .packages
        .iter()
        .find(|package| package.source_key == project.root_key)
        .and_then(|package| package.manifest.as_ref())
        .ok_or_else(|| PlanError::MissingPackage(project.root_key.clone()))?;
    for kind in &root.emit {
        if kind != "pex" {
            return Err(PlanError::UnsupportedArtifact(kind.clone()));
        }
    }
    let loaded = project
        .packages
        .iter()
        .map(|package| (&package.source_id, package))
        .collect::<BTreeMap<_, _>>();
    let mut scripts = BTreeMap::<PackageId, Vec<PlannedScript>>::new();
    let mut artifacts = BTreeSet::new();
    for selection in &metadata.scripts {
        let Some(path) = &selection.selected.source_path else {
            continue;
        };
        let package = &selection.selected.package;
        if *package != metadata.root {
            continue;
        }
        let carrier = loaded
            .get(&package.source)
            .ok_or_else(|| PlanError::MissingPackage(package.name.clone()))?;
        if !carrier
            .source_files
            .iter()
            .any(|source| source.display_path == *path)
        {
            return Err(PlanError::MissingSource {
                package: package.name.clone(),
                path: path.clone(),
            });
        }
        // Resolver script identities are already normalized and collision checked.
        let artifact_path = format!("{}.pex", selection.script);
        if !super::output::safe_artifact_name(&artifact_path) {
            return Err(PlanError::InvalidArtifact(artifact_path));
        }
        if !artifacts.insert(artifact_path.clone()) {
            return Err(PlanError::DuplicateArtifact(artifact_path));
        }
        scripts
            .entry(package.clone())
            .or_default()
            .push(PlannedScript {
                script: selection.script.clone(),
                source_path: path.clone(),
                package: package.clone(),
                artifact_path,
            });
    }
    let mut units = Vec::new();
    for package in metadata
        .packages
        .iter()
        .filter(|package| package.id == metadata.root)
    {
        let mut unit_scripts = scripts.remove(&package.id).unwrap_or_default();
        unit_scripts.sort_by(|left, right| left.script.cmp(&right.script));
        let identity = serde_json::to_vec(&(&package.id, &metadata.target, &metadata.profile))
            .expect("build identity is serializable");
        let id = blake3::hash(&identity).to_hex().to_string();
        units.push(BuildUnit {
            id,
            package: package.id.clone(),
            target: metadata.target.clone(),
            profile: metadata.profile.clone(),
            scripts: unit_scripts,
        });
    }
    units.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(ProjectPlan {
        schema: 2,
        root: metadata.root.clone(),
        output: metadata.output.clone(),
        units,
        external_requirements: metadata.external_requirements.clone(),
    })
}

/// Match resolver provenance to a loaded input without a host path in the plan.
pub fn source_input<'a>(
    project: &'a LoadedProject,
    source: &SourceId,
    path: &str,
) -> Option<&'a folio_project_resolve::io::LoadedSourceInput> {
    let package = project
        .packages
        .iter()
        .find(|package| &package.source_id == source)?;
    project
        .source_inputs
        .iter()
        .find(|input| input.package_key == package.source_key && input.display_path == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use folio_project_model::{
        LoadedPackage, Manifest, ResolvedPackage, ScriptProvider, ScriptSelection, SelectionReason,
        SourceFile,
    };
    use folio_project_resolve::io::LoadedSourceInput;
    use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

    fn fixture(text: &str, script_name: &str) -> (LoadedProject, Metadata) {
        let id = PackageId {
            name: "app".into(),
            version: "0.1.0".into(),
            source: SourceId::Project,
        };
        let manifest = Manifest {
            source: "folio.toml".into(),
            fields: BTreeMap::new(),
            name: "app".into(),
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
            emit: vec!["pex".into()],
            dependencies: vec![],
        };
        let source = SourceFile {
            path: format!("src/{script_name}.psc"),
            display_path: format!("src/{script_name}.psc"),
            script_candidate: script_name.into(),
        };
        let input = LoadedSourceInput {
            package_key: "root".into(),
            canonical_path: PathBuf::from(format!("C:/workspace/src/{script_name}.psc")),
            display_path: source.display_path.clone(),
            script_candidate: script_name.into(),
            text: Arc::from(text),
        };
        let provider = ScriptProvider {
            script: script_name.into(),
            package: id.clone(),
            definition: None,
            declaration: None,
            source_path: Some(source.display_path.clone()),
        };
        let metadata = Metadata {
            schema: 4,
            root: id.clone(),
            target: "skyrim-se".into(),
            profile: "dev".into(),
            fill_missing_arguments: false,
            debug_info: true,
            user_flags: vec![],
            source: "Source/Scripts".into(),
            output: "Scripts".into(),
            packages: vec![ResolvedPackage {
                id,
                source_root: Some("src".into()),
                source_files: vec![source.path.clone()],
                language: Some("papyrus".into()),
                dialect: Some("skyrim".into()),
            }],
            dependencies: vec![],
            scripts: vec![ScriptSelection {
                script: script_name.to_ascii_lowercase(),
                selected: provider.clone(),
                providers: vec![provider],
                reason: SelectionReason::SoleProvider,
            }],
            external_requirements: vec![],
        };
        let project = LoadedProject {
            root_key: "root".into(),
            packages: vec![LoadedPackage {
                source_key: "root".into(),
                source_id: SourceId::Project,
                manifest: Some(manifest),
                sdk: None,
                source_files: vec![source],
                links: vec![],
            }],
            declaration_bundles: BTreeMap::new(),
            source_inputs: vec![input],
        };
        (project, metadata)
    }

    #[test]
    fn plan_selects_provenance_and_rejects_unsafe_output() {
        let (project, metadata) = fixture("Scriptname Sky", "Sky");
        let plan = project_plan(&project, &metadata).unwrap();
        assert_eq!(plan.units.len(), 1);
        assert_eq!(plan.units[0].scripts[0].artifact_path, "sky.pex");
        assert_eq!(plan.units[0].scripts[0].source_path, "src/Sky.psc");
        let (project, metadata) = fixture("Scriptname CON", "CON");
        assert!(matches!(
            project_plan(&project, &metadata),
            Err(PlanError::InvalidArtifact(_))
        ));
    }

    #[test]
    fn local_dependency_is_visible_without_becoming_build_output() {
        let (mut project, mut metadata) = fixture("Scriptname Sky", "Sky");
        let dependency = PackageId {
            name: "api".into(),
            version: "1".into(),
            source: SourceId::Local {
                path: "../api/folio.toml".into(),
            },
        };
        let file = SourceFile {
            path: "Source/Scripts/External.psc".into(),
            display_path: "Source/Scripts/External.psc".into(),
            script_candidate: "External".into(),
        };
        project.packages.push(LoadedPackage {
            source_key: "api".into(),
            source_id: dependency.source.clone(),
            manifest: None,
            sdk: None,
            source_files: vec![file.clone()],
            links: vec![],
        });
        metadata.packages.push(ResolvedPackage {
            id: dependency.clone(),
            source_root: Some("Source/Scripts".into()),
            source_files: vec![file.path.clone()],
            language: Some("papyrus".into()),
            dialect: Some("skyrim".into()),
        });
        let provider = ScriptProvider {
            script: "External".into(),
            package: dependency,
            definition: None,
            declaration: None,
            source_path: Some(file.display_path),
        };
        metadata.scripts.push(ScriptSelection {
            script: "external".into(),
            selected: provider.clone(),
            providers: vec![provider],
            reason: SelectionReason::SoleProvider,
        });
        let plan = project_plan(&project, &metadata).unwrap();
        assert_eq!(plan.units.len(), 1);
        assert_eq!(plan.units[0].scripts.len(), 1);
        assert_eq!(plan.units[0].scripts[0].script, "sky");
    }

    #[test]
    fn fingerprint_tracks_contents_without_host_path_identity() {
        let (first, metadata) = fixture("Scriptname Sky", "Sky");
        let (mut second, _) = fixture("Scriptname Sky", "Sky");
        second.source_inputs[0].canonical_path = PathBuf::from("D:/other/src/Sky.psc");
        let key = crate::fingerprint::command_fingerprint(&first, &metadata, "compiler-a", &[]);
        let mut relocated = metadata.clone();
        relocated.output = "Build/Scripts".into();
        assert_eq!(
            key,
            crate::fingerprint::command_fingerprint(&first, &relocated, "compiler-a", &[])
        );
        assert_eq!(
            project_plan(&first, &relocated).unwrap().output,
            "Build/Scripts"
        );
        assert_eq!(
            key,
            crate::fingerprint::command_fingerprint(&second, &metadata, "compiler-a", &[])
        );
        second.source_inputs[0].text = Arc::from("Scriptname Sky\n");
        assert_ne!(
            key,
            crate::fingerprint::command_fingerprint(&second, &metadata, "compiler-a", &[])
        );
        assert_ne!(
            key,
            crate::fingerprint::command_fingerprint(&first, &metadata, "compiler-b", &[])
        );
        let mut changed = metadata.clone();
        changed.profile = "release".into();
        assert_ne!(
            key,
            crate::fingerprint::command_fingerprint(&first, &changed, "compiler-a", &[])
        );
        changed = metadata.clone();
        changed.user_flags.push("Special".into());
        assert_ne!(
            key,
            crate::fingerprint::command_fingerprint(&first, &changed, "compiler-a", &[])
        );
        changed = metadata.clone();
        changed.fill_missing_arguments = true;
        assert_ne!(
            key,
            crate::fingerprint::command_fingerprint(&first, &changed, "compiler-a", &[])
        );
        changed = metadata.clone();
        changed.debug_info = false;
        assert_ne!(
            key,
            crate::fingerprint::command_fingerprint(&first, &changed, "compiler-a", &[])
        );
        assert_ne!(
            key,
            crate::fingerprint::command_fingerprint(
                &first,
                &metadata,
                "compiler-a",
                &["short-circuit:conditional-branch".into()]
            )
        );
    }

    #[test]
    fn fingerprint_tracks_dependency_precedence() {
        let (mut project, metadata) = fixture("Scriptname Sky", "Sky");
        let manifest = project.packages[0].manifest.as_mut().unwrap();
        let span = folio_project_model::SourceSpan {
            source: "folio.toml".into(),
            start: 0,
            end: 1,
        };
        let dependency = |name: &str| folio_project_model::DependencySpec {
            name: folio_project_model::LocatedString {
                value: name.into(),
                span: span.clone(),
            },
            kind: folio_project_model::DependencyKind::Builtin,
            path: folio_project_model::LocatedString {
                value: name.into(),
                span: span.clone(),
            },
        };
        manifest.dependencies = vec![dependency("ck"), dependency("skse")];
        let first = crate::fingerprint::command_fingerprint(&project, &metadata, "compiler", &[]);
        project.packages[0]
            .manifest
            .as_mut()
            .unwrap()
            .dependencies
            .reverse();
        let second = crate::fingerprint::command_fingerprint(&project, &metadata, "compiler", &[]);
        assert_ne!(first, second);
    }

    #[test]
    fn target_flag_capacity_reserves_builtin_bits() {
        let (project, mut metadata) = fixture("Scriptname Sky", "Sky");
        metadata.user_flags = (0..30).map(|index| format!("Flag{index}")).collect();
        assert!(project_plan(&project, &metadata).is_ok());
        metadata.user_flags.push("OneMore".into());
        assert_eq!(
            project_plan(&project, &metadata),
            Err(PlanError::TooManyUserFlags {
                count: 31,
                maximum: 30,
            })
        );
    }
}
