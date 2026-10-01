//! Formatting and symbol requests with the existing cancellation and scheduling.
use super::*;
use crate::protocol::{parse_position, path_to_uri, range_json};
use folio_format::format_source;
use folio_ide::{Position, PositionIndex};
use folio_papyrus::PapyrusDialect;
use folio_project_model::{DependencyKind, SourceId};
use folio_source::TextRange;
use std::thread;

impl Server {
    /// Returns edits only for the current buffer generation and negotiated encoding.
    pub(super) fn format_document(&self, id: Value, params: &Value) -> Result<(), LspError> {
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

    pub(super) fn navigation(
        &self,
        id: Value,
        method: &str,
        params: &Value,
    ) -> Result<(), LspError> {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
        let file = uri_to_path(uri).and_then(|path| self.paths.get(&path).copied());
        let at = parse_position(&params["position"]);
        let view = self.view.clone();
        let metadata = self.metadata.clone();
        let disk_project = self.disk_project.as_ref().map(|(loaded, _)| loaded.clone());
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
                    if let Some(span) = folio_ide::source_declaration(view, file, byte) {
                        let source = view.sources.get(&span.file)?;
                        let target = view.analysis.text(span.file)?;
                        tracing::debug!(request = %key, source_file = ?file, target_file = ?span.file, "resolved source declaration");
                        return Some(json!({"uri":path_to_uri(&source.canonical_path),"range":range_json(folio_ide::range(target, span.range, encoding)?)}));
                    }
                    // Only direct PSC dependencies have verified local source snapshots.
                    let symbol = folio_ide::referenced_symbol(view, file, byte)?;
                    let owner = folio_ide::symbol_script(&symbol)?;
                    let metadata = metadata.as_deref()?;
                    let loaded = disk_project.as_ref()?;
                    let selected = &metadata.scripts.iter().find(|selection| selection.script.eq_ignore_ascii_case(owner))?.selected;
                    let dependency = loaded.dependencies.iter().find(|dependency| dependency.source_id == selected.package.source && dependency.kind == DependencyKind::Psc)?;
                    let location = selected.declaration.as_ref()?;
                    let source_path = dependency.canonical_path.join(location.source_path.as_ref()?);
                    let bytes = loaded.input_snapshots.iter().find_map(|snapshot| match snapshot {
                        folio_project_resolve::io::InputSnapshot::File { path, bytes } if path == &source_path => Some(bytes),
                        _ => None,
                    })?;
                    let SourceId::Dependency { index, .. } = &dependency.source_id else { return None; };
                    let source_encoding = loaded.root.manifest.dependencies.get(*index)?.encoding;
                    let target = folio_project_resolve::io::decode_source(bytes, source_encoding).ok()?;
                    let range = folio_ide::external_declaration_range(&target, &symbol)?;
                    tracing::debug!(request = %key, path = %source_path.display(), "resolved PSC dependency declaration");
                    Some(json!({"uri":path_to_uri(&source_path),"range":range_json(folio_ide::range(&target, range, encoding)?)}))
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
    pub(super) fn symbol_request(
        &self,
        id: Value,
        method: &str,
        params: &Value,
    ) -> Result<(), LspError> {
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
        Some(DependencyKind::Psc) => "PSC dependency",
        Some(DependencyKind::Decl) => "declaration",
        Some(DependencyKind::Repo) => "repository declaration",
        Some(DependencyKind::Pex) => "PEX declaration",
        None => match &selected.package.source {
            SourceId::Project => "project",
            SourceId::Dependency { .. } => "dependency",
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
        selected.package.version.as_deref().unwrap_or(""),
        path.map_or(String::new(), |path| format!(" · {path}"))
    ))
}

/// Results tied to an older project generation are never sent as current answers.
fn request_stale(request_generation: u64, current_generation: u64, cancelled: bool) -> bool {
    cancelled || request_generation != current_generation
}

#[cfg(test)]
mod tests;
