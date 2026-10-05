//! Formatting and symbol requests with the existing cancellation and scheduling.
use folio_format::format_source;
use folio_ide::{Position, PositionIndex};
use folio_papyrus::PapyrusDialect;
use folio_project_model::{DependencyKind, SourceId};
use folio_source::TextRange;

use super::{LspError, Metadata, Ordering, PositionEncoding, Server, Value, json, uri_to_path};
use crate::protocol::range_json;

impl Server {
    /// Returns edits only for the current buffer generation and negotiated encoding.
    pub(super) fn format_document(&self, id: &Value, params: &Value) -> Result<(), LspError> {
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
            return self.reply(id, &json!([]));
        }
        let Some(source) = self.documents.text(&path) else {
            return self.reply(id, &json!([]));
        };
        let generation = self.generation.load(Ordering::SeqCst);
        let version = self.documents.version(&path);
        let formatted = match format_source(source, PapyrusDialect::Skyrim) {
            Ok(formatted) => formatted,
            Err(error) => {
                tracing::debug!(?error, uri, "document formatting rejected");
                return self.error(id, -32001, "document cannot be safely formatted");
            },
        };
        if generation != self.generation.load(Ordering::SeqCst)
            || version != self.documents.version(&path)
        {
            return self.error(id, -32800, "request cancelled");
        }
        if formatted == source {
            return self.reply(id, &json!([]));
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
        self.reply(
            id,
            &json!([{"range":range_json(range),"newText":formatted}]),
        )
    }
}

/// Encodes sorted identifier spans in the position units negotiated by the client.
pub(super) fn encode_semantic_tokens(
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

pub(super) fn document_symbol_json(
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

pub(super) fn symbol_origin(metadata: &Metadata, script: &str) -> Option<String> {
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
pub(super) fn request_stale(
    request_generation: u64,
    current_generation: u64,
    cancelled: bool,
) -> bool {
    cancelled || request_generation != current_generation
}

#[cfg(test)]
mod tests;
