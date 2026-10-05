//! All semantic editor queries run against one cancellable immutable snapshot.
use folio_hir::Symbol;
use folio_ide::PositionIndex;
use folio_source::{SourceSpan, TextRange};

use super::{
    Arc, BTreeMap, BTreeSet, FileId, Instant, LoadedProject, LspError, Metadata, Mutex, Ordering,
    PathBuf, PositionEncoding, Server, Value, json, presentation, requests, scheduling, send,
    settings, uri_to_path,
};
use crate::protocol::{parse_position, path_to_uri, range_json};

mod completions;
mod features;
mod navigation;
pub(super) use completions::CompletionCache;

#[derive(Clone)]
pub(super) struct QueryContext {
    view: Option<folio_ide::IdeSnapshot>,
    metadata: Option<Arc<Metadata>>,
    loaded: Option<Arc<LoadedProject>>,
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
    completions: CompletionCache,
}

pub(super) type QueryResult = Result<Value, (i32, String)>;

/// Retains the original arrival time while the matching project snapshot is loading.
pub(super) struct DeferredQuery {
    id: Value,
    method: String,
    params: Value,
    generation: u64,
    received: Instant,
}

/// Only interchangeable automatic requests may supersede each other while waiting.
fn query_meta(id: &str, generation: u64, method: &str, params: &Value) -> scheduling::QueryMeta {
    let interactive = matches!(
        method,
        "textDocument/completion"
            | "completionItem/resolve"
            | "textDocument/hover"
            | "textDocument/signatureHelp"
            | "textDocument/definition"
            | "textDocument/declaration"
            | "textDocument/documentSymbol"
            | "textDocument/documentHighlight"
    );
    let coalesce_key = matches!(
        method,
        "textDocument/semanticTokens/full" | "textDocument/codeLens" | "textDocument/inlayHint"
    )
    .then(|| format!("{method}:{params}"));
    scheduling::QueryMeta {
        id: id.into(),
        generation,
        interactive,
        coalesce_key,
    }
}

/// Retire bookkeeping before making a response visible: clients may then reuse its id.
fn finish_request(
    pending: &Mutex<BTreeSet<String>>,
    cancelled: &Mutex<BTreeSet<String>>,
    key: &str,
    publish: impl FnOnce() -> Result<(), LspError>,
) -> Result<(), LspError> {
    if let Ok(mut items) = pending.lock() {
        items.remove(key);
        if let Ok(mut items) = cancelled.lock() {
            items.remove(key);
        }
    }
    publish()
}

impl Server {
    pub(super) fn editor_request(
        &mut self,
        id: Value,
        method: &str,
        params: &Value,
        received: Instant,
    ) -> Result<(), LspError> {
        if self.session.shutdown {
            return self.error(&id, -32800, "server is shutting down");
        }
        if self.loading {
            if matches!(method, "completionItem/resolve" | "codeLens/resolve") {
                return self.error(&id, -32801, "project snapshot changed; request a new item");
            }
            if matches!(
                method,
                "textDocument/semanticTokens/full"
                    | "textDocument/codeLens"
                    | "textDocument/inlayHint"
            ) {
                return self.reply(&id, &empty_answer(method));
            }
            // Outlines have no standard refresh notification. Keep their request,
            // along with interactive queries, until the matching snapshot is ready.
            if self.deferred.len() >= 64 {
                return self.error(&id, -32800, "loading query queue is full");
            }
            self.pending
                .lock()
                .map_err(|_poisoned| LspError::Protocol("pending lock poisoned".into()))?
                .insert(id.to_string());
            self.deferred.push_back(DeferredQuery {
                id,
                method: method.to_owned(),
                params: params.clone(),
                generation: self.generation.load(Ordering::SeqCst),
                received,
            });
            return Ok(());
        }
        self.enqueue_query(id, method, params, received, 0)
    }

    fn enqueue_query(
        &mut self,
        id: Value,
        method: &str,
        params: &Value,
        received: Instant,
        loading_wait_us: u128,
    ) -> Result<(), LspError> {
        let context_started = Instant::now();
        let mut context = self.query_context();
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
            .map_err(|_poisoned| LspError::Protocol("pending lock poisoned".into()))?
            .insert(key.clone());
        let rejected_id = id.clone();
        let rejected_key = key.clone();
        let meta = query_meta(&key, context.generation, &method, &params);
        let cancelled_id = id.clone();
        let cancelled_key = key.clone();
        let cancelled_method = method.clone();
        let cancelled_output = Arc::clone(&output);
        let cancelled_pending = Arc::clone(&pending);
        let cancelled_items = Arc::clone(&cancelled);
        let context_us = context_started.elapsed().as_micros();
        let queued = Instant::now();
        let accepted = self.queries.submit(meta, move || {
            let started = Instant::now();
            let queue_wait_us = queued.elapsed().as_micros();
            let request_generation = context.generation;
            let query_cancelled = Arc::clone(&cancelled);
            let query_key = key.clone();
            let obsolete: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
                requests::request_stale(
                    request_generation,
                    generation.load(Ordering::SeqCst),
                    query_cancelled.lock().is_ok_and(|items| items.contains(&query_key)),
                )
            });
            context.view = context.view.map(|view| view.with_cancellation(Arc::clone(&obsolete)));
            let result = context.answer_cancellable(&method, &params, &*obsolete);
            let query_us = started.elapsed().as_micros();
            let publication_started = Instant::now();
            // The handler cannot invalidate or cancel between this check and the write.
            let publication_guard = publication.lock();
            let result = if publication_guard.is_err() {
                Err((-32603, "publication state unavailable".into()))
            } else if obsolete() {
                Err((-32800, "request cancelled".into()))
            } else {
                result
            };
            let publication_wait_us = publication_started.elapsed().as_micros();
            let cancelled_response = result.as_ref().is_err_and(|error| error.0 == -32800);
            let response_started = Instant::now();
            let response = match result {
                Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                Err((code, message)) => {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
                }
            };
            if let Err(error) = finish_request(&pending, &cancelled, &key, || send(&output, &response)) {
                tracing::error!(%error, method, "failed to send editor query response");
            }
            drop(publication_guard);
            tracing::debug!(
                method,
                generation = context.generation,
                loading_wait_us,
                context_us,
                queue_wait_us,
                query_us,
                publication_wait_us,
                response_us = response_started.elapsed().as_micros(),
                elapsed_us = received.elapsed().as_micros(),
                cancelled = cancelled_response,
                "completed editor query"
            );
        }, move || {
            let response = json!({"jsonrpc":"2.0","id":cancelled_id,
                "error":{"code":-32800,"message":"queued request cancelled"}});
            if let Err(error) = finish_request(&cancelled_pending, &cancelled_items, &cancelled_key,
                || send(&cancelled_output, &response)) {
                tracing::error!(%error, method = cancelled_method, "failed to send query cancellation");
            }
            tracing::debug!(method = cancelled_method, loading_wait_us,
                elapsed_us = received.elapsed().as_micros(), "cancelled queued editor query");
        });
        if !accepted {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&rejected_key);
            }
            tracing::warn!(id=%rejected_id, "editor query queue full");
            self.error(
                &rejected_id,
                -32800,
                "editor query queue is full; retry request",
            )?;
        }
        Ok(())
    }

    /// Capture all query inputs before a worker starts using the immutable generation.
    fn query_context(&self) -> QueryContext {
        QueryContext {
            view: self.ide.clone(),
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
            markdown: self.client.presentation.markdown,
            commands: self.client.presentation.commands,
            virtual_documents: self.client.presentation.declaration_documents,
            overlays: self
                .documents
                .overlays()
                .map(|(path, text)| (path.clone(), Arc::<str>::from(text)))
                .collect(),
            completions: self.completions.clone(),
        }
    }

    pub(super) fn poll_deferred(&mut self) -> Result<(), LspError> {
        for query in std::mem::take(&mut self.deferred) {
            let DeferredQuery {
                id,
                method,
                params,
                generation,
                received,
            } = query;
            let key = id.to_string();
            let cancelled = self
                .cancelled
                .lock()
                .is_ok_and(|items| items.contains(&key));
            let stale = generation != self.generation.load(Ordering::SeqCst);
            if self.loading && !cancelled && !stale && !self.session.shutdown {
                self.deferred.push_back(DeferredQuery {
                    id,
                    method,
                    params,
                    generation,
                    received,
                });
                continue;
            }
            if let Ok(mut items) = self.pending.lock() {
                items.remove(&key);
            }
            if let Ok(mut items) = self.cancelled.lock() {
                items.remove(&key);
            }
            if cancelled || stale || self.session.shutdown {
                self.error(&id, -32800, "loading query cancelled")?;
                tracing::debug!(
                    method,
                    generation,
                    elapsed_us = received.elapsed().as_micros(),
                    "cancelled deferred editor query"
                );
            } else {
                self.enqueue_query(
                    id,
                    &method,
                    &params,
                    received,
                    received.elapsed().as_micros(),
                )?;
            }
        }
        Ok(())
    }
}

impl QueryContext {
    /// Warm and answer only while the captured generation remains current.
    fn answer_cancellable(
        &self,
        method: &str,
        params: &Value,
        obsolete: &dyn Fn() -> bool,
    ) -> QueryResult {
        if obsolete()
            || self
                .view
                .as_ref()
                .is_some_and(|view| view.analysis.try_warm_semantics(obsolete).is_err())
        {
            Err((-32800, "request cancelled".into()))
        } else {
            self.answer(method, params)
        }
    }

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
        // Selected dependency snapshots have no root FileId. Resolve their exact URIs
        // without consulting a filesystem that may have changed since this generation.
        if self.selected_document(uri).is_some() {
            return None;
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
        if let Some(text) = self.selected_document(uri) {
            return Some(text);
        }
        // Keep canonical path aliases available at the protocol boundary.
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

    /// Exact document identity is supplied by the immutable project snapshot.
    fn selected_document(&self, uri: &str) -> Option<String> {
        if let Some(owner) = presentation::virtual_owner(uri) {
            return self.declaration_text(&owner);
        }
        let loaded = self.loaded.as_ref()?;
        let metadata = self.metadata.as_ref()?;
        for selection in &metadata.scripts {
            if let Some((candidate, text)) =
                presentation::external_source(metadata, loaded, &selection.script, &self.overlays)
                && path_to_uri(&candidate) == uri
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
