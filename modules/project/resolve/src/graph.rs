//! Deterministic decisions over explicitly loaded, in-memory project inputs.

use std::collections::{BTreeMap, BTreeSet};

use folio_project_model::{
    DependencyEdge, ExternalRequirement, LoadedPackage, Metadata, PackageId, ResolvedPackage,
    ScriptProvider, ScriptSelection, SelectionReason, SourceId, SourceSpan,
};
use tracing::{debug, info, instrument};

pub const METADATA_SCHEMA: u32 = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveError {
    pub kind: ResolveErrorKind,
    pub location: Option<Box<SourceSpan>>,
    pub related: Box<[SourceSpan]>,
    pub providers: Box<[ScriptProvider]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveErrorKind {
    MissingSource(String),
    DuplicateSource(String),
    UnreachableSource(String),
    InvalidCarrier(String),
    DuplicatePackageName(String),
    MissingDependency(String),
    DependencyCycle(String),
    DependencyIdentityMismatch {
        declared: String,
        loaded: String,
    },
    DependencyKindMismatch(String),
    TargetMismatch {
        package: String,
        expected: String,
        actual: String,
    },
    AbiMismatch {
        package: String,
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
        for location in &self.related {
            write!(
                f,
                "; related {}:{}..{}",
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

/// Resolve all packages, edges, script providers and runtime requirements.
/// Input order does not affect the result or which deterministic error is reported.
#[instrument(name = "dependency.resolve", skip_all, fields(package_id = %root_key, phase = "resolve"))]
pub fn resolve(root_key: &str, packages: &[LoadedPackage]) -> Result<Metadata, ResolveError> {
    info!(package_count = packages.len(), "resolving loaded project");
    let mut indexed = BTreeMap::new();
    for package in packages {
        if indexed
            .insert(package.source_key.as_str(), package)
            .is_some()
        {
            return Err(error(
                ResolveErrorKind::DuplicateSource(package.source_key.clone()),
                None,
            ));
        }
    }
    let root = *indexed
        .get(root_key)
        .ok_or_else(|| error(ResolveErrorKind::MissingSource(root_key.into()), None))?;
    let root_manifest = root
        .manifest()
        .ok_or_else(|| error(ResolveErrorKind::InvalidCarrier(root_key.into()), None))?;
    if root.source_id != SourceId::Project {
        return Err(error(
            ResolveErrorKind::InvalidCarrier(root_key.into()),
            None,
        ));
    }
    let mut ids = BTreeMap::new();
    let mut names = BTreeMap::<&str, (&str, Option<SourceSpan>)>::new();
    for (&key, package) in &indexed {
        let (name, version, location) = match &package.carrier {
            folio_project_model::LoadedCarrier::Manifest(manifest) => (
                &manifest.name,
                &manifest.version,
                manifest.fields.get("package.name").cloned(),
            ),
            folio_project_model::LoadedCarrier::Declarations { sdk, .. } => {
                (&sdk.name, &sdk.version, None)
            }
        };
        if let Some((prior_key, prior_span)) = names.insert(name, (key, location.clone()))
            && prior_key != key
        {
            return Err(ResolveError {
                kind: ResolveErrorKind::DuplicatePackageName(name.clone()),
                location: location.map(Box::new),
                related: prior_span.into_iter().collect(),
                providers: Box::new([]),
            });
        }
        ids.insert(
            key,
            PackageId {
                name: name.clone(),
                version: version.clone(),
                source: package.source_id.clone(),
            },
        );
    }

    let mut edges = Vec::new();
    let mut adjacency = BTreeMap::<&str, Vec<&str>>::new();
    for (&key, package) in &indexed {
        let Some(manifest) = package.manifest() else {
            continue;
        };
        for (index, dep) in manifest.dependencies.iter().enumerate() {
            let links: Vec<_> = package
                .links
                .iter()
                .filter(|link| link.dependency_index == index)
                .collect();
            if links.len() != 1 {
                return Err(error(
                    ResolveErrorKind::MissingDependency(dep.name.value.clone()),
                    Some(dep.path.span.clone()),
                ));
            }
            let target_key = links[0].source_key.as_str();
            let target = indexed.get(target_key).ok_or_else(|| {
                error(
                    ResolveErrorKind::MissingSource(target_key.into()),
                    Some(dep.path.span.clone()),
                )
            })?;
            let loaded_id = &ids[target_key];
            if dep.name.value != loaded_id.name {
                return Err(error(
                    ResolveErrorKind::DependencyIdentityMismatch {
                        declared: dep.name.value.clone(),
                        loaded: loaded_id.name.clone(),
                    },
                    Some(dep.name.span.clone()),
                ));
            }
            let kind_matches = dep.kind == target.kind();
            if !kind_matches {
                return Err(error(
                    ResolveErrorKind::DependencyKindMismatch(dep.name.value.clone()),
                    Some(dep.name.span.clone()),
                ));
            }
            edges.push(DependencyEdge {
                from: ids[key].clone(),
                to: loaded_id.clone(),
                kind: dep.kind,
                declared_path: dep.path.value.clone(),
                declaration: dep.name.span.clone(),
            });
            adjacency.entry(key).or_default().push(target_key);
        }
        if package
            .links
            .iter()
            .any(|link| link.dependency_index >= manifest.dependencies.len())
        {
            return Err(error(ResolveErrorKind::InvalidCarrier(key.into()), None));
        }
    }
    edges.sort_by(|left, right| {
        (&left.from, &left.to, &left.declared_path).cmp(&(
            &right.from,
            &right.to,
            &right.declared_path,
        ))
    });

    // An I/O shell may gather candidate files, but only declared graph sources count.
    let mut reachable = BTreeSet::new();
    let mut pending = vec![root_key];
    while let Some(key) = pending.pop() {
        if reachable.insert(key) {
            pending.extend(adjacency.get(key).into_iter().flatten().copied());
        }
    }
    if let Some((&key, _)) = indexed.iter().find(|(key, _)| !reachable.contains(**key)) {
        return Err(error(ResolveErrorKind::UnreachableSource(key.into()), None));
    }
    let precedence = ordered_sources(root_key, &adjacency)?
        .into_iter()
        .enumerate()
        .map(|(rank, key)| (ids[key].clone(), rank))
        .collect::<BTreeMap<_, _>>();
    debug!(
        package_count = precedence.len(),
        "resolved dependency precedence"
    );
    let root_id = ids[root_key].clone();
    let mut resolved_packages = Vec::new();
    let mut providers = BTreeMap::<String, Vec<ScriptProvider>>::new();
    for (&key, package) in &indexed {
        let id = ids[key].clone();
        let (source_root, language, dialect) = package.manifest().map_or_else(
            || (None, None, None),
            |manifest| {
                (
                    Some(manifest.source_path.value.clone()),
                    Some(manifest.language.clone()),
                    Some(manifest.dialect.clone()),
                )
            },
        );
        let mut source_files: Vec<_> = package
            .source_files
            .iter()
            .map(|file| file.path.clone())
            .collect();
        source_files.sort();
        resolved_packages.push(ResolvedPackage {
            id: id.clone(),
            source_root,
            source_files,
            language,
            dialect,
        });
        for file in &package.source_files {
            providers
                .entry(normalize_script(&file.script_candidate))
                .or_default()
                .push(ScriptProvider {
                    script: file.script_candidate.clone(),
                    package: id.clone(),
                    definition: None,
                    declaration: None,
                    source_path: Some(file.display_path.clone()),
                });
        }
        if let Some(sdk) = package.sdk() {
            if sdk.target != root_manifest.target {
                return Err(error(
                    ResolveErrorKind::TargetMismatch {
                        package: id.name.clone(),
                        expected: root_manifest.target.clone(),
                        actual: sdk.target.clone(),
                    },
                    root_manifest.fields.get("build.target").cloned(),
                ));
            }
            let expected_abi = format!("papyrus-{}", root_manifest.dialect);
            if sdk.abi != expected_abi {
                return Err(error(
                    ResolveErrorKind::AbiMismatch {
                        package: id.name.clone(),
                        expected: expected_abi,
                        actual: sdk.abi.clone(),
                    },
                    root_manifest
                        .fields
                        .get("languages.papyrus.dialect")
                        .cloned(),
                ));
            }
            for script in &sdk.scripts {
                providers
                    .entry(normalize_script(&script.name))
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
    }
    resolved_packages.sort_by(|left, right| left.id.cmp(&right.id));
    let mut selections = Vec::new();
    for (script, mut choices) in providers {
        choices.sort_by(|left, right| {
            let left_rank = precedence[&left.package];
            let right_rank = precedence[&right.package];
            (left_rank, &left.source_path).cmp(&(right_rank, &right.source_path))
        });
        let (selected, reason) = select_script(&script, &choices)?;
        debug!(script, selected_package = %selected.package.name, provider_count = choices.len(), ?reason, "selected script provider");
        selections.push(ScriptSelection {
            script,
            selected,
            providers: choices,
            reason,
        });
    }
    let mut external_requirements = BTreeMap::new();
    for (&key, package) in &indexed {
        let id = &ids[key];
        if id == &root_id {
            continue;
        }
        let sdk = package.sdk();
        external_requirements.insert(
            id.clone(),
            ExternalRequirement {
                package: id.clone(),
                target: sdk.map_or_else(|| root_manifest.target.clone(), |sdk| sdk.target.clone()),
                abi: sdk.map_or_else(
                    || format!("papyrus-{}", root_manifest.dialect),
                    |sdk| sdk.abi.clone(),
                ),
                reason: match package.kind() {
                    folio_project_model::DependencyKind::Psc => {
                        "PSC API requires an external runtime"
                    }
                    folio_project_model::DependencyKind::Pex => {
                        "PEX API requires an external runtime"
                    }
                    folio_project_model::DependencyKind::Sdk
                    | folio_project_model::DependencyKind::Builtin => {
                        "SDK API supplied by external runtime"
                    }
                    folio_project_model::DependencyKind::Package => {
                        "source dependency is visible without local code generation"
                    }
                }
                .into(),
            },
        );
    }
    let metadata = Metadata {
        schema: METADATA_SCHEMA,
        root: root_id,
        target: root_manifest.target.clone(),
        profile: root_manifest.profile.clone(),
        fill_missing_arguments: root_manifest.fill_missing_arguments,
        debug_info: root_manifest.debug_info,
        source: root_manifest.source_path.value.clone(),
        output: root_manifest.output_path.value.clone(),
        user_flags: root_manifest.user_flags.clone(),
        packages: resolved_packages,
        dependencies: edges,
        scripts: selections,
        external_requirements: external_requirements.into_values().collect(),
    };
    info!(
        package_count = metadata.packages.len(),
        script_count = metadata.scripts.len(),
        "project resolution complete"
    );
    Ok(metadata)
}

fn select_script(
    script: &str,
    choices: &[ScriptProvider],
) -> Result<(ScriptProvider, SelectionReason), ResolveError> {
    if choices
        .windows(2)
        .any(|pair| pair[0].package == pair[1].package)
    {
        let mut issue = error(ResolveErrorKind::ScriptConflict(script.into()), None);
        issue.providers = choices.to_vec().into_boxed_slice();
        return Err(issue);
    }
    let selected = choices.last().expect("script has a provider").clone();
    let reason = if choices.len() == 1 {
        SelectionReason::SoleProvider
    } else {
        SelectionReason::DependencyOrder
    };
    Ok((selected, reason))
}

/// Rank each package by its last declaration path without expanding every diamond path.
fn ordered_sources<'a>(
    root: &'a str,
    adjacency: &BTreeMap<&'a str, Vec<&'a str>>,
) -> Result<Vec<&'a str>, ResolveError> {
    let mut active = BTreeSet::new();
    let mut finished = BTreeSet::new();
    let mut postorder = Vec::new();
    visit_source(root, adjacency, &mut active, &mut finished, &mut postorder)?;
    postorder.reverse();
    let mut paths = BTreeMap::from([(root, Vec::<usize>::new())]);
    for key in postorder {
        let parent_path = paths[key].clone();
        for (index, dependency) in adjacency.get(key).into_iter().flatten().enumerate() {
            let mut candidate = parent_path.clone();
            candidate.push(index);
            if paths
                .get(dependency)
                .is_none_or(|current| candidate > *current)
            {
                paths.insert(dependency, candidate);
            }
        }
    }
    let mut sources = paths.into_iter().collect::<Vec<_>>();
    sources.sort_by(|(_, left), (_, right)| {
        left.iter()
            .zip(right)
            .find_map(|(a, b)| (a != b).then(|| a.cmp(b)))
            .unwrap_or_else(|| right.len().cmp(&left.len()))
    });
    Ok(sources.into_iter().map(|(key, _)| key).collect())
}

fn visit_source<'a>(
    key: &'a str,
    adjacency: &BTreeMap<&'a str, Vec<&'a str>>,
    active: &mut BTreeSet<&'a str>,
    finished: &mut BTreeSet<&'a str>,
    postorder: &mut Vec<&'a str>,
) -> Result<(), ResolveError> {
    if !active.insert(key) {
        tracing::warn!(package_key = key, "dependency cycle detected");
        return Err(error(ResolveErrorKind::DependencyCycle(key.into()), None));
    }
    if !finished.contains(key) {
        for dependency in adjacency.get(key).into_iter().flatten() {
            visit_source(dependency, adjacency, active, finished, postorder)?;
        }
        finished.insert(key);
        postorder.push(key);
    }
    active.remove(key);
    Ok(())
}

fn normalize_script(name: &str) -> String {
    name.to_ascii_lowercase()
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
