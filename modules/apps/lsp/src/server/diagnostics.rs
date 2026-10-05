//! Publish diagnostics from one coherent project generation.
use super::*;
use crate::protocol::{path_to_uri, range_json};
use folio_diagnostics::Severity;
use folio_ide::PositionIndex;

impl Server {
    pub(super) fn clear_view(&mut self) -> Result<(), LspError> {
        self.view = None;
        self.ide = None;
        self.projected_inputs = None;
        self.metadata = None;
        self.paths.clear();
        self.project_message = None;
        for uri in &self.published {
            send(
                &self.output,
                &json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"diagnostics":[]}}),
            )?;
        }
        self.published.clear();
        self.diagnostics.clear();
        self.refresh_editor()?;
        Ok(())
    }

    pub(super) fn publish(
        &mut self,
        view: Arc<ProjectAnalysisView>,
        generation: u64,
        diagnostics: Vec<folio_diagnostics::Diagnostic>,
    ) -> Result<(), LspError> {
        let started = Instant::now();
        let current = view
            .sources
            .values()
            .map(|source| path_to_uri(&source.canonical_path))
            .collect::<BTreeSet<_>>();
        let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for source in view.sources.values() {
            grouped.insert(path_to_uri(&source.canonical_path), Vec::new());
        }
        let mut project_messages = Vec::new();
        let positions = view
            .sources
            .iter()
            .map(|(&file, source)| (file, PositionIndex::new(&source.text)))
            .collect::<BTreeMap<_, _>>();
        tracing::debug!(
            generation,
            elapsed_us = started.elapsed().as_micros(),
            phase = "diagnostics",
            "LSP project phase complete"
        );
        let started = Instant::now();
        for diagnostic in diagnostics {
            let Some(span) = diagnostic.primary else {
                project_messages.push(format!("{}: {}", diagnostic.code, diagnostic.message));
                continue;
            };
            let Some(source) = view.sources.get(&span.file) else {
                continue;
            };
            let Some(range) = positions[&span.file].range(span.range, self.encoding) else {
                continue;
            };
            let severity = match diagnostic.severity {
                Severity::Error => 1,
                Severity::Warning => 2,
                Severity::Info => 3,
            };
            grouped.entry(path_to_uri(&source.canonical_path)).or_default().push(json!({"range": range_json(range), "severity":severity,"code":diagnostic.code,"source":"folio","message":diagnostic.message}));
        }
        for old in &self.published {
            grouped.entry(old.clone()).or_default();
        }
        for (uri, diagnostics) in grouped.iter() {
            if self.generation.load(Ordering::SeqCst) != generation {
                return Ok(());
            }
            let version = uri_to_path(uri)
                .as_ref()
                .and_then(|path| self.documents.version(path));
            send(
                &self.output,
                &json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"version":version,"diagnostics":diagnostics}}),
            )?;
        }
        self.diagnostics = grouped
            .into_iter()
            .filter(|(uri, _)| current.contains(uri))
            .collect();
        self.published = current;
        let project_message = (!project_messages.is_empty()).then(|| project_messages.join("\n"));
        if project_message != self.project_message {
            if let Some(message) = &project_message {
                send(
                    &self.output,
                    &json!({"jsonrpc":"2.0","method":"window/showMessage","params":{"type":1,"message":message}}),
                )?;
            }
            self.project_message = project_message;
        }
        tracing::debug!(
            generation,
            elapsed_us = started.elapsed().as_micros(),
            phase = "publish",
            "LSP project phase complete"
        );
        Ok(())
    }
}
