//! Deterministic whole-script selection over a root and ordered leaf dependencies.

use std::collections::{BTreeMap, BTreeSet};

use folio_project_model::{
    DependencyEdge, ExternalRequirement, LoadedDependency, LoadedRoot, Metadata, PackageId,
    ResolvedPackage, ScriptProvider, ScriptSelection, SelectionReason, SourceId, SourceSpan,
};
use tracing::{debug, info, instrument};

pub const METADATA_SCHEMA: u32 = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveError {
    pub kind: ResolveErrorKind,
    pub location: Option<Box<SourceSpan>>,
    pub related: Box<[SourceSpan]>,
    pub providers: Box<[ScriptProvider]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveErrorKind {
    DuplicateSource(String),
    InvalidRoot,
    InvalidDependency(String),
    DuplicateDependencyAlias(String),
    MissingDependency(String),
    ProfileMismatch {
        dependency: String,
        expected: String,
        actual: String,
    },
    ScriptConflict(String),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.kind)?;
        if let Some(location) = &self.location {
            write!(
                f,
                " at {}:{}..{}",
                location.source, location.start, location.end
            )?;
        }
        for provider in &self.providers {
            write!(
                f,
                "; provider {} ({})",
                provider.package.name,
                provider
                    .source_path
                    .as_deref()
                    .or_else(|| provider
                        .declaration
                        .as_ref()
                        .map(|location| location.carrier_path.as_str()))
                    .unwrap_or("unknown source")
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for ResolveError {}

/// Resolve each dependency occurrence in manifest order, followed by root sources.
#[instrument(name = "dependency.resolve", skip_all, fields(package_id = %root.manifest.name, phase = "resolve"))]
pub fn resolve(
    root: &LoadedRoot,
    dependencies: &[LoadedDependency],
) -> Result<Metadata, ResolveError> {
    if root.source_id != SourceId::Project {
        return Err(error(ResolveErrorKind::InvalidRoot, None));
    }
    let manifest = &root.manifest;
    if dependencies.len() != manifest.dependencies.len() {
        return Err(error(
            ResolveErrorKind::MissingDependency(
                "dependency occurrence count does not match manifest".into(),
            ),
            None,
        ));
    }
    let root_id = PackageId {
        name: manifest.name.clone(),
        version: Some(manifest.version.clone()),
        source: SourceId::Project,
    };
    let mut source_keys = BTreeSet::from([root.source_key.as_str()]);
    let mut aliases = BTreeSet::new();
    let mut providers = BTreeMap::<String, Vec<ScriptProvider>>::new();
    let mut resolved_packages = Vec::new();
    let mut edges = Vec::new();
    let mut requirements = Vec::new();
    let expected_profile = format!("papyrus-{}", manifest.dialect);
    info!(
        dependency_count = dependencies.len(),
        "resolving dependency occurrences"
    );
    for (index, (dependency, specification)) in
        dependencies.iter().zip(&manifest.dependencies).enumerate()
    {
        if !source_keys.insert(dependency.source_key.as_str()) {
            return Err(error(
                ResolveErrorKind::DuplicateSource(dependency.source_key.clone()),
                Some(dependency.declaration.clone()),
            ));
        }
        if !aliases.insert(dependency.name.as_str()) {
            return Err(error(
                ResolveErrorKind::DuplicateDependencyAlias(dependency.name.clone()),
                Some(dependency.declaration.clone()),
            ));
        }
        if dependency.name != specification.name.value
            || dependency.kind != specification.kind
            || dependency.declared_path != specification.path.value
            || !matches!(&dependency.source_id, SourceId::Dependency { index: occurrence, kind, path, .. }
                if *occurrence == index && *kind == dependency.kind && path == &dependency.declared_path)
        {
            return Err(error(
                ResolveErrorKind::InvalidDependency(dependency.name.clone()),
                Some(dependency.declaration.clone()),
            ));
        }
        if dependency.profile != expected_profile {
            return Err(error(
                ResolveErrorKind::ProfileMismatch {
                    dependency: dependency.name.clone(),
                    expected: expected_profile.clone(),
                    actual: dependency.profile.clone(),
                },
                Some(dependency.declaration.clone()),
            ));
        }
        let id = PackageId {
            name: dependency.name.clone(),
            version: None,
            source: dependency.source_id.clone(),
        };
        edges.push(DependencyEdge {
            from: root_id.clone(),
            to: id.clone(),
            kind: dependency.kind,
            declared_path: dependency.declared_path.clone(),
            declaration: dependency.declaration.clone(),
        });
        resolved_packages.push(ResolvedPackage {
            id: id.clone(),
            source_root: None,
            source_files: Vec::new(),
            language: Some(manifest.language.clone()),
            dialect: Some(manifest.dialect.clone()),
        });
        requirements.push(ExternalRequirement {
            package: id.clone(),
            target: manifest.target.clone(),
            abi: dependency.profile.clone(),
            reason: "dependency API requires an external runtime".into(),
        });
        for script in &dependency.scripts {
            providers
                .entry(script.name.to_ascii_lowercase())
                .or_default()
                .push(ScriptProvider {
                    script: script.name.clone(),
                    package: id.clone(),
                    definition: None,
                    declaration: Some(script.location.clone()),
                    source_path: None,
                });
        }
    }
    for source in &root.source_files {
        providers
            .entry(source.script_candidate.to_ascii_lowercase())
            .or_default()
            .push(ScriptProvider {
                script: source.script_candidate.clone(),
                package: root_id.clone(),
                definition: None,
                declaration: None,
                source_path: Some(source.display_path.clone()),
            });
    }
    let mut selections = Vec::new();
    for (script, choices) in providers {
        let mut identities = BTreeSet::new();
        if choices
            .iter()
            .any(|provider| !identities.insert(&provider.package))
        {
            let mut issue = error(ResolveErrorKind::ScriptConflict(script), None);
            issue.providers = choices.into_boxed_slice();
            return Err(issue);
        }
        let selected = choices.last().expect("script has a provider").clone();
        let reason = if choices.len() == 1 {
            SelectionReason::SoleProvider
        } else {
            SelectionReason::DependencyOrder
        };
        debug!(script, selected_dependency = %selected.package.name, provider_count = choices.len(), ?reason, "selected script provider");
        selections.push(ScriptSelection {
            script,
            selected,
            providers: choices,
            reason,
        });
    }
    let mut source_files = root
        .source_files
        .iter()
        .map(|source| source.path.clone())
        .collect::<Vec<_>>();
    source_files.sort();
    resolved_packages.push(ResolvedPackage {
        id: root_id.clone(),
        source_root: Some(manifest.source_path.value.clone()),
        source_files,
        language: Some(manifest.language.clone()),
        dialect: Some(manifest.dialect.clone()),
    });
    let metadata = Metadata {
        schema: METADATA_SCHEMA,
        root: root_id,
        target: manifest.target.clone(),
        profile: manifest.profile.clone(),
        fill_missing_arguments: manifest.fill_missing_arguments,
        debug_info: manifest.debug_info,
        source: manifest.source_path.value.clone(),
        output: manifest.output_path.value.clone(),
        user_flags: manifest.user_flags.clone(),
        packages: resolved_packages,
        dependencies: edges,
        scripts: selections,
        external_requirements: requirements,
    };
    info!(
        script_count = metadata.scripts.len(),
        "project resolution complete"
    );
    Ok(metadata)
}

fn error(kind: ResolveErrorKind, location: Option<SourceSpan>) -> ResolveError {
    ResolveError {
        kind,
        location: location.map(Box::new),
        related: Box::new([]),
        providers: Box::new([]),
    }
}

#[cfg(test)]
mod tests;
