//! Serial project build orchestration. Semantic results are the sole compiler input.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use folio_diagnostics::{Diagnostic, Severity};
use folio_project_model::Metadata;
use folio_project_resolve::{LoadedProject, io::load_and_resolve_with_home};
use tracing::{debug, info, warn};

use crate::{
    ProjectAnalysisView,
    fingerprint::{command_fingerprint, unit_fingerprint},
    output::{
        self, ArtifactRecord, CacheDecision, ExternalRecord, OutputError, SuccessRecord, UnitRecord,
    },
    plan::{self, PlanError, ProjectPlan},
};

#[derive(Debug)]
pub enum BuildError {
    Plan(PlanError),
    Diagnostics(Vec<Diagnostic>),
    MissingSemantic(String),
    InvalidSelection(String),
    InputsChanged(String),
    Output(OutputError),
    Cancelled,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(cause) => write!(f, "build plan: {cause}"),
            Self::Diagnostics(items) => {
                write!(f, "build rejected with {} diagnostic(s)", items.len())
            }
            Self::MissingSemantic(script) => {
                write!(f, "no semantic model for selected script {script}")
            }
            Self::InvalidSelection(reason) => write!(f, "incomplete build selection: {reason}"),
            Self::InputsChanged(reason) => write!(f, "build inputs changed: {reason}"),
            Self::Output(cause) => write!(f, "{cause}"),
            Self::Cancelled => write!(f, "build cancelled"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<OutputError> for BuildError {
    fn from(value: OutputError) -> Self {
        Self::Output(value)
    }
}

/// Target plan retains validated MIR for emission and records decisions in the key.
pub struct TargetPlan {
    pub project: ProjectPlan,
    pub scripts: BTreeMap<String, folio_mir::Script>,
    pub decisions: Vec<String>,
    pub user_flags: Vec<(String, u8)>,
}

/// Analyze all selected providers, then perform target legality checks before I/O.
pub fn target_plan(
    project: &LoadedProject,
    metadata: &Metadata,
    view: &ProjectAnalysisView,
) -> Result<TargetPlan, BuildError> {
    target_plan_with_cancel(project, metadata, view, None)
}

fn target_plan_with_cancel(
    project: &LoadedProject,
    metadata: &Metadata,
    view: &ProjectAnalysisView,
    cancelled: Option<&AtomicBool>,
) -> Result<TargetPlan, BuildError> {
    debug!(
        target = %metadata.target,
        fill_missing_arguments = metadata.fill_missing_arguments,
        debug_info = metadata.debug_info,
        "selected compilation policies"
    );
    let plan = plan::project_plan(project, metadata).map_err(BuildError::Plan)?;
    let target_profile = folio_profiles::TargetId::new(&metadata.target)
        .and_then(|id| folio_profiles::TargetProfile::implemented(&id))
        .ok_or_else(|| BuildError::Plan(PlanError::UnsupportedTarget(metadata.target.clone())))?;
    let diagnostics = view.diagnostics();
    if diagnostics
        .iter()
        .any(|item| item.severity == Severity::Error)
    {
        return Err(BuildError::Diagnostics(diagnostics));
    }
    let mut scripts = BTreeMap::new();
    let mut decisions = Vec::new();
    let mut lowering_errors = Vec::new();
    let resolved_flags =
        folio_profiles::resolve_user_flags(&metadata.user_flags, target_profile.max_user_flag_bit)
            .map_err(|reason| BuildError::Plan(PlanError::InvalidUserFlags(reason)))?;
    let user_flags = resolved_flags
        .iter()
        .map(|flag| (flag.name.clone(), flag.bit.expect("resolved flag bit")))
        .collect::<Vec<_>>();
    for unit in &plan.units {
        for script in &unit.scripts {
            if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                return Err(BuildError::Cancelled);
            }
            let input = plan::source_input(project, &script.package.source, &script.source_path)
                .ok_or_else(|| BuildError::MissingSemantic(script.script.clone()))?;
            let file = view
                .file_for(&input.package_key, &input.canonical_path)
                .ok_or_else(|| BuildError::MissingSemantic(script.script.clone()))?;
            let hir = view
                .analysis
                .hir(file)
                .ok_or_else(|| BuildError::MissingSemantic(script.script.clone()))?;
            match folio_lowering::lower_script(&hir, target_profile, &resolved_flags) {
                Ok(mir) => {
                    for decision in &mir.decisions {
                        decisions.push(format!(
                            "{}:{}:{:?}:{}:{}",
                            script.script,
                            decision.feature,
                            decision.outcome,
                            decision.source.range.start,
                            decision.source.range.end
                        ));
                    }
                    scripts.insert(script.script.clone(), mir);
                }
                Err(mut issues) => lowering_errors.append(&mut issues),
            }
        }
    }
    if !lowering_errors.is_empty() {
        return Err(BuildError::Diagnostics(lowering_errors));
    }
    decisions.push(format!("user-flags:{user_flags:?}"));
    Ok(TargetPlan {
        project: plan,
        scripts,
        decisions,
        user_flags,
    })
}

/// Check analysis and target feasibility without encoding or writing artifacts.
pub fn check_target(
    project: &LoadedProject,
    metadata: &Metadata,
    view: &ProjectAnalysisView,
) -> Result<(), BuildError> {
    target_plan(project, metadata, view).map(|_| ())
}

/// A completed command result and cache explanations.
pub struct BuildOutcome {
    pub success: SuccessRecord,
    pub cache_decisions: Vec<(String, CacheDecision)>,
    pub unchanged: bool,
}

fn snapshot_still_current(
    project_root: &Path,
    project: &LoadedProject,
    metadata: &Metadata,
    compiler_identity: &str,
    decisions: &[String],
    command_key: &str,
) -> Result<bool, BuildError> {
    folio_project_resolve::io::verify_snapshots(&project.input_snapshots)
        .map_err(|cause| BuildError::InputsChanged(cause.to_string()))?;
    let (current, current_metadata) =
        load_and_resolve_with_home(project_root, None, &project.folio_home)
            .map_err(|cause| BuildError::InputsChanged(cause.to_string()))?;
    Ok(current.input_snapshots == project.input_snapshots
        && current_metadata.output == metadata.output
        && command_fingerprint(&current, &current_metadata, compiler_identity, decisions)
            == command_key)
}

/// A command may publish only when every planned unit and artifact has a staged owner.
pub fn prepare_success(
    plan: &ProjectPlan,
    units: Vec<UnitRecord>,
    cancelled: bool,
) -> Result<SuccessRecord, BuildError> {
    if cancelled {
        return Err(BuildError::Cancelled);
    }
    if units.len() != plan.units.len() {
        return Err(BuildError::InvalidSelection(
            "build unit count differs from plan".into(),
        ));
    }
    let mut seen = BTreeSet::new();
    for (planned, completed) in plan.units.iter().zip(&units) {
        if !seen.insert(&completed.unit_id)
            || planned.id != completed.unit_id
            || planned.package.name != completed.package
            || planned.package.version.as_deref() != Some(completed.package_version.as_str())
            || serde_json::to_value(&planned.package.source).expect("source identity serializes")
                != completed.package_source
            || planned.target != completed.target
            || planned.profile != completed.profile
            || completed.fingerprint.is_empty()
            || !output::safe_generation(&completed.generation, &completed.unit_id)
        {
            return Err(BuildError::InvalidSelection(format!(
                "unit {} differs from plan",
                planned.id
            )));
        }
        if planned.scripts.len() != completed.artifacts.len() {
            return Err(BuildError::InvalidSelection(format!(
                "artifact count differs for {}",
                planned.id
            )));
        }
        for (script, artifact) in planned.scripts.iter().zip(&completed.artifacts) {
            if artifact.kind != "pex"
                || artifact.format != "pex-3.2-skyrim-se"
                || artifact.path != script.artifact_path
                || artifact.script != script.script
                || artifact.source_package != script.package.name
                || artifact.source_path != script.source_path
                || artifact.digest.is_empty()
            {
                return Err(BuildError::InvalidSelection(format!(
                    "artifact {} differs from plan",
                    script.artifact_path
                )));
            }
        }
    }
    Ok(SuccessRecord {
        schema: 2,
        output: plan.output.clone(),
        units,
        external_requirements: plan
            .external_requirements
            .iter()
            .map(|requirement| ExternalRecord {
                package: requirement.package.name.clone(),
                package_version: requirement.package.version.clone(),
                package_source: serde_json::to_value(&requirement.package.source)
                    .expect("source identity serializes"),
                target: requirement.target.clone(),
                abi: requirement.abi.clone(),
                reason: requirement.reason.clone(),
            })
            .collect(),
    })
}

/// Execute each target task in stable order and publish only the complete command.
#[tracing::instrument(name = "project.build", skip_all, fields(package = %metadata.root.name, target = %metadata.target, revision = view.analysis.generation()))]
pub fn build_project(
    project: &LoadedProject,
    metadata: &Metadata,
    view: &ProjectAnalysisView,
    project_root: &Path,
    compiler_identity: &str,
    cancelled: &AtomicBool,
) -> Result<BuildOutcome, BuildError> {
    if compiler_identity.is_empty() {
        return Err(BuildError::InvalidSelection(
            "compiler identity is required".into(),
        ));
    }
    let target = target_plan_with_cancel(project, metadata, view, Some(cancelled))?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(BuildError::Cancelled);
    }
    let command_key = command_fingerprint(project, metadata, compiler_identity, &target.decisions);
    let (build_root, cache_root) = output::managed_paths(project_root)?;
    let _lock = output::BuildLock::acquire(&build_root)?;
    let prior = output::read_index(&build_root)?;
    if let Some(previous) = &prior.result
        && previous.output == metadata.output
        && previous.units.len() == target.project.units.len()
        && previous
            .units
            .iter()
            .zip(&target.project.units)
            .all(|(record, planned)| {
                record.unit_id == planned.id
                    && record.fingerprint == unit_fingerprint(&command_key, planned)
                    && record
                        .artifacts
                        .iter()
                        .map(|artifact| &artifact.path)
                        .eq(planned.scripts.iter().map(|script| &script.artifact_path))
            })
        && output::verify_published(project_root, previous)?
    {
        if !snapshot_still_current(
            project_root,
            project,
            metadata,
            compiler_identity,
            &target.decisions,
            &command_key,
        )? {
            return Err(BuildError::InputsChanged(
                "workspace or dependency snapshot changed".into(),
            ));
        }
        // Carrier relocation can preserve the API and PEX while changing provenance.
        let success = prepare_success(
            &target.project,
            previous.units.clone(),
            cancelled.load(Ordering::Relaxed),
        )?;
        if success != *previous {
            output::publish(&build_root, &success)?;
            info!(output = %metadata.output, "refreshed source provenance for reused artifacts");
        }
        info!(output = %metadata.output, "workspace snapshot and published output unchanged");
        return Ok(BuildOutcome {
            success,
            cache_decisions: target
                .project
                .units
                .iter()
                .map(|unit| (unit.id.clone(), CacheDecision::Hit))
                .collect(),
            unchanged: true,
        });
    }
    let mut units = Vec::new();
    let mut cache_decisions = Vec::new();
    for unit in &target.project.units {
        if cancelled.load(Ordering::Relaxed) {
            return Err(BuildError::Cancelled);
        }
        let fingerprint = unit_fingerprint(&command_key, unit);
        let expected = unit
            .scripts
            .iter()
            .map(|script| script.artifact_path.clone())
            .collect::<Vec<_>>();
        let (mut cache_decision, mut cached) =
            match output::read_cache(&cache_root, &fingerprint, &expected) {
                Ok(value) => value,
                Err(cause) => {
                    warn!(package = %unit.package.name, %cause, "cache read failed; rebuilding");
                    (CacheDecision::Invalid("cache read failure"), Vec::new())
                }
            };
        if matches!(cache_decision, CacheDecision::Hit)
            && !cached
                .iter()
                .zip(&unit.scripts)
                .all(|((artifact, _), script)| {
                    artifact.kind == "pex"
                        && artifact.format == "pex-3.2-skyrim-se"
                        && artifact.script == script.script
                        && artifact.source_package == script.package.name
                        && artifact.source_path == script.source_path
                })
        {
            cache_decision = CacheDecision::Invalid("artifact provenance");
            cached.clear();
        }
        debug!(package = %unit.package.name, target = %unit.target, fingerprint, decision = ?cache_decision, "cache decision");
        if matches!(cache_decision, CacheDecision::Invalid(_))
            && let Err(cause) = output::discard_cache(&cache_root, &fingerprint)
        {
            warn!(package = %unit.package.name, %cause, "cache discard failed; continuing without cache");
        }
        let generated = if matches!(cache_decision, CacheDecision::Hit) {
            cached
        } else {
            let mut generated = Vec::new();
            for script in &unit.scripts {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(BuildError::Cancelled);
                }
                let mir = target
                    .scripts
                    .get(&script.script)
                    .ok_or_else(|| BuildError::MissingSemantic(script.script.clone()))?;
                let input =
                    plan::source_input(project, &script.package.source, &script.source_path)
                        .ok_or_else(|| BuildError::MissingSemantic(script.script.clone()))?;
                let file = view
                    .file_for(&input.package_key, &input.canonical_path)
                    .ok_or_else(|| BuildError::MissingSemantic(script.script.clone()))?;
                let options = folio_backend_pex::EmissionOptions {
                    source_file_name: script.source_path.clone(),
                    user_name: String::new(),
                    computer_name: String::new(),
                    compilation_time: 0,
                    debug_info: metadata.debug_info,
                    source_text: BTreeMap::from([(file, input.text.to_string())]),
                    user_flags: target.user_flags.clone(),
                };
                let bytes =
                    folio_backend_pex::emit(mir, &options).map_err(BuildError::Diagnostics)?;
                let artifact = ArtifactRecord {
                    kind: "pex".into(),
                    path: script.artifact_path.clone(),
                    digest: blake3::hash(&bytes).to_hex().to_string(),
                    format: "pex-3.2-skyrim-se".into(),
                    script: script.script.clone(),
                    source_package: script.package.name.clone(),
                    source_path: script.source_path.clone(),
                };
                generated.push((artifact, bytes));
            }
            if let Err(cause) = output::write_cache(&cache_root, &fingerprint, &generated) {
                warn!(package = %unit.package.name, %cause, "discardable cache write failed");
            }
            generated
        };
        let (generation, dir) = output::create_generation(&build_root, &unit.id)?;
        let mut artifacts = Vec::new();
        for (artifact, bytes) in generated {
            let digest = output::write_artifact(&dir, &artifact.path, &bytes)?;
            if digest != artifact.digest {
                return Err(BuildError::Output(OutputError {
                    operation: "verify staged artifact",
                    path: dir.join(&artifact.path),
                    cause: "digest mismatch".into(),
                }));
            }
            artifacts.push(artifact);
        }
        let record = UnitRecord {
            unit_id: unit.id.clone(),
            package: unit.package.name.clone(),
            package_version: unit
                .package
                .version
                .clone()
                .expect("build units belong to the versioned root"),
            package_source: serde_json::to_value(&unit.package.source)
                .expect("source identity serializes"),
            target: unit.target.clone(),
            profile: unit.profile.clone(),
            fingerprint: fingerprint.clone(),
            generation,
            artifacts,
        };
        output::write_generation_manifest(&dir, &record)?;
        info!(package = %unit.package.name, target = %unit.target, artifacts = record.artifacts.len(), generation = %record.generation, "staged build unit");
        units.push(record);
        cache_decisions.push((unit.id.clone(), cache_decision));
    }
    let success = prepare_success(&target.project, units, cancelled.load(Ordering::Relaxed))?;
    if !snapshot_still_current(
        project_root,
        project,
        metadata,
        compiler_identity,
        &target.decisions,
        &command_key,
    )? {
        return Err(BuildError::InputsChanged(
            "workspace or dependency snapshot changed before publication".into(),
        ));
    }
    output::publish_project(project_root, &build_root, prior.result.as_ref(), &success)?;
    Ok(BuildOutcome {
        success,
        cache_decisions,
        unchanged: false,
    })
}

#[cfg(test)]
mod tests;
