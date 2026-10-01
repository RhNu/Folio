//! All semantic editor queries run against one cancellable immutable snapshot.
use super::*;
use crate::protocol::{parse_position, path_to_uri, range_json};
use folio_hir::Symbol;
use folio_ide::PositionIndex;
use folio_source::{SourceSpan, TextRange};

mod features;
mod navigation;

#[derive(Clone)]
pub(super) struct QueryContext {
    view: Option<Arc<ProjectAnalysisView>>,
    metadata: Option<Arc<Metadata>>,
    loaded: Option<LoadedProject>,
    paths: BTreeMap<PathBuf, FileId>,
    uris: BTreeMap<String, FileId>,
    versions: BTreeMap<FileId, Option<i32>>,
    encoding: PositionEncoding,
    generation: u64,
    settings: settings::EditorSettings,
    markdown: bool,
    commands: bool,
    virtual_documents: bool,
    overlays: BTreeMap<PathBuf, Arc<str>>,
}

pub(super) type QueryResult = Result<Value, (i32, String)>;

impl Server {
    pub(super) fn editor_request(
        &self,
        id: Value,
        method: &str,
        params: &Value,
    ) -> Result<(), LspError> {
        let context = QueryContext {
            view: self.view.clone(),
            metadata: self.metadata.clone(),
            loaded: self.projected_inputs.clone(),
            paths: self.paths.clone(),
            uris: self
                .paths
                .iter()
                .map(|(path, file)| (path_to_uri(path), *file))
                .collect(),
            versions: self
                .paths
                .iter()
                .map(|(path, file)| (*file, self.documents.version(path)))
                .collect(),
            encoding: self.encoding,
            generation: self.generation.load(Ordering::SeqCst),
            settings: self.editor_settings.clone(),
            markdown: self.markdown,
            commands: self.client_commands,
            virtual_documents: self.declaration_documents,
            overlays: self
                .documents
                .overlays()
                .map(|(path, text)| (path.clone(), Arc::<str>::from(text)))
                .collect(),
        };
        let method = method.to_owned();
        let params = params.clone();
        let generation = Arc::clone(&self.generation);
        let cancelled = Arc::clone(&self.cancelled);
        let pending = Arc::clone(&self.pending);
        let output = Arc::clone(&self.output);
        let publication = Arc::clone(&self.publication);
        let key = id.to_string();
        pending
            .lock()
            .map_err(|_| LspError::Protocol("pending lock poisoned".into()))?
            .insert(key.clone());
        std::thread::spawn(move || {
            let started = Instant::now();
            let obsolete = || {
                requests::request_stale(
                    context.generation,
                    generation.load(Ordering::SeqCst),
                    cancelled.lock().is_ok_and(|items| items.contains(&key)),
                )
            };
            let result = if obsolete()
                || context
                    .view
                    .as_ref()
                    .is_some_and(|view| view.analysis.try_warm_semantics(obsolete).is_err())
            {
                Err((-32800, "request cancelled".into()))
            } else {
                context.answer(&method, &params)
            };
            // The handler cannot invalidate or cancel between this check and the write.
            let publication_guard = publication.lock();
            let result = if publication_guard.is_err() {
                Err((-32603, "publication state unavailable".into()))
            } else if obsolete() {
                Err((-32800, "request cancelled".into()))
            } else {
                result
            };
            let response = match result {
                Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                Err((code, message)) => {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
                }
            };
            if let Err(error) = send(&output, &response) {
                tracing::error!(%error, method, "failed to send editor query response");
            }
            drop(publication_guard);
            tracing::debug!(
                method,
                generation = context.generation,
                elapsed_us = started.elapsed().as_micros(),
                "completed editor query"
            );
            if let Ok(mut items) = pending.lock() {
                items.remove(&key);
                if let Ok(mut items) = cancelled.lock() {
                    items.remove(&key);
                }
            }
        });
        Ok(())
    }
}

impl QueryContext {
    fn answer(&self, method: &str, params: &Value) -> QueryResult {
        if self.view.is_none() {
            return Ok(empty_answer(method));
        }
        match method {
            "textDocument/hover"
            | "textDocument/definition"
            | "textDocument/declaration"
            | "textDocument/references"
            | "textDocument/implementation"
            | "textDocument/documentHighlight"
            | "folio/declarationContent" => self.navigation(method, params),
            _ => self.features(method, params),
        }
    }

    fn file(&self, uri: &str) -> Option<FileId> {
        if let Some(file) = self.uris.get(uri) {
            return Some(*file);
        }
        self.paths.get(&uri_to_path(uri)?).copied()
    }

    fn at(&self, params: &Value) -> Option<(FileId, usize)> {
        let file = self.file(params["textDocument"]["uri"].as_str()?)?;
        let text = self.view.as_ref()?.analysis.text(file)?;
        let byte = folio_ide::offset(text, parse_position(&params["position"])?, self.encoding)?;
        Some((file, byte))
    }

    fn locations(&self, spans: &[SourceSpan]) -> Vec<Value> {
        let Some(view) = &self.view else {
            return Vec::new();
        };
        spans
            .iter()
            .filter_map(|span| presentation::source_location(view, *span, self.encoding))
            .collect()
    }

    fn document_source(&self, uri: &str) -> Option<String> {
        if let Some(owner) = presentation::virtual_owner(uri) {
            return self.declaration_text(&owner);
        }
        let path = uri_to_path(uri)?;
        let loaded = self.loaded.as_ref()?;
        let metadata = self.metadata.as_ref()?;
        for selection in &metadata.scripts {
            if let Some((candidate, text)) =
                presentation::external_source(metadata, loaded, &selection.script, &self.overlays)
                && candidate == path
            {
                return Some(text);
            }
        }
        None
    }

    fn occurrence(&self, params: &Value) -> Option<Symbol> {
        if let Some((file, byte)) = self.at(params) {
            let occurrence = folio_ide::symbol_at(self.view.as_ref()?, file, byte)?;
            return Some(occurrence.symbol);
        }
        let uri = params["textDocument"]["uri"].as_str()?;
        let text = self.document_source(uri)?;
        let byte = folio_ide::offset(&text, parse_position(&params["position"])?, self.encoding)?;
        if let Some(owner) = presentation::virtual_owner(uri) {
            let document = folio_ide::declaration_document(self.view.as_ref()?, &owner)?;
            let prefix = text.len().checked_sub(document.text.len())?;
            if let Some((symbol, _)) = document
                .declarations
                .iter()
                .find(|(_, range)| range.start + prefix <= byte && byte < range.end + prefix)
            {
                return Some(symbol.clone());
            }
        }
        folio_ide::external_declaration_symbol(&text, byte)
    }
}

/// Unsupported documents and an unavailable project still receive valid result shapes.
fn empty_answer(method: &str) -> Value {
    match method {
        "textDocument/semanticTokens/full" => json!({"data":[]}),
        "textDocument/documentSymbol"
        | "textDocument/codeLens"
        | "textDocument/inlayHint"
        | "textDocument/references"
        | "textDocument/implementation"
        | "textDocument/documentHighlight"
        | "workspace/symbol"
        | "textDocument/completion" => json!([]),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests;
