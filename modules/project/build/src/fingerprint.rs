//! Stable semantic build identity, separate from the loader's physical snapshot.

use folio_project_model::{Metadata, SourceId};
use folio_project_resolve::LoadedProject;

/// Bump when key inputs or cache representation change.
pub const CACHE_SCHEMA: u32 = 6;

/// Logical provider identity excludes host paths, carrier bytes and provenance.
fn occurrence(source: &SourceId) -> Option<usize> {
    match source {
        SourceId::Project => None,
        SourceId::Dependency { index, .. } => Some(*index),
    }
}

/// Hash semantic inputs, dependency occurrences and selected whole-script APIs.
/// Physical input changes are checked independently before publishing artifacts.
pub fn command_fingerprint(
    project: &LoadedProject,
    metadata: &Metadata,
    compiler_identity: &str,
    target_decisions: &[String],
) -> String {
    let root = &project.root.manifest;
    let mut sources = project
        .source_inputs
        .iter()
        .map(|input| {
            (
                &input.display_path,
                blake3::hash(input.text.as_bytes()).to_hex().to_string(),
            )
        })
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| left.0.cmp(right.0));
    let declarations = project
        .dependencies
        .iter()
        .map(|dependency| {
            let bundle = project
                .declaration_bundles
                .get(&dependency.source_key)
                .expect("loaded dependency owns a declaration bundle");
            (
                &dependency.name,
                dependency.kind,
                folio_format_declarations::semantic_digest(bundle),
            )
        })
        .collect::<Vec<_>>();
    let dependency_settings = root
        .dependencies
        .iter()
        .map(|dependency| (&dependency.name.value, dependency.kind))
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
                        &provider.package.name,
                        occurrence(&provider.package.source),
                        &provider.source_path,
                    )
                })
                .collect::<Vec<_>>();
            (
                &selection.script,
                &selection.selected.package.name,
                occurrence(&selection.selected.package.source),
                &selection.selected.source_path,
                providers,
                selection.reason,
            )
        })
        .collect::<Vec<_>>();
    let settings = (
        &root.name,
        &root.version,
        &root.source_path.value,
        &root.language,
        &root.dialect,
        &root.extensions,
        &root.emit,
        root.experimental_pex_dependencies,
        &metadata.target,
        &metadata.profile,
        &metadata.user_flags,
        metadata.fill_missing_arguments,
        metadata.debug_info,
    );
    let payload = serde_json::to_vec(&(
        CACHE_SCHEMA,
        env!("CARGO_PKG_VERSION"),
        "folio-pex-skyrim-v1",
        compiler_identity,
        settings,
        dependency_settings,
        sources,
        declarations,
        selections,
        target_decisions,
    ))
    .expect("build key inputs are serializable");
    blake3::hash(&payload).to_hex().to_string()
}

/// A unit key includes its logical identity even when its command inputs match.
pub fn unit_fingerprint(command: &str, unit: &super::plan::BuildUnit) -> String {
    let identity = serde_json::to_vec(&(command, unit)).expect("build unit is serializable");
    blake3::hash(&identity).to_hex().to_string()
}
