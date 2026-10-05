//! One session state and message dispatch for the language server.
use super::{
    LspError,
    protocol::{Output, negotiate_encoding, parse_range, send, uri_to_path},
};
use folio_build::{ProjectAnalysis, ProjectAnalysisView};
use folio_ide::{Documents, PositionEncoding};
use folio_project_model::Metadata;
use folio_project_resolve::{LoadedProject, io::FolioHome};
use folio_source::FileId;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

mod diagnostics;
mod editor;
mod loading;
mod overlays;
mod presentation;
mod project;
mod requests;
mod scheduling;
mod settings;

/// Session inputs and publication state shared by handlers and navigation workers.
pub(super) struct Server {
    cwd: PathBuf,
    home: FolioHome,
    manifest_path: Option<PathBuf>,
    output: Output,
    documents: Documents,
    projected_inputs: Option<Arc<LoadedProject>>,
    loader: Option<loading::Loader>,
    loading: bool,
    reload: scheduling::ReloadSchedule,
    status_supported: bool,
    semantic_tokens_refresh: bool,
    queries: scheduling::QueryPool,
    completions: editor::CompletionCache,
    deferred: VecDeque<editor::DeferredQuery>,
    view: Option<Arc<ProjectAnalysisView>>,
    ide: Option<folio_ide::IdeSnapshot>,
    metadata: Option<Arc<Metadata>>,
    paths: BTreeMap<PathBuf, FileId>,
    published: BTreeSet<String>,
    diagnostics: BTreeMap<String, Vec<Value>>,
    dynamic_watches: bool,
    client_ready: bool,
    watchers: Option<Value>,
    project_message: Option<String>,
    last_error: Option<String>,
    encoding: PositionEncoding,
    generation: Arc<AtomicU64>,
    publication: Arc<Mutex<()>>,
    cancelled: Arc<Mutex<BTreeSet<String>>>,
    pending: Arc<Mutex<BTreeSet<String>>>,
    initialized: bool,
    shutdown: bool,
    editor_settings: settings::EditorSettings,
    markdown: bool,
    client_commands: bool,
    declaration_documents: bool,
    code_lens_refresh: bool,
    inlay_hint_refresh: bool,
}

impl Server {
    pub(super) fn new(
        cwd: PathBuf,
        manifest_path: Option<PathBuf>,
        output: Output,
        home: FolioHome,
    ) -> Self {
        Self {
            cwd,
            home,
            manifest_path,
            output,
            documents: Documents::default(),
            projected_inputs: None,
            loader: None,
            loading: false,
            reload: scheduling::ReloadSchedule::default(),
            status_supported: false,
            semantic_tokens_refresh: false,
            queries: scheduling::QueryPool::new(),
            completions: editor::CompletionCache::default(),
            deferred: VecDeque::new(),
            view: None,
            ide: None,
            metadata: None,
            paths: BTreeMap::new(),
            published: BTreeSet::new(),
            diagnostics: BTreeMap::new(),
            dynamic_watches: false,
            client_ready: false,
            watchers: None,
            project_message: None,
            last_error: None,
            encoding: PositionEncoding::Utf16,
            generation: Arc::new(AtomicU64::new(0)),
            publication: Arc::new(Mutex::new(())),
            cancelled: Arc::new(Mutex::new(BTreeSet::new())),
            pending: Arc::new(Mutex::new(BTreeSet::new())),
            initialized: false,
            shutdown: false,
            editor_settings: settings::EditorSettings::default(),
            markdown: false,
            client_commands: false,
            declaration_documents: false,
            code_lens_refresh: false,
            inlay_hint_refresh: false,
        }
    }

    pub(super) fn handle(&mut self, message: Value) -> Result<bool, LspError> {
        let started = Instant::now();
        if message.get("method").is_none() {
            if let Some(error) = message.get("error") {
                tracing::warn!(id = ?message.get("id"), %error, "LSP client request failed");
            }
            return Ok(false);
        }
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        let params = &message["params"];
        tracing::debug!(method, "received LSP message");
        match method {
            "initialize" => {
                self.encoding = negotiate_encoding(params);
                self.editor_settings =
                    settings::EditorSettings::read(&params["initializationOptions"]);
                self.client_commands = params["initializationOptions"]["folio"]["clientCommands"]
                    .as_bool()
                    == Some(true);
                self.declaration_documents =
                    params["initializationOptions"]["folio"]["declarationDocuments"].as_bool()
                        == Some(true);
                self.markdown = params["capabilities"]["textDocument"]["hover"]["contentFormat"]
                    .as_array()
                    .is_some_and(|formats| {
                        formats
                            .iter()
                            .any(|format| format.as_str() == Some("markdown"))
                    });
                self.code_lens_refresh =
                    params["capabilities"]["workspace"]["codeLens"]["refreshSupport"].as_bool()
                        == Some(true);
                self.inlay_hint_refresh =
                    params["capabilities"]["workspace"]["inlayHint"]["refreshSupport"].as_bool()
                        == Some(true);
                let watches = &params["capabilities"]["workspace"]["didChangeWatchedFiles"];
                self.dynamic_watches = watches["dynamicRegistration"].as_bool() == Some(true)
                    && watches["relativePatternSupport"].as_bool() == Some(true);
                if self.manifest_path.is_none() {
                    if let Some(uri) = params["rootUri"].as_str() {
                        self.cwd = uri_to_path(uri).unwrap_or_else(|| self.cwd.clone());
                    } else if let Some(path) = params["rootPath"].as_str() {
                        self.cwd = PathBuf::from(path);
                    }
                }
                self.initialized = true;
                self.status_supported =
                    params["initializationOptions"]["folio"]["status"].as_bool() == Some(true);
                self.semantic_tokens_refresh = params["capabilities"]["workspace"]["semanticTokens"]
                    ["refreshSupport"]
                    .as_bool() == Some(true);
                let name = match self.encoding {
                    PositionEncoding::Utf8 => "utf-8",
                    PositionEncoding::Utf16 => "utf-16",
                };
                if let Some(id) = id {
                    send(
                        &self.output,
                        &json!({"jsonrpc":"2.0","id":id,"result":{"capabilities":{"positionEncoding":name,"textDocumentSync":{"openClose":true,"change":2,"save":{"includeText":false}},"hoverProvider":true,"definitionProvider":true,"declarationProvider":true,"implementationProvider":true,"referencesProvider":true,"documentHighlightProvider":true,"workspaceSymbolProvider":true,"completionProvider":{"triggerCharacters":[".","("],"resolveProvider":true},"renameProvider":{"prepareProvider":true},"codeLensProvider":{"resolveProvider":true},"inlayHintProvider":true,"documentSymbolProvider":true,"signatureHelpProvider":{"triggerCharacters":["(",","],"retriggerCharacters":[","]},"semanticTokensProvider":{"legend":{"tokenTypes":["class","type","namespace","function","method","event","property","variable","parameter"],"tokenModifiers":["declaration","readonly"]},"full":true},"documentFormattingProvider":true},"serverInfo":{"name":"Folio","version":env!("CARGO_PKG_VERSION")}}}),
                    )?;
                }
                tracing::info!(encoding = name, "LSP initialized");
            }
            "initialized" => {
                self.client_ready = true;
                self.reload_report(true)?;
            }
            "shutdown" => {
                self.shutdown = true;
                self.advance_generation()?;
                if let Some(id) = id {
                    self.reply(id, Value::Null)?;
                }
            }
            "exit" => return Ok(true),
            "$/cancelRequest" => {
                let publication = self
                    .publication
                    .lock()
                    .map_err(|_| LspError::Protocol("publication lock poisoned".into()))?;
                if let Some(id) = params.get("id") {
                    let key = id.to_string();
                    let pending = self
                        .pending
                        .lock()
                        .map_err(|_| LspError::Protocol("pending lock poisoned".into()))?;
                    if pending.contains(&key) {
                        let mut cancelled = self
                            .cancelled
                            .lock()
                            .map_err(|_| LspError::Protocol("cancellation lock poisoned".into()))?;
                        cancelled.insert(key);
                        tracing::debug!(id = %id, "LSP request cancelled");
                    }
                }
                drop(publication);
                if let Some(id) = params.get("id") {
                    self.queries.cancel(&id.to_string());
                }
            }
            "textDocument/didOpen" => {
                if let (Some(uri), Some(version), Some(text)) = (
                    params["textDocument"]["uri"].as_str(),
                    params["textDocument"]["version"].as_i64(),
                    params["textDocument"]["text"].as_str(),
                ) && let (Some(path), Ok(version)) = (uri_to_path(uri), i32::try_from(version))
                {
                    match self.documents.open(&path, version, Arc::from(text)) {
                        Ok(()) => self.update_document(&path)?,
                        Err(error) => tracing::warn!(?error, uri, "ignored document open"),
                    }
                }
            }
            "textDocument/didChange" => {
                if let (Some(uri), Some(version), Some(changes)) = (
                    params["textDocument"]["uri"].as_str(),
                    params["textDocument"]["version"].as_i64(),
                    params["contentChanges"].as_array(),
                ) && let (Some(path), Ok(version)) = (uri_to_path(uri), i32::try_from(version))
                {
                    let parsed = changes
                        .iter()
                        .filter_map(|change| {
                            let text = change["text"].as_str()?.to_owned();
                            let range = if change.get("range").is_some() {
                                Some(parse_range(&change["range"])?)
                            } else {
                                None
                            };
                            Some((range, text))
                        })
                        .collect::<Vec<_>>();
                    if parsed.len() == changes.len() {
                        match self
                            .documents
                            .change(&path, version, &parsed, self.encoding)
                        {
                            Ok(()) => self.update_document(&path)?,
                            Err(error) => {
                                tracing::warn!(?error, uri, "ignored document change")
                            }
                        }
                    }
                }
            }
            "textDocument/didClose" => {
                if let Some(uri) = params["textDocument"]["uri"].as_str()
                    && let Some(path) = uri_to_path(uri)
                {
                    if let Err(error) = self.documents.close(&path) {
                        tracing::warn!(?error, uri, "ignored document close");
                    } else {
                        self.close_document(&path)?;
                    }
                }
            }
            "textDocument/didSave" => {
                if self.file_event_relevant(&params["textDocument"]["uri"]) {
                    self.reload_report(true)?;
                }
            }
            "workspace/didChangeWatchedFiles" => {
                if params["changes"].as_array().is_some_and(|changes| {
                    changes
                        .iter()
                        .any(|change| self.file_event_relevant(&change["uri"]))
                }) {
                    self.reload_report(true)?;
                }
            }
            "workspace/didChangeConfiguration" => {
                self.editor_settings = settings::EditorSettings::read(&params["settings"]);
                if self.loading {
                    self.reload_report(false)?;
                } else {
                    self.advance_generation()?;
                }
                self.refresh_editor()?;
                tracing::debug!(?self.editor_settings, "updated editor presentation settings");
            }
            "textDocument/hover"
            | "textDocument/definition"
            | "textDocument/declaration"
            | "textDocument/implementation"
            | "textDocument/references"
            | "textDocument/documentHighlight"
            | "workspace/symbol"
            | "textDocument/completion"
            | "completionItem/resolve"
            | "textDocument/codeLens"
            | "codeLens/resolve"
            | "textDocument/inlayHint"
            | "textDocument/prepareRename"
            | "textDocument/rename"
            | "folio/declarationContent" => {
                if let Some(id) = id {
                    self.editor_request(id, method, params, started)?;
                }
            }
            "textDocument/signatureHelp"
            | "textDocument/documentSymbol"
            | "textDocument/semanticTokens/full" => {
                if let Some(id) = id {
                    self.editor_request(id, method, params, started)?;
                }
            }
            "textDocument/formatting" => {
                if let Some(id) = id {
                    self.format_document(id, params)?;
                }
            }
            _ if id.is_some() => self.error(id.unwrap(), -32601, "method not found")?,
            _ => {}
        }
        tracing::debug!(
            method,
            elapsed_us = started.elapsed().as_micros(),
            "handled LSP message"
        );
        Ok(false)
    }

    fn reply(&self, id: Value, result: Value) -> Result<(), LspError> {
        send(
            &self.output,
            &json!({"jsonrpc":"2.0","id":id,"result":result}),
        )
    }

    /// Invalidation and final worker publication share a barrier to close the stale-send race.
    fn advance_generation(&self) -> Result<u64, LspError> {
        let publication = self
            .publication
            .lock()
            .map_err(|_| LspError::Protocol("publication lock poisoned".into()))?;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        drop(publication);
        self.queries.retain_generation(generation);
        Ok(generation)
    }

    fn error(&self, id: Value, code: i32, message: &str) -> Result<(), LspError> {
        send(
            &self.output,
            &json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}),
        )
    }
}
