//! Completion, signatures, lenses, hints, and verified workspace edits.
use super::*;

impl QueryContext {
    pub(super) fn features(&self, method: &str, params: &Value) -> QueryResult {
        let view = self.view.as_ref().expect("query view");
        if method == "workspace/symbol" {
            return Ok(json!(
                folio_ide::workspace_symbols(view, params["query"].as_str().unwrap_or(""))
                    .iter()
                    .filter_map(|item| Some(json!({"name":item.name,"kind":item.kind,
                    "location":presentation::source_location(view,item.span,self.encoding)?,
                    "containerName":item.container})))
                    .collect::<Vec<_>>()
            ));
        }
        if method == "codeLens/resolve" {
            return self.resolve_lens(params);
        }
        if method == "completionItem/resolve" {
            return self.resolve_completion(params);
        }
        let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
        let Some(file) = self.file(uri) else {
            if matches!(method, "textDocument/prepareRename" | "textDocument/rename") {
                return Err((
                    -32602,
                    "only verified project sources can be renamed".into(),
                ));
            }
            return Ok(empty_answer(method));
        };
        let text = view.analysis.text(file).expect("project source");
        let result = match method {
            "textDocument/completion" => {
                let Some((_, byte)) = self.at(params) else {
                    return Ok(json!([]));
                };
                let items = folio_ide::completion(view,file,byte).iter().enumerate().filter_map(|(index,item)| {
                    Some(json!({"label":item.label,"detail":item.detail,"kind":item.kind,
                        "textEdit":{"range":range_json(folio_ide::range(text,item.replacement,self.encoding)?),"newText":item.insert_text},
                        "data":{"uri":uri,"position":params["position"],"generation":self.generation,"index":index,"label":item.label}}))
                }).collect::<Vec<_>>();
                tracing::debug!(
                    ?file,
                    count = items.len(),
                    "collected completion candidates"
                );
                json!({"isIncomplete":false,"items":items})
            }
            "textDocument/signatureHelp" => {
                let Some((_, byte)) = self.at(params) else {
                    return Ok(Value::Null);
                };
                let Some(info) = folio_ide::signature_help(view, file, byte) else {
                    return Ok(Value::Null);
                };
                let parameters = info
                    .parameters
                    .iter()
                    .map(|label| json!({"label":label}))
                    .collect::<Vec<_>>();
                let mut signature = json!({"label":info.label,"parameters":parameters});
                if self.settings.documentation
                    && let Some(documentation) = info.documentation
                {
                    signature["documentation"] = json!({"kind":"plaintext","value":documentation});
                }
                json!({"signatures":[signature],"activeSignature":0,"activeParameter":info.active_parameter})
            }
            "textDocument/documentSymbol" => {
                let positions = PositionIndex::new(text);
                json!(
                    folio_ide::document_symbols(view, file)
                        .iter()
                        .filter_map(|item| requests::document_symbol_json(
                            &positions,
                            item,
                            self.encoding
                        ))
                        .collect::<Vec<_>>()
                )
            }
            "textDocument/semanticTokens/full" => {
                json!({"data":requests::encode_semantic_tokens(text,&folio_ide::semantic_tokens(view,file),self.encoding)})
            }
            "textDocument/codeLens" => self.lenses(file, uri),
            "textDocument/inlayHint" => {
                if !self.settings.parameter_names {
                    return Ok(json!([]));
                }
                let Some(range) = crate::protocol::parse_range(&params["range"]) else {
                    return Err((-32602, "invalid inlay hint range".into()));
                };
                let Some(start) = folio_ide::offset(text, range.start, self.encoding) else {
                    return Err((-32602, "invalid range start".into()));
                };
                let Some(end) = folio_ide::offset(text, range.end, self.encoding) else {
                    return Err((-32602, "invalid range end".into()));
                };
                if start > end {
                    return Err((-32602, "reversed inlay hint range".into()));
                }
                json!(folio_ide::inlay_hints(view,file,TextRange{start,end}).iter().filter_map(|hint| {
                    let position = folio_ide::position(text,hint.byte,self.encoding)?;
                    let mut label = json!({"value":hint.label});
                    if let Some(symbol) = &hint.parameter && let Some(location) = self.declaration_location(symbol) {
                        label["location"] = location;
                    }
                    Some(json!({"position":{"line":position.line,"character":position.character},
                        "label":[label],"kind":2,"paddingRight":true}))
                }).collect::<Vec<_>>())
            }
            "textDocument/prepareRename" => {
                let Some((_, byte)) = self.at(params) else {
                    return Err((-32602, "no editable symbol at position".into()));
                };
                let target = folio_ide::prepare_rename(view, file, byte)
                    .map_err(|error| (-32602, error.to_string()))?;
                json!({"range":range_json(folio_ide::range(text,target.span.range,self.encoding).ok_or((-32602,"invalid rename range".into()))?),
                    "placeholder":target.placeholder})
            }
            "textDocument/rename" => {
                let Some((_, byte)) = self.at(params) else {
                    return Err((-32602, "no editable symbol at position".into()));
                };
                let name = params["newName"]
                    .as_str()
                    .ok_or((-32602, "missing new name".into()))?;
                let edits = folio_ide::rename(view, file, byte, name)
                    .map_err(|error| (-32602, error.to_string()))?;
                let mut grouped = BTreeMap::<FileId, Vec<Value>>::new();
                for edit in edits {
                    let Some(location) =
                        presentation::source_location(view, edit.span, self.encoding)
                    else {
                        return Err((
                            -32602,
                            "rename target is not an editable project source".into(),
                        ));
                    };
                    grouped
                        .entry(edit.span.file)
                        .or_default()
                        .push(json!({"range":location["range"],"newText":edit.replacement}));
                }
                let documents = grouped.into_iter().map(|(file,edits)|json!({
                    "textDocument":{"uri":path_to_uri(&view.sources[&file].canonical_path),"version":self.versions[&file]},
                    "edits":edits
                })).collect::<Vec<_>>();
                json!({"documentChanges":documents})
            }
            _ => Value::Null,
        };
        Ok(result)
    }

    fn resolve_completion(&self, params: &Value) -> QueryResult {
        let data = &params["data"];
        if data["generation"].as_u64() != Some(self.generation) {
            return Err((-32801, "completion snapshot changed".into()));
        }
        let query = json!({"textDocument":{"uri":data["uri"]},"position":data["position"]});
        let Some((file, byte)) = self.at(&query) else {
            return Err((-32602, "invalid completion data".into()));
        };
        let items = folio_ide::completion(self.view.as_ref().expect("query view"), file, byte);
        let item = data["index"]
            .as_u64()
            .and_then(|index| items.get(index as usize))
            .filter(|item| data["label"].as_str() == Some(item.label.as_str()))
            .ok_or((-32801, "completion candidate changed".into()))?;
        let mut result = params.clone();
        if let Some(symbol) = &item.symbol
            && let Some(hover) =
                folio_ide::hover_symbol(self.view.as_ref().expect("query view"), symbol)
        {
            let origin = hover.owner_script.as_deref().and_then(|owner| {
                self.metadata
                    .as_ref()
                    .and_then(|metadata| requests::symbol_origin(metadata, owner))
            });
            result["documentation"] = json!({"kind":if self.markdown{"markdown"}else{"plaintext"},
                "value":presentation::hover_text(&hover,origin.as_deref(),&self.settings,self.markdown,&[])});
        } else if self.settings.documentation
            && let Some(documentation) = &item.documentation
        {
            result["documentation"] = json!({"kind":"plaintext","value":documentation});
        }
        Ok(result)
    }

    fn lenses(&self, file: FileId, uri: &str) -> Value {
        if !self.settings.lenses || !self.commands {
            return json!([]);
        }
        let view = self.view.as_ref().expect("query view");
        let Some(script) = view.analysis.hir(file) else {
            return json!([]);
        };
        let text = view.analysis.text(file).unwrap_or("");
        let mut declarations = script
            .declarations
            .iter()
            .filter(|item| {
                matches!(
                    item.symbol,
                    Symbol::Member { .. } | Symbol::StateMember { .. }
                )
            })
            .map(|item| item.span)
            .collect::<Vec<_>>();
        if let Some(name) = &script.name {
            declarations.insert(0, name.span);
        }
        let mut lenses = Vec::new();
        for span in declarations {
            let Some(range) = folio_ide::range(text, span.range, self.encoding) else {
                continue;
            };
            let Some(occurrence) = folio_ide::symbol_at(view, file, span.range.start) else {
                continue;
            };
            let hierarchy_target = matches!(occurrence.symbol, Symbol::Script(_))
                || script.members.iter().any(|member| {
                    member.symbol == occurrence.symbol
                        && matches!(
                            member.kind,
                            folio_hir::MemberKind::Function { global: false, .. }
                        )
                });
            for (enabled, kind) in [
                (self.settings.references, "references"),
                (
                    self.settings.implementations && hierarchy_target,
                    "implementations",
                ),
                (
                    self.settings.source && matches!(occurrence.symbol, Symbol::Script(_)),
                    "source",
                ),
            ] {
                if enabled {
                    lenses.push(json!({"range":range_json(range),"data":{"uri":uri,
                        "position":{"line":range.start.line,"character":range.start.character},
                        "generation":self.generation,"kind":kind}}));
                }
            }
            if matches!(occurrence.symbol, Symbol::Script(_))
                && self.settings.implementations
                && script.parent.is_some()
            {
                lenses.push(json!({"range":range_json(range),"data":{"uri":uri,
                    "position":{"line":range.start.line,"character":range.start.character},
                    "generation":self.generation,"kind":"parent"}}));
            }
        }
        json!(lenses)
    }

    fn resolve_lens(&self, params: &Value) -> QueryResult {
        let data = &params["data"];
        if data["generation"].as_u64() != Some(self.generation) {
            return Err((-32801, "CodeLens snapshot changed".into()));
        }
        let query = json!({"textDocument":{"uri":data["uri"]},"position":data["position"]});
        let symbol = self
            .occurrence(&query)
            .ok_or((-32801, "CodeLens symbol changed".into()))?;
        let view = self.view.as_ref().expect("query view");
        let mut lens = params.clone();
        let command = match data["kind"].as_str() {
            Some("parent") => {
                let (file, _) = self
                    .at(&query)
                    .ok_or((-32602, "invalid parent lens".into()))?;
                let parent = view
                    .analysis
                    .hir(file)
                    .and_then(|script| script.parent.clone())
                    .ok_or((-32801, "parent declaration changed".into()))?;
                let location = self
                    .declaration_location(&Symbol::Script(parent.text.clone()))
                    .ok_or((-32602, "parent declaration unavailable".into()))?;
                json!({"title":format!("extends {}",parent.text),"command":"folio.openLocation","arguments":[location["uri"],location["range"]]})
            }
            Some("source") => {
                let owner = folio_ide::symbol_script(&symbol)
                    .ok_or((-32602, "invalid source lens".into()))?;
                let title = self
                    .metadata
                    .as_ref()
                    .and_then(|metadata| requests::symbol_origin(metadata, owner))
                    .unwrap_or_else(|| owner.to_owned());
                let location = self
                    .declaration_location(&symbol)
                    .ok_or((-32602, "source unavailable".into()))?;
                json!({"title":title,"command":"folio.openLocation","arguments":[location["uri"],location["range"]]})
            }
            Some(kind @ ("references" | "implementations")) => {
                let locations = if kind == "references" {
                    self.locations(&folio_ide::references_of(view, &symbol, false))
                } else {
                    self.implementation_locations(&symbol)
                };
                let title = if kind == "references" {
                    format!("{} project references", locations.len())
                } else if matches!(symbol, Symbol::Script(_)) {
                    format!("{} derived scripts", locations.len())
                } else {
                    format!("{} overrides", locations.len())
                };
                json!({"title":title,"command":if kind=="references"{"folio.showReferences"}else{"folio.showImplementations"},
                    "arguments":[data["uri"],data["position"],locations]})
            }
            _ => return Err((-32602, "invalid CodeLens kind".into())),
        };
        lens["command"] = command;
        Ok(lens)
    }
}
