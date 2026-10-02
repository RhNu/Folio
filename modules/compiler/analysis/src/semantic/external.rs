//! Immutable declaration indexing and source-independent semantic preparation.

use super::*;
use folio_format_declarations::DeclarationBundle;
use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct ExternalDeclarations {
    pub(crate) bundles: Vec<DeclarationBundle>,
    index: BTreeMap<String, (usize, usize)>,
    pub(super) entries: Vec<ExternalEntry>,
    validation: Mutex<Option<ExternalValidation>>,
}

#[cfg(test)]
mod tests;

/// Body and source-member edits cannot affect external type visibility or ancestry.
struct ExternalValidation {
    source_headers: Vec<(String, Option<String>)>,
    diagnostics: Arc<Vec<Diagnostic>>,
}

pub(super) struct ExternalEntry {
    pub(super) info: Arc<ScriptInfo>,
    pub(super) diagnostics: Vec<Diagnostic>,
}

impl ExternalDeclarations {
    /// Prepare signatures and literal checks once for all views of these API inputs.
    pub(crate) fn new(bundles: Vec<DeclarationBundle>) -> Self {
        let mut index = BTreeMap::new();
        let mut entries = Vec::new();
        for (bundle_index, bundle) in bundles.iter().enumerate() {
            for (script_index, script) in bundle.scripts.iter().enumerate() {
                index
                    .entry(key(&script.name))
                    .or_insert((bundle_index, script_index));
                let info = Arc::new(script_from_external(script));
                let mut diagnostics = Vec::new();
                validate_external_initializers(script, &mut diagnostics);
                validate_external_defaults(&info, &mut diagnostics);
                entries.push(ExternalEntry { info, diagnostics });
            }
        }
        tracing::debug!(
            scripts = entries.len(),
            selected = index.len(),
            "prepared external analysis inputs"
        );
        Self {
            bundles,
            index,
            entries,
            validation: Mutex::default(),
        }
    }

    pub(crate) fn script(&self, name: &str) -> Option<&ExternalScript> {
        let &(bundle, script) = self.index.get(&key(name))?;
        Some(&self.bundles[bundle].scripts[script])
    }

    /// Keep a bounded cache while allowing older views to retain their own diagnostics.
    pub(super) fn validate_world(
        &self,
        world: &World,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Arc<Vec<Diagnostic>>, AnalysisCancelled> {
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        let source_headers = world
            .scripts
            .iter()
            .filter(|(_, script)| script.definition.is_some())
            .map(|(name, script)| (name.clone(), script.parent.as_deref().map(key)))
            .collect::<Vec<_>>();
        {
            let validation = self
                .validation
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(cached) = validation.as_ref()
                && cached.source_headers == source_headers
            {
                tracing::trace!("reused external world validation");
                return Ok(Arc::clone(&cached.diagnostics));
            }
        }
        // No mutex spans semantic work: independent generations may be cancelled separately.
        let diagnostics = Arc::new(validate_external_world(world, cancelled)?);
        *self
            .validation
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(ExternalValidation {
            source_headers,
            diagnostics: Arc::clone(&diagnostics),
        });
        tracing::debug!(diagnostics = diagnostics.len(), "validated external world");
        Ok(diagnostics)
    }
}
