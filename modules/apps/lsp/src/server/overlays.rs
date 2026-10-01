//! Unsaved PSC dependency APIs remain declarations; their bodies never enter root analysis.
use super::*;
use folio_declaration_tools::{GenerationOptions, SourceInput, generate};
use folio_format_declarations::semantic_digest;
use folio_project_model::{DeclarationLocation, DeclaredScript, DependencyKind, SourceId};
use folio_project_resolve::io::{InputSnapshot, decode_source};

/// Reproject existing dependency files from explicit buffers and verified disk snapshots.
pub(super) fn apply_dependency_overlays(
    loaded: &mut LoadedProject,
    overlays: &BTreeMap<PathBuf, Arc<str>>,
) -> Result<bool, LspError> {
    let mut changed = false;
    for dependency in &mut loaded.dependencies {
        if dependency.kind != DependencyKind::Psc {
            continue;
        }
        let Some(original) = loaded.declaration_bundles.get(&dependency.source_key) else {
            continue;
        };
        if !original
            .scripts
            .iter()
            .filter_map(|script| script.source.as_ref())
            .any(|source| overlays.contains_key(&dependency.canonical_path.join(&source.path)))
        {
            continue;
        }
        let SourceId::Dependency { index, .. } = dependency.source_id else {
            continue;
        };
        let encoding = loaded
            .root
            .manifest
            .dependencies
            .get(index)
            .ok_or_else(|| LspError::Project("PSC dependency has no manifest entry".into()))?
            .encoding;
        let mut sources = Vec::new();
        for script in &original.scripts {
            let source = script
                .source
                .as_ref()
                .ok_or_else(|| LspError::Project("PSC dependency has no source mapping".into()))?;
            let path = dependency.canonical_path.join(&source.path);
            let text = if let Some(text) = overlays.get(&path) {
                text.to_string()
            } else {
                let bytes = loaded
                    .input_snapshots
                    .iter()
                    .find_map(|snapshot| match snapshot {
                        InputSnapshot::File {
                            path: candidate,
                            bytes,
                        } if candidate == &path => Some(bytes),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        LspError::Project("PSC dependency source snapshot is missing".into())
                    })?;
                decode_source(bytes, encoding)
                    .map_err(|error| LspError::Project(error.to_string()))?
            };
            sources.push((source.path.clone(), text));
        }
        let inputs = sources
            .iter()
            .map(|(path, text)| SourceInput { path, text })
            .collect::<Vec<_>>();
        let bundle = generate(
            GenerationOptions {
                source: &original.origin.source,
            },
            &inputs,
        )
        .map_err(|error| {
            LspError::Project(format!("unsaved dependency {}: {error}", dependency.name))
        })?;
        if &bundle == original {
            continue;
        }
        if let SourceId::Dependency { digest, .. } = &mut dependency.source_id {
            *digest = semantic_digest(&bundle);
        }
        dependency.scripts = bundle
            .scripts
            .iter()
            .map(|script| DeclaredScript {
                name: script.name.clone(),
                location: DeclarationLocation {
                    carrier_path: dependency.declared_path.clone(),
                    script_name: script.name.to_ascii_lowercase(),
                    source_path: script.source.as_ref().map(|source| source.path.clone()),
                    line: script.source.as_ref().map(|source| source.line),
                    column: script.source.as_ref().map(|source| source.column),
                },
            })
            .collect();
        tracing::debug!(dependency=%dependency.name,scripts=bundle.scripts.len(),"projected unsaved PSC dependency declarations");
        loaded
            .declaration_bundles
            .insert(dependency.source_key.clone(), bundle);
        changed = true;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests;
