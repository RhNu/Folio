//! Rich hover, definition locations, and project-wide navigation.
use super::*;

impl QueryContext {
    pub(super) fn navigation(&self, method: &str, params: &Value) -> QueryResult {
        let Some(view) = &self.view else {
            return Ok(Value::Null);
        };
        if method == "folio/declarationContent" {
            let Some(owner) = params["uri"].as_str().and_then(presentation::virtual_owner) else {
                return Err((-32602, "invalid declaration document URI".into()));
            };
            return Ok(self.declaration_text(&owner).map_or(
                Value::Null,
                |text| json!({"text":text,"languageId":"papyrus"}),
            ));
        }
        if method == "textDocument/hover" {
            return Ok(self.hover(params).unwrap_or(Value::Null));
        }
        let Some(symbol) = self.occurrence(params) else {
            return Ok(Value::Null);
        };
        let result = match method {
            "textDocument/definition" | "textDocument/declaration" => {
                self.declaration_location(&symbol).unwrap_or(Value::Null)
            }
            "textDocument/references" => json!(
                self.locations(&folio_ide::references_of(
                    view,
                    &symbol,
                    params["context"]["includeDeclaration"]
                        .as_bool()
                        .unwrap_or(false)
                ))
            ),
            "textDocument/implementation" => json!(self.implementation_locations(&symbol)),
            "textDocument/documentHighlight" => {
                let Some((file, byte)) = self.at(params) else {
                    return Ok(json!([]));
                };
                let text = view.analysis.text(file).unwrap_or("");
                json!(folio_ide::document_highlights(view, file, byte).iter().filter_map(|span| {
                    Some(json!({"range":range_json(folio_ide::range(text, span.range, self.encoding)?),"kind":1}))
                }).collect::<Vec<_>>())
            }
            _ => Value::Null,
        };
        Ok(result)
    }

    pub(super) fn declaration_text(&self, owner: &str) -> Option<String> {
        let document = folio_ide::declaration_document(self.view.as_ref()?, owner)?;
        let origin = self
            .metadata
            .as_ref()
            .and_then(|metadata| requests::symbol_origin(metadata, owner))
            .unwrap_or_default();
        Some(format!(
            "; {owner} · {}\n; Read-only API of the selected provider.\n\n{}",
            origin.replace(['\r', '\n'], " "),
            document.text
        ))
    }

    pub(super) fn implementation_locations(&self, symbol: &Symbol) -> Vec<Value> {
        folio_ide::implementation_symbols(self.view.as_ref().expect("query view"), symbol)
            .iter()
            .filter_map(|symbol| self.declaration_location(symbol))
            .collect()
    }

    pub(super) fn declaration_location(&self, symbol: &Symbol) -> Option<Value> {
        let view = self.view.as_ref()?;
        if let Some(span) = folio_ide::definition_of(view, symbol) {
            return presentation::source_location(view, span, self.encoding);
        }
        let owner = folio_ide::symbol_script(symbol)?;
        if let (Some(metadata), Some(loaded)) = (&self.metadata, &self.loaded)
            && let Some((path, text)) =
                presentation::external_source(metadata, loaded, owner, &self.overlays)
            && let Some(range) = folio_ide::external_declaration_range(&text, symbol)
        {
            return Some(
                json!({"uri":path_to_uri(&path),"range":range_json(folio_ide::range(&text, range, self.encoding)?)}),
            );
        }
        if !self.virtual_documents {
            return None;
        }
        let document = folio_ide::declaration_document(view, owner)?;
        let text = self.declaration_text(owner)?;
        let prefix = text.len().checked_sub(document.text.len())?;
        let range = document
            .declarations
            .iter()
            .find(|(candidate, _)| same_symbol(candidate, symbol))
            .map(|(_, range)| TextRange {
                start: range.start + prefix,
                end: range.end + prefix,
            })?;
        Some(
            json!({"uri":presentation::virtual_uri(owner),"range":range_json(folio_ide::range(&text, range, self.encoding)?)}),
        )
    }

    fn hover(&self, params: &Value) -> Option<Value> {
        let view = self.view.as_ref()?;
        let uri = params["textDocument"]["uri"].as_str()?;
        let (item, range) = if let Some((file, byte)) = self.at(params) {
            let item = folio_ide::hover(view, file, byte)?;
            let range =
                folio_ide::range(view.analysis.text(file)?, item.span?.range, self.encoding)?;
            (item, range)
        } else {
            let symbol = self.occurrence(params)?;
            let text = self.document_source(uri)?;
            let byte =
                folio_ide::offset(&text, parse_position(&params["position"])?, self.encoding)?;
            let token = folio_papyrus::lex(&text)
                .into_iter()
                .find(|token| token.range.start <= byte && byte < token.range.end)?;
            (
                folio_ide::hover_symbol(view, &symbol)?,
                folio_ide::range(&text, token.range, self.encoding)?,
            )
        };
        let origin = item.owner_script.as_ref().and_then(|owner| {
            self.metadata
                .as_ref()
                .and_then(|metadata| requests::symbol_origin(metadata, owner))
        });
        let links = if self.settings.details {
            item.symbol
                .as_ref()
                .map_or_else(Vec::new, |symbol| self.hover_links(params, symbol))
        } else {
            Vec::new()
        };
        Some(json!({
            "contents":{"kind":if self.markdown { "markdown" } else { "plaintext" },
                "value":presentation::hover_text(&item,origin.as_deref(),&self.settings,self.markdown,&links)},
            "range":range_json(range)
        }))
    }

    fn hover_links(&self, params: &Value, symbol: &Symbol) -> Vec<String> {
        let mut links = Vec::new();
        if let Some(location) = self.declaration_location(symbol) {
            if self.commands {
                links.push(presentation::command_link(
                    "Go to declaration",
                    "folio.openLocation",
                    json!([location["uri"], location["range"]]),
                ));
            } else if let (Some(uri), Some(line), Some(character)) = (
                location["uri"].as_str(),
                location["range"]["start"]["line"].as_u64(),
                location["range"]["start"]["character"].as_u64(),
            ) {
                links.push(format!(
                    "[Go to declaration]({uri}#L{},{})",
                    line + 1,
                    character + 1
                ));
            }
        }
        if let Some(owner) = folio_ide::symbol_script(symbol)
            && !matches!(symbol, Symbol::Script(_))
            && let Some(location) = self.declaration_location(&Symbol::Script(owner.into()))
            && self.commands
        {
            links.push(presentation::command_link(
                "Owning script",
                "folio.openLocation",
                json!([location["uri"], location["range"]]),
            ));
        }
        if self.commands {
            let view = self.view.as_ref().expect("query view");
            let references = folio_ide::references_of(view, symbol, false);
            let references = self.locations(&references);
            let implementations = self.implementation_locations(symbol);
            for (label, command, locations) in [
                ("project references", "folio.showReferences", references),
                (
                    "implementations",
                    "folio.showImplementations",
                    implementations,
                ),
            ] {
                if locations.is_empty() {
                    continue;
                }
                links.push(presentation::command_link(
                    &format!("{} {label}", locations.len()),
                    command,
                    json!([params["textDocument"]["uri"], params["position"], locations]),
                ));
            }
        }
        links
    }
}

fn same_symbol(left: &Symbol, right: &Symbol) -> bool {
    match (left, right) {
        (Symbol::Script(a), Symbol::Script(b)) => a.eq_ignore_ascii_case(b),
        (Symbol::Member { script: a, name: c }, Symbol::Member { script: b, name: d }) => {
            a.eq_ignore_ascii_case(b) && c.eq_ignore_ascii_case(d)
        }
        (
            Symbol::StateMember {
                script: a,
                state: c,
                name: e,
            },
            Symbol::StateMember {
                script: b,
                state: d,
                name: f,
            },
        ) => a.eq_ignore_ascii_case(b) && c.eq_ignore_ascii_case(d) && e.eq_ignore_ascii_case(f),
        _ => left == right,
    }
}
