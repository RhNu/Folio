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
    InvalidUserFlags(String),
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
            Self::InvalidUserFlags(reason) => write!(f, "invalid user flags: {reason}"),
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
    folio_profiles::resolve_user_flags(&metadata.user_flags, target.max_user_flag_bit)
        .map_err(PlanError::InvalidUserFlags)?;
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
mod tests;
