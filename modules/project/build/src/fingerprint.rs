//! Canonical, conservative content identity for one build command.

use folio_project_model::{DependencyKind, Metadata};
use folio_project_resolve::LoadedProject;
use serde::Serialize;

/// Bump when key inputs or cache representation change.
pub const CACHE_SCHEMA: u32 = 4;

#[derive(Serialize)]
struct SourceContent<'a> {
    package: &'a folio_project_model::SourceId,
    path: &'a str,
    dialect: &'a str,
    digest: String,
}

#[derive(Serialize)]
struct ManifestSettings<'a> {
    package: &'a folio_project_model::SourceId,
    name: &'a str,
    version: &'a str,
    source: &'a str,
    language: &'a str,
    dialect: &'a str,
    extensions: &'a [String],
    flags: &'a [String],
    fill_missing_arguments: bool,
    target: &'a str,
    profile: &'a str,
    debug_info: bool,
    experimental_pex_dependencies: bool,
    emit: &'a [String],
    dependencies: Vec<(&'a str, DependencyKind, &'a str)>,
}

/// Hash all loaded semantic inputs, resolved provider decisions and target options.
/// Host absolute paths and unordered map iteration never enter this payload.
pub fn command_fingerprint(
    project: &LoadedProject,
    metadata: &Metadata,
    compiler_identity: &str,
    target_decisions: &[String],
) -> String {
    let packages = project
        .packages
        .iter()
        .map(|package| (&package.source_key, package))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut sources = Vec::new();
    for input in &project.source_inputs {
        let package = packages
            .get(&input.package_key)
            .expect("loaded source belongs to a loaded package");
        let dialect = package
            .manifest()
            .map(|manifest| manifest.dialect.as_str())
            .unwrap_or("");
        sources.push(SourceContent {
            package: &package.source_id,
            path: &input.display_path,
            dialect,
            digest: blake3::hash(input.text.as_bytes()).to_hex().to_string(),
        });
    }
    sources.sort_by(|left, right| (&left.package, left.path).cmp(&(&right.package, right.path)));
    let mut manifests = Vec::new();
    for package in &project.packages {
        if let Some(manifest) = package.manifest() {
            manifests.push(ManifestSettings {
                package: &package.source_id,
                name: &manifest.name,
                version: &manifest.version,
                source: &manifest.source_path.value,
                language: &manifest.language,
                dialect: &manifest.dialect,
                extensions: &manifest.extensions,
                flags: &manifest.user_flags,
                fill_missing_arguments: manifest.fill_missing_arguments,
                target: &manifest.target,
                profile: &manifest.profile,
                debug_info: manifest.debug_info,
                experimental_pex_dependencies: manifest.experimental_pex_dependencies,
                emit: &manifest.emit,
                dependencies: manifest
                    .dependencies
                    .iter()
                    .map(|item| {
                        (
                            item.name.value.as_str(),
                            item.kind,
                            item.path.value.as_str(),
                        )
                    })
                    .collect(),
            });
        }
    }
    manifests.sort_by(|left, right| left.package.cmp(right.package));
    let mut declarations = project
        .declaration_bundles
        .iter()
        .map(|(key, bundle)| {
            let package = packages
                .get(key)
                .expect("declaration belongs to loaded package");
            (&package.source_id, bundle)
        })
        .collect::<Vec<_>>();
    declarations.sort_by(|left, right| left.0.cmp(right.0));
    let packages = metadata
        .packages
        .iter()
        .map(|package| {
            (
                &package.id,
                &package.source_root,
                &package.source_files,
                &package.language,
                &package.dialect,
            )
        })
        .collect::<Vec<_>>();
    let dependencies = metadata
        .dependencies
        .iter()
        .map(|edge| (&edge.from, &edge.to, edge.kind, &edge.declared_path))
        .collect::<Vec<_>>();
    let selections = metadata
        .scripts
        .iter()
        .map(|selection| {
            let providers = selection
                .providers
                .iter()
                .map(|provider| {
                    (
                        &provider.package,
                        &provider.source_path,
                        provider
                            .declaration
                            .as_ref()
                            .map(|location| location.script_index),
                    )
                })
                .collect::<Vec<_>>();
            (
                &selection.script,
                &selection.selected.package,
                &selection.selected.source_path,
                providers,
                selection.reason,
            )
        })
        .collect::<Vec<_>>();
    let resolution = (
        &metadata.root,
        &metadata.target,
        &metadata.profile,
        metadata.fill_missing_arguments,
        metadata.debug_info,
        &metadata.user_flags,
        packages,
        dependencies,
        selections,
        &metadata.external_requirements,
    );
    let payload = serde_json::to_vec(&(
        CACHE_SCHEMA,
        env!("CARGO_PKG_VERSION"),
        "folio-pex-skyrim-v1",
        compiler_identity,
        resolution,
        manifests,
        sources,
        declarations,
        target_decisions,
    ))
    .expect("cache key inputs are serializable");
    blake3::hash(&payload).to_hex().to_string()
}

/// A unit key includes its logical identity even when its command inputs match another unit.
pub fn unit_fingerprint(command: &str, unit: &super::plan::BuildUnit) -> String {
    let identity = serde_json::to_vec(&(command, unit)).expect("build unit is serializable");
    blake3::hash(&identity).to_hex().to_string()
}
