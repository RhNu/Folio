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
    let root = &project.root.manifest;
    for kind in &root.emit {
        if kind != "pex" {
            return Err(PlanError::UnsupportedArtifact(kind.clone()));
        }
    }
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
        if package.source != SourceId::Project
            || !project
                .root
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
    {
        let package = &metadata.root;
        let mut unit_scripts = scripts.remove(package).unwrap_or_default();
        unit_scripts.sort_by(|left, right| left.script.cmp(&right.script));
        let identity = serde_json::to_vec(&(package, &metadata.target, &metadata.profile))
            .expect("build identity is serializable");
        let id = blake3::hash(&identity).to_hex().to_string();
        units.push(BuildUnit {
            id,
            package: package.clone(),
            target: metadata.target.clone(),
            profile: metadata.profile.clone(),
            scripts: unit_scripts,
        });
    }
    units.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(ProjectPlan {
        schema: 3,
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
    if *source != SourceId::Project {
        return None;
    }
    project
        .source_inputs
        .iter()
        .find(|input| input.package_key == project.root_key && input.display_path == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

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

    #[test]
    fn target_flag_capacity_reserves_builtin_bits() {
        let loaded = test_support::fixture("Scriptname Sky", "Sky");
        let mut metadata = test_support::resolve(&loaded);
        metadata.user_flags = (0..30).map(|index| format!("Flag{index}")).collect();
        assert!(project_plan(&loaded, &metadata).is_ok());
        metadata.user_flags.push("OneMore".into());
        assert_eq!(
            project_plan(&loaded, &metadata),
            Err(PlanError::TooManyUserFlags {
                count: 31,
                maximum: 30
            })
        );
    }
}
