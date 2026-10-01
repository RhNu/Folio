//! LSP 3.17 stdio adapter over the project resolver and shared semantic analysis.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use folio_build::{ProjectAnalysis, ProjectAnalysisView};
use folio_diagnostics::Severity;
use folio_format::format_source;
use folio_ide::{Documents, Position, PositionEncoding, PositionIndex, Range};
use folio_lint::{LintConfig, lint_script};
use folio_papyrus::PapyrusDialect;
use folio_project_model::{DependencyKind, LoadedCarrier, Metadata, SourceFile, SourceId};
use folio_project_resolve::{
    LoadedProject, discover, io::LoadedSourceInput, load_and_resolve, resolve,
};
use folio_source::{FileId, TextRange};
use serde_json::{Value, json};

#[derive(Debug)]
pub enum LspError {
    Io(io::Error),
    Project(String),
    Protocol(String),
}

impl std::fmt::Display for LspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "LSP I/O: {error}"),
            Self::Project(error) => write!(f, "LSP project: {error}"),
            Self::Protocol(error) => write!(f, "LSP protocol: {error}"),
        }
    }
}
impl std::error::Error for LspError {}
impl From<io::Error> for LspError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

type Output = Arc<Mutex<io::Stdout>>;

fn send(output: &Output, value: &Value) -> Result<(), LspError> {
    let bytes = serde_json::to_vec(value).map_err(|error| LspError::Protocol(error.to_string()))?;
    let mut stream = output
        .lock()
        .map_err(|_| LspError::Protocol("stdout lock poisoned".into()))?;
    write!(stream, "Content-Length: {}\r\n\r\n", bytes.len())?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

fn read_message(reader: &mut impl BufRead) -> Result<Option<Value>, LspError> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return if length.is_none() {
                Ok(None)
            } else {
                Err(LspError::Protocol("incomplete LSP header".into()))
            };
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("Content-Length")
        {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| LspError::Protocol("invalid Content-Length".into()))?,
            );
        }
    }
    let length = length.ok_or_else(|| LspError::Protocol("missing Content-Length".into()))?;
    if length > 16 * 1024 * 1024 {
        return Err(LspError::Protocol("LSP message exceeds 16 MiB".into()));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| LspError::Protocol(error.to_string()))
}

/// Runs a single LSP session. The caller initializes a tracing subscriber that writes to stderr.
pub fn serve_stdio(manifest_path: Option<&Path>) -> Result<(), LspError> {
    let cwd = std::env::current_dir()?;
    let output = Arc::new(Mutex::new(io::stdout()));
    let input = io::stdin();
    let mut reader = io::BufReader::new(input.lock());
    let mut server = Server::new(cwd, manifest_path.map(Path::to_path_buf), output);
    while let Some(message) = read_message(&mut reader)? {
        if server.handle(message)? {
            break;
        }
    }
    Ok(())
}

struct Server {
    cwd: PathBuf,
    manifest_path: Option<PathBuf>,
    output: Output,
    documents: Documents,
    project: ProjectAnalysis,
    disk_project: Option<(LoadedProject, Metadata)>,
    view: Option<Arc<ProjectAnalysisView>>,
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
    cancelled: Arc<Mutex<BTreeSet<String>>>,
    pending: Arc<Mutex<BTreeSet<String>>>,
    initialized: bool,
    shutdown: bool,
}

impl Server {
    fn new(cwd: PathBuf, manifest_path: Option<PathBuf>, output: Output) -> Self {
        Self {
            cwd,
            manifest_path,
            output,
            documents: Documents::default(),
            project: ProjectAnalysis::new(),
            disk_project: None,
            view: None,
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
            cancelled: Arc::new(Mutex::new(BTreeSet::new())),
            pending: Arc::new(Mutex::new(BTreeSet::new())),
            initialized: false,
            shutdown: false,
        }
    }

    fn handle(&mut self, message: Value) -> Result<bool, LspError> {
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
                let name = match self.encoding {
                    PositionEncoding::Utf8 => "utf-8",
                    PositionEncoding::Utf16 => "utf-16",
                };
                if let Some(id) = id {
                    send(
                        &self.output,
                        &json!({"jsonrpc":"2.0","id":id,"result":{"capabilities":{"positionEncoding":name,"textDocumentSync":{"openClose":true,"change":2,"save":{"includeText":false}},"hoverProvider":true,"definitionProvider":true,"declarationProvider":true,"documentSymbolProvider":true,"signatureHelpProvider":{"triggerCharacters":["(",","],"retriggerCharacters":[","]},"semanticTokensProvider":{"legend":{"tokenTypes":["class","type","namespace","function","method","event","property","variable","parameter"],"tokenModifiers":["declaration","readonly"]},"full":true},"documentFormattingProvider":true},"serverInfo":{"name":"Folio","version":env!("CARGO_PKG_VERSION")}}}),
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
                self.generation.fetch_add(1, Ordering::SeqCst);
                if let Some(id) = id {
                    self.reply(id, Value::Null)?;
                }
            }
            "exit" => return Ok(true),
            "$/cancelRequest" => {
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
            "textDocument/didSave" => self.reload_report(true)?,
            "workspace/didChangeWatchedFiles" => self.reload_report(true)?,
            "textDocument/hover" | "textDocument/definition" | "textDocument/declaration" => {
                if let Some(id) = id {
                    self.navigation(id, method, params)?;
                }
            }
            "textDocument/signatureHelp"
            | "textDocument/documentSymbol"
            | "textDocument/semanticTokens/full" => {
                if let Some(id) = id {
                    self.symbol_request(id, method, params)?;
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
    fn error(&self, id: Value, code: i32, message: &str) -> Result<(), LspError> {
        send(
            &self.output,
            &json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}),
        )
    }

    fn reload_report(&mut self, refresh_disk: bool) -> Result<(), LspError> {
        if let Err(error) = self.reload(refresh_disk) {
            tracing::error!(%error, "LSP project analysis unavailable");
            self.clear_view()?;
            self.disk_project = None;
            let message = error.to_string();
            if self.last_error.as_ref() != Some(&message) {
                send(
                    &self.output,
                    &json!({"jsonrpc":"2.0","method":"window/showMessage","params":{"type":1,"message":message}}),
                )?;
                self.last_error = Some(message);
            }
        } else {
            self.last_error = None;
        }
        Ok(())
    }

    fn clear_view(&mut self) -> Result<(), LspError> {
        self.view = None;
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
        Ok(())
    }

    /// Buffer lifecycle alone does not change semantic inputs or invalidate requests.
    fn update_document(&mut self, path: &Path) -> Result<(), LspError> {
        let unchanged = self
            .paths
            .get(path)
            .and_then(|file| self.view.as_ref()?.analysis.text(*file))
            .is_some_and(|text| Some(text) == self.documents.text(&path.to_path_buf()));
        if unchanged {
            let uri = path_to_uri(path);
            if let Some(diagnostics) = self.diagnostics.get(&uri) {
                send(
                    &self.output,
                    &json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"version":self.documents.version(&path.to_path_buf()),"diagnostics":diagnostics}}),
                )?;
            }
            tracing::debug!(path = %path.display(), "reused unchanged LSP project view");
            return Ok(());
        }
        self.reload_report(false)
    }

    /// Re-read the closed file so discarding a buffer restores even unwatched disk edits.
    fn close_document(&mut self, path: &Path) -> Result<(), LspError> {
        if let Some((loaded, _)) = self.disk_project.as_mut()
            && let Some(source) = loaded
                .source_inputs
                .iter_mut()
                .find(|source| source.canonical_path == path)
            && let Ok(text) = std::fs::read_to_string(path)
        {
            source.text = Arc::from(text);
            self.documents
                .disk_update(path.to_path_buf(), Arc::clone(&source.text));
            return self.update_document(path);
        }
        // New unsaved files and removed files require rebuilding provider selection.
        self.reload_report(true)
    }

    fn reload(&mut self, refresh_disk: bool) -> Result<(), LspError> {
        if !self.initialized || self.shutdown {
            return Ok(());
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let started = Instant::now();
        if refresh_disk || self.disk_project.is_none() {
            match load_and_resolve(&self.cwd, self.manifest_path.as_deref()) {
                Ok(project) => self.disk_project = Some(project),
                Err(error) => {
                    tracing::error!(%error, "LSP project reload failed");
                    return Err(LspError::Project(error.to_string()));
                }
            }
        }
        let (mut loaded, mut metadata) = self
            .disk_project
            .as_ref()
            .expect("loaded disk project")
            .clone();
        if refresh_disk {
            self.register_file_watches(generation)?;
        }
        tracing::debug!(
            generation,
            elapsed_us = started.elapsed().as_micros(),
            phase = "load",
            "LSP project phase complete"
        );
        let started = Instant::now();
        let mut disk_paths = BTreeSet::new();
        for source in &loaded.source_inputs {
            disk_paths.insert(source.canonical_path.clone());
            self.documents
                .disk_update(source.canonical_path.clone(), Arc::clone(&source.text));
        }
        self.documents.retain_disk_paths(&disk_paths);
        self.add_unsaved_sources(&mut loaded, &disk_paths)?;
        if loaded.source_inputs.len() != disk_paths.len() {
            metadata = resolve(&loaded.root_key, &loaded.packages)
                .map_err(|error| LspError::Project(error.to_string()))?;
        }
        for source in &mut loaded.source_inputs {
            if let Some(text) = self.documents.text(&source.canonical_path) {
                source.text = Arc::from(text);
            }
        }
        let view = Arc::new(
            self.project
                .sync_project(&loaded, &metadata)
                .map_err(|error| LspError::Project(error.to_string()))?,
        );
        tracing::debug!(
            generation,
            elapsed_us = started.elapsed().as_micros(),
            phase = "projection",
            "LSP project phase complete"
        );
        self.metadata = Some(Arc::new(metadata));
        self.paths = view
            .sources
            .iter()
            .map(|(&file, source)| (source.canonical_path.clone(), file))
            .collect();
        tracing::info!(
            generation,
            files = self.paths.len(),
            "LSP project view updated"
        );
        let rules = loaded
            .packages
            .iter()
            .find(|package| package.source_key == loaded.root_key)
            .and_then(|package| package.manifest())
            .ok_or_else(|| LspError::Project("root package manifest is missing".into()))?
            .lint_rules
            .clone();
        let lint =
            LintConfig::from_rules(&rules).map_err(|error| LspError::Project(error.to_string()))?;
        self.publish(view.clone(), generation, &loaded.root_key, &lint)?;
        self.view = Some(view);
        Ok(())
    }

    /// The resolver owns dependency paths, including carriers outside the editor folder.
    fn register_file_watches(&mut self, generation: u64) -> Result<(), LspError> {
        // Capability registration is allowed only after the initialized notification.
        if !self.dynamic_watches || !self.client_ready {
            return Ok(());
        }
        let Some((loaded, _)) = &self.disk_project else {
            return Ok(());
        };
        let mut patterns = BTreeSet::new();
        for package in &loaded.packages {
            let path = Path::new(&package.source_key);
            match &package.carrier {
                LoadedCarrier::Manifest(manifest) => {
                    if let Some(base) = path.parent() {
                        patterns.insert((path_to_uri(base), "folio.toml"));
                        patterns
                            .insert((path_to_uri(&base.join(&manifest.source_path.value)), "**/*"));
                    }
                }
                LoadedCarrier::Declarations {
                    kind: DependencyKind::Builtin,
                    ..
                } => {}
                LoadedCarrier::Declarations {
                    kind: DependencyKind::Psc | DependencyKind::Pex,
                    ..
                } => {
                    patterns.insert((path_to_uri(path), "**/*"));
                }
                LoadedCarrier::Declarations { .. } => {
                    if let Some(base) = path.parent() {
                        patterns.insert((path_to_uri(base), "*"));
                    }
                }
            }
        }
        let watchers = json!(
            patterns
                .into_iter()
                .map(|(base, pattern)| {
                    json!({"globPattern":{"baseUri":base,"pattern":pattern},"kind":7})
                })
                .collect::<Vec<_>>()
        );
        if self.watchers.as_ref() == Some(&watchers) {
            return Ok(());
        }
        if self.watchers.is_some() {
            send(
                &self.output,
                &json!({"jsonrpc":"2.0","id":format!("folio/watch/unregister/{generation}"),"method":"client/unregisterCapability","params":{"unregisterations":[{"id":"folio-project-files","method":"workspace/didChangeWatchedFiles"}]}}),
            )?;
        }
        tracing::debug!(
            count = watchers.as_array().map_or(0, Vec::len),
            "registered LSP project file watches"
        );
        send(
            &self.output,
            &json!({"jsonrpc":"2.0","id":format!("folio/watch/register/{generation}"),"method":"client/registerCapability","params":{"registrations":[{"id":"folio-project-files","method":"workspace/didChangeWatchedFiles","registerOptions":{"watchers":watchers}}]}}),
        )?;
        self.watchers = Some(watchers);
        Ok(())
    }

    /// Projects new open files through the same root manifest and resolver graph as disk files.
    fn add_unsaved_sources(
        &self,
        loaded: &mut folio_project_resolve::LoadedProject,
        disk_paths: &BTreeSet<PathBuf>,
    ) -> Result<(), LspError> {
        let manifest_path = discover(&self.cwd, self.manifest_path.as_deref())
            .map_err(|error| LspError::Project(error.to_string()))?;
        let base = manifest_path
            .parent()
            .ok_or_else(|| LspError::Project("manifest has no parent".into()))?;
        let Some(package) = loaded
            .packages
            .iter_mut()
            .find(|package| package.source_key == loaded.root_key)
        else {
            return Err(LspError::Project("root package is missing".into()));
        };
        let Some(manifest) = package.manifest().cloned() else {
            return Err(LspError::Project("root manifest is missing".into()));
        };
        for (path, text) in self.documents.overlays() {
            if disk_paths.contains(path) {
                continue;
            }
            let display_path = std::fs::canonicalize(base.join(&manifest.source_path.value))
                .ok()
                .and_then(|source_root| path.strip_prefix(source_root).ok().map(Path::to_path_buf))
                .map(|relative| {
                    Path::new(&manifest.source_path.value)
                        .join(relative)
                        .to_string_lossy()
                        .replace('\\', "/")
                });
            let extension_allowed = path.extension().is_some_and(|extension| {
                manifest
                    .extensions
                    .iter()
                    .any(|item| extension.eq_ignore_ascii_case(item))
            });
            if !extension_allowed {
                continue;
            }
            let Some(display_path) = display_path else {
                continue;
            };
            let candidate = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| {
                    LspError::Project(format!("invalid script name: {}", path.display()))
                })?;
            let portable = folio_project_resolve::io::relative_portable(base, path)
                .map_err(|error| LspError::Project(error.to_string()))?;
            tracing::debug!(path = %path.display(), "added unsaved project source");
            package.source_files.push(SourceFile {
                path: portable.clone(),
                display_path: display_path.clone(),
                script_candidate: candidate.to_owned(),
            });
            loaded.source_inputs.push(LoadedSourceInput {
                package_key: loaded.root_key.clone(),
                canonical_path: path.clone(),
                display_path,
                script_candidate: candidate.to_owned(),
                text: Arc::from(text),
            });
        }
        Ok(())
    }

    fn publish(
        &mut self,
        view: Arc<ProjectAnalysisView>,
        generation: u64,
        root_key: &str,
        lint: &LintConfig,
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
        let mut diagnostics = view.diagnostics();
        for (&file, source) in &view.sources {
            if source.package_key == root_key
                && let Some(script) = view.analysis.hir(file)
            {
                diagnostics.extend(lint_script(&script, lint));
            }
        }
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

    /// Returns edits only for the current buffer generation and negotiated encoding.
    fn format_document(&self, id: Value, params: &Value) -> Result<(), LspError> {
        let Some(uri) = params["textDocument"]["uri"].as_str() else {
            return self.error(id, -32602, "missing document URI");
        };
        let Some(path) = uri_to_path(uri) else {
            return self.error(id, -32602, "invalid document URI");
        };
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("psc"))
        {
            return self.reply(id, json!([]));
        }
        let Some(source) = self.documents.text(&path) else {
            return self.reply(id, json!([]));
        };
        let generation = self.generation.load(Ordering::SeqCst);
        let version = self.documents.version(&path);
        let formatted = match format_source(source, PapyrusDialect::Skyrim) {
            Ok(formatted) => formatted,
            Err(error) => {
                tracing::debug!(?error, uri, "document formatting rejected");
                return self.error(id, -32001, "document cannot be safely formatted");
            }
        };
        if generation != self.generation.load(Ordering::SeqCst)
            || version != self.documents.version(&path)
        {
            return self.error(id, -32800, "request cancelled");
        }
        if formatted == source {
            return self.reply(id, json!([]));
        }
        let range = folio_ide::range(
            source,
            TextRange {
                start: 0,
                end: source.len(),
            },
            self.encoding,
        )
        .ok_or_else(|| LspError::Protocol("invalid formatting range".into()))?;
        tracing::debug!(uri, version, "document formatting edits computed");
        self.reply(id, json!([{"range":range_json(range),"newText":formatted}]))
    }

    fn navigation(&self, id: Value, method: &str, params: &Value) -> Result<(), LspError> {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
        let file = uri_to_path(uri).and_then(|path| self.paths.get(&path).copied());
        let at = parse_position(&params["position"]);
        let view = self.view.clone();
        let metadata = self.metadata.clone();
        let generation = self.generation.load(Ordering::SeqCst);
        let active = Arc::clone(&self.generation);
        let cancelled = Arc::clone(&self.cancelled);
        let pending = Arc::clone(&self.pending);
        let output = Arc::clone(&self.output);
        let encoding = self.encoding;
        let hover = method == "textDocument/hover";
        pending
            .lock()
            .map_err(|_| LspError::Protocol("pending lock poisoned".into()))?
            .insert(id.to_string());
        thread::spawn(move || {
            let key = id.to_string();
            let is_cancelled = || {
                request_stale(
                    generation,
                    active.load(Ordering::SeqCst),
                    cancelled.lock().is_ok_and(|set| set.contains(&key)),
                )
            };
            if is_cancelled() {
                let _ = send(
                    &output,
                    &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32800,"message":"request cancelled"}}),
                );
                if let Ok(mut set) = pending.lock() {
                    set.remove(&key);
                    if let Ok(mut cancelled) = cancelled.lock() {
                        cancelled.remove(&key);
                    }
                }
                return;
            }
            let result = (|| {
                let view = view.as_ref()?;
                if view.analysis.try_warm_semantics(is_cancelled).is_err() {
                    tracing::debug!(request = %key, "stopped obsolete semantic query");
                    return None;
                }
                let file = file?;
                let text = view.analysis.text(file)?;
                let byte = folio_ide::offset(text, at?, encoding)?;
                if hover {
                    let mut item = folio_ide::hover(view, file, byte)?;
                    if let (Some(metadata), Some(owner)) = (metadata.as_deref(), item.owner_script.as_deref())
                        && let Some(origin) = symbol_origin(metadata, owner)
                    {
                        item.content.push_str("\nSource: ");
                        item.content.push_str(&origin);
                    }
                    tracing::debug!(request = %key, file = ?file, "resolved hover symbol");
                    Some(json!({"contents":{"kind":"plaintext","value":item.content},"range":range_json(folio_ide::range(text, item.span.range, encoding)?)}))
                } else {
                    let span = folio_ide::source_declaration(view, file, byte)?;
                    let source = view.sources.get(&span.file)?;
                    let target = view.analysis.text(span.file)?;
                    tracing::debug!(request = %key, source_file = ?file, target_file = ?span.file, "resolved source declaration");
                    Some(json!({"uri":path_to_uri(&source.canonical_path),"range":range_json(folio_ide::range(target, span.range, encoding)?)}))
                }
            })().unwrap_or(Value::Null);
            let response = if is_cancelled() {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32800,"message":"request cancelled"}})
            } else {
                json!({"jsonrpc":"2.0","id":id,"result":result})
            };
            if let Err(error) = send(&output, &response) {
                tracing::error!(%error, "failed to send LSP navigation response");
            }
            if let Ok(mut set) = pending.lock() {
                set.remove(&key);
                if let Ok(mut cancelled) = cancelled.lock() {
                    cancelled.remove(&key);
                }
            }
        });
        Ok(())
    }

    /// Answers editor symbol requests from one coherent project generation.
    fn symbol_request(&self, id: Value, method: &str, params: &Value) -> Result<(), LspError> {
        let started = Instant::now();
        let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
        let file = uri_to_path(uri).and_then(|path| self.paths.get(&path).copied());
        let Some(view) = &self.view else {
            return self.reply(id, Value::Null);
        };
        let Some(file) = file else {
            return self.reply(id, Value::Null);
        };
        let Some(text) = view.analysis.text(file) else {
            return self.reply(id, Value::Null);
        };
        let result = match method {
            "textDocument/signatureHelp" => {
                let info = parse_position(&params["position"])
                    .and_then(|position| folio_ide::offset(text, position, self.encoding))
                    .and_then(|byte| folio_ide::signature_help(view, file, byte));
                info.map_or(Value::Null, |info| {
                    tracing::debug!(?file, active_parameter = info.active_parameter, "resolved signature help");
                    let parameters = info.parameters.iter().map(|label| json!({"label":label})).collect::<Vec<_>>();
                    json!({"signatures":[{"label":info.label,"parameters":parameters}],"activeSignature":0,"activeParameter":info.active_parameter})
                })
            }
            "textDocument/documentSymbol" => {
                let symbols = folio_ide::document_symbols(view, file);
                let positions = PositionIndex::new(text);
                tracing::debug!(
                    method,
                    elapsed_us = started.elapsed().as_micros(),
                    phase = "symbols",
                    "LSP query phase complete"
                );
                tracing::debug!(?file, count = symbols.len(), "collected document symbols");
                json!(
                    symbols
                        .iter()
                        .filter_map(|item| document_symbol_json(&positions, item, self.encoding))
                        .collect::<Vec<_>>()
                )
            }
            "textDocument/semanticTokens/full" => {
                let tokens = folio_ide::semantic_tokens(view, file);
                tracing::debug!(
                    method,
                    elapsed_us = started.elapsed().as_micros(),
                    phase = "tokens",
                    "LSP query phase complete"
                );
                tracing::debug!(?file, count = tokens.len(), "collected semantic tokens");
                json!({"data":encode_semantic_tokens(text, &tokens, self.encoding)})
            }
            _ => Value::Null,
        };
        self.reply(id, result)
    }
}

/// Encodes sorted identifier spans in the position units negotiated by the client.
fn encode_semantic_tokens(
    text: &str,
    tokens: &[folio_ide::SemanticToken],
    encoding: PositionEncoding,
) -> Vec<u32> {
    let positions = PositionIndex::new(text);
    let mut data = Vec::with_capacity(tokens.len() * 5);
    let mut previous = Position {
        line: 0,
        character: 0,
    };
    for item in tokens {
        let Some(range) = positions.range(item.range, encoding) else {
            continue;
        };
        if range.start.line != range.end.line {
            continue;
        }
        let kind = match item.kind {
            folio_ide::SemanticTokenKind::Class => 0,
            folio_ide::SemanticTokenKind::Type => 1,
            folio_ide::SemanticTokenKind::Namespace => 2,
            folio_ide::SemanticTokenKind::Function => 3,
            folio_ide::SemanticTokenKind::Method => 4,
            folio_ide::SemanticTokenKind::Event => 5,
            folio_ide::SemanticTokenKind::Property => 6,
            folio_ide::SemanticTokenKind::Variable => 7,
            folio_ide::SemanticTokenKind::Parameter => 8,
        };
        let Some(delta_line) = range.start.line.checked_sub(previous.line) else {
            continue;
        };
        let Some(delta_start) = (if delta_line == 0 {
            range.start.character.checked_sub(previous.character)
        } else {
            Some(range.start.character)
        }) else {
            continue;
        };
        let length = range.end.character - range.start.character;
        let modifiers = u32::from(item.declaration) | (u32::from(item.readonly) << 1);
        data.extend([delta_line, delta_start, length, kind, modifiers]);
        previous = range.start;
    }
    data
}

fn document_symbol_json(
    positions: &PositionIndex<'_>,
    item: &folio_ide::DocumentSymbol,
    encoding: PositionEncoding,
) -> Option<Value> {
    let children = item
        .children
        .iter()
        .filter_map(|child| document_symbol_json(positions, child, encoding))
        .collect::<Vec<_>>();
    Some(
        json!({"name":item.name,"detail":item.detail,"kind":item.kind,
        "range":range_json(positions.range(item.range, encoding)?),
        "selectionRange":range_json(positions.range(item.selection_range, encoding)?),
        "children":children}),
    )
}

fn symbol_origin(metadata: &Metadata, script: &str) -> Option<String> {
    let selected = &metadata
        .scripts
        .iter()
        .find(|item| item.script.eq_ignore_ascii_case(script))?
        .selected;
    let dependency_kind = metadata
        .dependencies
        .iter()
        .find(|edge| edge.to == selected.package)
        .map(|edge| edge.kind);
    let kind = match dependency_kind {
        Some(DependencyKind::Package) => "package dependency",
        Some(DependencyKind::Psc) => "PSC dependency",
        Some(DependencyKind::Sdk) => "SDK declaration",
        Some(DependencyKind::Builtin) => "built-in declaration",
        Some(DependencyKind::Pex) => "PEX declaration",
        None => match &selected.package.source {
            SourceId::Project => "project",
            SourceId::Local { .. } => "local dependency",
            SourceId::DeclarationSdk { .. } => "SDK declaration",
            SourceId::BinaryPex { .. } => "PEX declaration",
        },
    };
    let path = selected.source_path.as_deref().or_else(|| {
        selected
            .declaration
            .as_ref()
            .map(|item| item.carrier_path.as_str())
    });
    Some(format!(
        "{} {} ({kind}){}",
        selected.package.name,
        selected.package.version,
        path.map_or(String::new(), |path| format!(" · {path}"))
    ))
}

/// Results tied to an older project generation are never sent as current answers.
fn request_stale(request_generation: u64, current_generation: u64, cancelled: bool) -> bool {
    cancelled || request_generation != current_generation
}

fn negotiate_encoding(params: &Value) -> PositionEncoding {
    let encodings = params["capabilities"]["general"]["positionEncodings"].as_array();
    if encodings.is_some_and(|items| items.iter().any(|item| item.as_str() == Some("utf-8"))) {
        PositionEncoding::Utf8
    } else {
        PositionEncoding::Utf16
    }
}

fn parse_position(value: &Value) -> Option<Position> {
    Some(Position {
        line: u32::try_from(value["line"].as_u64()?).ok()?,
        character: u32::try_from(value["character"].as_u64()?).ok()?,
    })
}
fn parse_range(value: &Value) -> Option<Range> {
    Some(Range {
        start: parse_position(&value["start"])?,
        end: parse_position(&value["end"])?,
    })
}
fn range_json(range: Range) -> Value {
    json!({"start":{"line":range.start.line,"character":range.start.character},"end":{"line":range.end.line,"character":range.end.character}})
}

/// Converts local file URIs without treating percent escapes as source text.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let decoded = percent_decode(encoded)?;
    #[cfg(windows)]
    {
        let path = decoded
            .strip_prefix('/')
            .unwrap_or(&decoded)
            .replace('/', "\\");
        canonical_or_parent(Path::new(&path))
    }
    #[cfg(not(windows))]
    {
        canonical_or_parent(Path::new(&decoded))
    }
}

fn canonical_or_parent(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok().or_else(|| {
        let parent = std::fs::canonicalize(path.parent()?).ok()?;
        Some(parent.join(path.file_name()?))
    })
}

fn path_to_uri(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    let path = path.strip_prefix("//?/").unwrap_or(&path).to_owned();
    let prefix = if path.starts_with('/') {
        "file://"
    } else {
        "file:///"
    };
    let mut result = prefix.to_owned();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}
fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let source = text.as_bytes();
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            let hex = source.get(index + 1..index + 3)?;
            bytes.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            index += 3;
        } else {
            bytes.push(source[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::{encode_semantic_tokens, negotiate_encoding, request_stale};
    use folio_ide::PositionEncoding;
    use folio_source::TextRange;
    use serde_json::json;

    #[test]
    fn stale_or_cancelled_navigation_is_discarded() {
        assert!(!request_stale(3, 3, false));
        assert!(request_stale(3, 4, false));
        assert!(request_stale(3, 3, true));
    }

    #[test]
    fn utf16_is_default_and_utf8_is_negotiated() {
        assert_eq!(negotiate_encoding(&json!({})), PositionEncoding::Utf16);
        assert_eq!(
            negotiate_encoding(
                &json!({"capabilities":{"general":{"positionEncodings":["utf-8","utf-16"]}}})
            ),
            PositionEncoding::Utf8
        );
    }

    #[test]
    fn semantic_token_deltas_use_negotiated_character_units() {
        use folio_ide::{SemanticToken, SemanticTokenKind};

        let text = "🦊 Foo\nBar";
        let tokens = [
            SemanticToken {
                range: TextRange { start: 5, end: 8 },
                kind: SemanticTokenKind::Class,
                declaration: true,
                readonly: false,
            },
            SemanticToken {
                range: TextRange { start: 9, end: 12 },
                kind: SemanticTokenKind::Variable,
                declaration: false,
                readonly: false,
            },
        ];
        assert_eq!(
            encode_semantic_tokens(text, &tokens, PositionEncoding::Utf16),
            [0, 3, 3, 0, 1, 1, 0, 3, 7, 0]
        );
        assert_eq!(
            encode_semantic_tokens(text, &tokens, PositionEncoding::Utf8),
            [0, 5, 3, 0, 1, 1, 0, 3, 7, 0]
        );
    }
}
