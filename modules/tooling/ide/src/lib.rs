//! Pure editor document state and navigation over shared project analysis facts.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use folio_hir::Type;
use folio_source::{FileId, LineIndex, SourceSpan, TextRange};

mod assist;
mod external;
mod language_help;
mod navigation;
mod snapshot;
pub use snapshot::IdeSnapshot;
mod presentation;
mod rename;
mod symbols;
pub use assist::{CompletionItem, InlayHint, completion, completion_hover, inlay_hints};
pub use external::{
    external_declaration_range, external_declaration_symbol, referenced_symbol, symbol_script,
};
pub use language_help::{LanguageHelp, language_hover};
pub use navigation::{
    SymbolOccurrence, WorkspaceSymbol, definition_of, document_highlights, implementation_symbols,
    implementations, implementations_of, references, references_of, symbol_at, workspace_symbols,
};
pub use presentation::{DeclarationDocument, declaration_document, hover_symbol};
pub use rename::{RenameEdit, RenameError, RenameTarget, prepare_rename, rename};
pub use symbols::{
    DocumentSymbol, SemanticToken, SemanticTokenKind, SignatureInfo, document_symbols,
    semantic_tokens, signature_help, source_declaration,
};

/// The character unit used in editor positions; LSP defaults to UTF-16.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PositionEncoding {
    Utf8,
    #[default]
    Utf16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// Indexes one immutable text for repeated protocol conversions without prefix scans.
pub struct PositionIndex<'a> {
    text: &'a str,
    lines: LineIndex,
}

impl<'a> PositionIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            lines: LineIndex::new(text),
        }
    }

    pub fn position(&self, offset: usize, encoding: PositionEncoding) -> Option<Position> {
        let (line, byte_col) = self.lines.line_col(offset)?;
        let start = offset - byte_col;
        let character = match encoding {
            PositionEncoding::Utf8 => byte_col,
            PositionEncoding::Utf16 => self.text[start..offset].encode_utf16().count(),
        };
        Some(Position {
            line: u32::try_from(line).ok()?,
            character: u32::try_from(character).ok()?,
        })
    }

    pub fn range(&self, bytes: TextRange, encoding: PositionEncoding) -> Option<Range> {
        if bytes.start > bytes.end {
            return None;
        }
        Some(Range {
            start: self.position(bytes.start, encoding)?,
            end: self.position(bytes.end, encoding)?,
        })
    }
}

/// Converts a protocol position to a UTF-8 byte offset without splitting a scalar value.
pub fn offset(text: &str, position: Position, encoding: PositionEncoding) -> Option<usize> {
    let line_start = if position.line == 0 {
        0
    } else {
        text.bytes()
            .enumerate()
            .filter(|(_, byte)| *byte == b'\n')
            .nth(position.line as usize - 1)
            .map(|(index, _)| index + 1)?
    };
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |relative| line_start + relative);
    let line = &text[line_start..line_end];
    match encoding {
        PositionEncoding::Utf8 => {
            let result = line_start.checked_add(position.character as usize)?;
            (result <= line_end && text.is_char_boundary(result)).then_some(result)
        },
        PositionEncoding::Utf16 => {
            let mut units = 0usize;
            for (index, ch) in line.char_indices() {
                if units == position.character as usize {
                    return Some(line_start + index);
                }
                units += ch.len_utf16();
                if units > position.character as usize {
                    return None;
                }
            }
            (units == position.character as usize).then_some(line_end)
        },
    }
}

/// Converts a UTF-8 byte boundary to an editor position.
pub fn position(text: &str, offset: usize, encoding: PositionEncoding) -> Option<Position> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let character = match encoding {
        PositionEncoding::Utf8 => offset - start,
        PositionEncoding::Utf16 => prefix[start..].encode_utf16().count(),
    };
    Some(Position {
        line: u32::try_from(line).ok()?,
        character: u32::try_from(character).ok()?,
    })
}

pub fn range(text: &str, bytes: TextRange, encoding: PositionEncoding) -> Option<Range> {
    Some(Range {
        start: position(text, bytes.start, encoding)?,
        end: position(text, bytes.end, encoding)?,
    })
}

#[derive(Clone, Debug)]
struct Document {
    disk: Arc<str>,
    overlay: Option<(i32, Arc<str>)>,
}

/// Keeps unsaved buffers ahead of disk snapshots and rejects stale versions.
#[derive(Default)]
pub struct Documents {
    files: BTreeMap<PathBuf, Document>,
    generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditError {
    UnknownDocument,
    NotOpen,
    StaleVersion { current: i32, proposed: i32 },
    InvalidRange,
}

impl Documents {
    pub fn generation(&self) -> u64 { self.generation }

    pub fn disk_update(&mut self, path: PathBuf, text: Arc<str>) {
        match self.files.get_mut(&path) {
            Some(document) => document.disk = text,
            None => {
                self.files.insert(
                    path,
                    Document {
                        disk: text,
                        overlay: None,
                    },
                );
            },
        }
        self.generation += 1;
    }

    pub fn remove_disk(&mut self, path: &PathBuf) {
        if self
            .files
            .get(path)
            .is_some_and(|document| document.overlay.is_none())
        {
            self.files.remove(path);
        }
        self.generation += 1;
    }

    /// Drops vanished disk files while retaining open buffers until close.
    pub fn retain_disk_paths(&mut self, paths: &BTreeSet<PathBuf>) {
        self.files
            .retain(|path, document| paths.contains(path) || document.overlay.is_some());
        self.generation += 1;
    }

    /// # Errors
    /// Returns an error when the proposed buffer version does not advance the current version.
    pub fn open(&mut self, path: &Path, version: i32, text: Arc<str>) -> Result<(), EditError> {
        let document = self
            .files
            .entry(path.to_path_buf())
            .or_insert_with(|| Document {
                disk: Arc::from(""),
                overlay: None,
            });
        if let Some((current, _)) = &document.overlay
            && version <= *current
        {
            return Err(EditError::StaleVersion {
                current: *current,
                proposed: version,
            });
        }
        document.overlay = Some((version, text));
        self.generation += 1;
        Ok(())
    }

    /// Applies all changes against successive intermediate texts as LSP specifies.
    /// # Errors
    /// Returns an error for unknown or closed documents, stale versions, or invalid edit ranges.
    pub fn change(
        &mut self,
        path: &PathBuf,
        version: i32,
        changes: &[(Option<Range>, String)],
        encoding: PositionEncoding,
    ) -> Result<(), EditError> {
        let document = self.files.get_mut(path).ok_or(EditError::UnknownDocument)?;
        let (current, old) = document.overlay.as_ref().ok_or(EditError::NotOpen)?;
        if version <= *current {
            return Err(EditError::StaleVersion {
                current: *current,
                proposed: version,
            });
        }
        let mut updated = old.to_string();
        for (range, replacement) in changes {
            if let Some(range) = range {
                let start =
                    offset(&updated, range.start, encoding).ok_or(EditError::InvalidRange)?;
                let end = offset(&updated, range.end, encoding).ok_or(EditError::InvalidRange)?;
                if start > end {
                    return Err(EditError::InvalidRange);
                }
                updated.replace_range(start..end, replacement);
            } else {
                updated.clone_from(replacement);
            }
        }
        document.overlay = Some((version, Arc::from(updated)));
        self.generation += 1;
        Ok(())
    }

    /// # Errors
    /// Returns an error for an unknown document or a buffer that is not open.
    pub fn close(&mut self, path: &PathBuf) -> Result<(), EditError> {
        let document = self.files.get_mut(path).ok_or(EditError::UnknownDocument)?;
        if document.overlay.take().is_none() {
            return Err(EditError::NotOpen);
        }
        self.generation += 1;
        Ok(())
    }

    pub fn text(&self, path: &PathBuf) -> Option<&str> {
        self.files.get(path).map(|document| {
            document
                .overlay
                .as_ref()
                .map_or(document.disk.as_ref(), |(_, text)| text.as_ref())
        })
    }

    pub fn version(&self, path: &PathBuf) -> Option<i32> {
        self.files
            .get(path)?
            .overlay
            .as_ref()
            .map(|(version, _)| *version)
    }

    pub fn is_open(&self, path: &PathBuf) -> bool { self.version(path).is_some() }

    pub fn overlays(&self) -> impl Iterator<Item = (&PathBuf, &str)> {
        self.files.iter().filter_map(|(path, document)| {
            document
                .overlay
                .as_ref()
                .map(|(_, text)| (path, text.as_ref()))
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hover {
    pub content: String,
    pub symbol: Option<folio_hir::Symbol>,
    pub declaration: String,
    pub documentation: Option<String>,
    pub language: Option<LanguageHelp>,
    pub details: Vec<String>,
    pub span: Option<SourceSpan>,
    pub owner_script: Option<String>,
}

/// Returns a typed hover from the same semantic facts used by project checks.
pub fn hover(view: &IdeSnapshot, file: FileId, byte: usize) -> Option<Hover> {
    if !view.analysis.text(file)?.is_char_boundary(byte) {
        return None;
    }
    let parse = view.analysis.parse(file)?;
    let syntax = parse.syntax();
    let token = language_help::token_at(&syntax, byte)?;
    if matches!(
        token.kind(),
        folio_papyrus::SyntaxKind::Whitespace
            | folio_papyrus::SyntaxKind::Newline
            | folio_papyrus::SyntaxKind::Comment
            | folio_papyrus::SyntaxKind::UnclosedComment
            | folio_papyrus::SyntaxKind::UnclosedString
            | folio_papyrus::SyntaxKind::Continuation
    ) {
        return None;
    }
    let language = language_help::at(&syntax, view.analysis.dialect(file)?, byte);
    // Self and Parent resolve to scripts, but their help describes the special variable.
    if (token.text().eq_ignore_ascii_case("self") || token.text().eq_ignore_ascii_case("parent"))
        && let Some((mut result, range)) = language.clone()
    {
        result.span = Some(SourceSpan { file, range });
        if let Some(occurrence) = symbol_at(view, file, byte) {
            result.owner_script = symbols::owner_script(&occurrence.symbol);
            result.symbol = Some(occurrence.symbol);
        }
        if let Some(script) = view.analysis.hir(file)
            && let Some(fact) = script.expression_at(byte)
            && fact.ty != Type::Error
        {
            result.declaration = format!("{} {}", display_type(&fact.ty), result.declaration);
            result.content = result.declaration.clone();
        }
        return Some(result);
    }
    if let Some(occurrence) = symbol_at(view, file, byte)
        && let Some(mut result) =
            presentation::hover_at_definition(view, &occurrence.symbol, occurrence.definition)
    {
        result.span = Some(occurrence.span);
        if matches!(occurrence.symbol, folio_hir::Symbol::Intrinsic { .. })
            && let Some((help, _)) = language.clone()
        {
            result.documentation = help.documentation;
            result.language = help.language;
        }
        return Some(result);
    }
    if let Some((mut result, range)) = language {
        // Length is represented as an unbound member fact; keep its authoritative signature.
        if let Some(script) = view.analysis.hir(file)
            && let Some(fact) = script.expression_at(byte)
            && let folio_hir::ExpressionKind::Member { owner, name } = &fact.kind
            && name.span.range.start <= byte
            && byte < name.span.range.end
            && let Some(mut intrinsic) = presentation::intrinsic_hover(&name.text, Some(&owner.ty))
        {
            intrinsic.documentation = result.documentation;
            intrinsic.language = result.language;
            intrinsic.span = Some(name.span);
            return Some(intrinsic);
        }
        result.span = Some(SourceSpan { file, range });
        return Some(result);
    }
    semantic_hover(view, file, byte)
}

/// Recover typed hover when no declaration or language catalog entry handled the token.
fn semantic_hover(view: &IdeSnapshot, file: FileId, byte: usize) -> Option<Hover> {
    let script = view.analysis.hir(file)?;
    if let Some(name) = &script.name
        && name.span.range.start <= byte
        && byte < name.span.range.end
    {
        let parent = script
            .parent
            .as_ref()
            .map_or(String::new(), |parent| format!(" extends {}", parent.text));
        return Some(Hover {
            symbol: None,
            declaration: format!("script {}{parent}", name.text),
            documentation: None,
            language: None,
            details: Vec::new(),
            content: format!("script {}{parent}", name.text),
            span: Some(name.span),
            owner_script: Some(name.text.clone()),
        });
    }
    if let Some(parent) = &script.parent
        && parent.span.range.start <= byte
        && byte < parent.span.range.end
    {
        return Some(Hover {
            symbol: None,
            declaration: format!("script {}", parent.text),
            documentation: None,
            language: None,
            details: Vec::new(),
            content: format!("script {}", parent.text),
            span: Some(parent.span),
            owner_script: Some(parent.text.clone()),
        });
    }
    if let Some(declaration) = script.declarations.iter().find(|declaration| {
        declaration.span.range.start <= byte && byte < declaration.span.range.end
    }) {
        let content = symbols::describe_symbol(&script, &declaration.symbol, &declaration.ty);
        return Some(Hover {
            symbol: Some(declaration.symbol.clone()),
            declaration: content.clone(),
            documentation: None,
            language: None,
            details: Vec::new(),
            content,
            span: Some(declaration.span),
            owner_script: symbols::owner_script(&declaration.symbol),
        });
    }
    if let Some((name, span)) = symbols::script_reference(view, file, byte) {
        return Some(Hover {
            symbol: None,
            declaration: format!("script {name}"),
            documentation: None,
            language: None,
            details: Vec::new(),
            content: format!("script {name}"),
            span: Some(span),
            owner_script: Some(name),
        });
    }
    if let Some(hover) = state_hover(view, file, byte, &script) {
        return Some(hover);
    }
    let fact = script.expression_at(byte)?;
    if fact.ty == Type::Error {
        return None;
    }
    if let folio_hir::ExpressionKind::Member { owner, name } = &fact.kind
        && let Some(mut hover) = presentation::intrinsic_hover(&name.text, Some(&owner.ty))
    {
        hover.span = Some(name.span);
        return Some(hover);
    }
    let content = fact.binding.as_ref().map_or_else(
        || display_type(&fact.ty),
        |binding| symbols::describe_symbol(&script, &binding.symbol, &fact.ty),
    );
    let span = fact
        .binding
        .as_ref()
        .map_or(fact.span, |binding| binding.name.span);
    Some(Hover {
        symbol: fact.binding.as_ref().map(|binding| binding.symbol.clone()),
        declaration: content.clone(),
        documentation: None,
        language: None,
        details: Vec::new(),
        content,
        span: Some(span),
        owner_script: fact
            .binding
            .as_ref()
            .and_then(|binding| symbols::owner_script(&binding.symbol)),
    })
}

/// State names are runtime declarations rather than ordinary bound symbols.
fn state_hover(
    view: &IdeSnapshot,
    file: FileId,
    byte: usize,
    script: &folio_hir::Script,
) -> Option<Hover> {
    if let Some(parse) = view.analysis.parse(file) {
        for node in parse
            .syntax()
            .descendants()
            .filter(|node| node.kind() == folio_papyrus::SyntaxKind::StateDecl)
        {
            let Some(token) = node
                .children_with_tokens()
                .filter_map(folio_papyrus::SyntaxElement::into_token)
                .filter(|token| token.kind() == folio_papyrus::SyntaxKind::Ident)
                .find(|token| {
                    !token.text().eq_ignore_ascii_case("state")
                        && !token.text().eq_ignore_ascii_case("auto")
                })
            else {
                continue;
            };
            let range = symbols::token_range(&token);
            if range.start <= byte && byte < range.end {
                let declaration = folio_papyrus::declaration_header(&node);
                return Some(Hover {
                    content: declaration.clone(),
                    declaration,
                    documentation: folio_papyrus::declaration_documentation(&node),
                    language: None,
                    details: vec!["Runtime state".into()],
                    symbol: None,
                    span: Some(SourceSpan { file, range }),
                    owner_script: script.name.as_ref().map(|item| item.text.clone()),
                });
            }
        }
    }
    None
}

pub(crate) fn display_type(ty: &Type) -> String {
    match ty {
        Type::Void | Type::None => "None".into(),
        Type::Int => "Int".into(),
        Type::Float => "Float".into(),
        Type::Bool => "Bool".into(),
        Type::String => "String".into(),
        Type::Script(name) => name.clone(),
        Type::Array(item) => format!("{}[]", display_type(item)),
        Type::Error => "<unknown>".into(),
    }
}

pub fn definition(view: &IdeSnapshot, file: FileId, byte: usize) -> Option<SourceSpan> {
    view.analysis.definition(file, byte)
}

#[cfg(test)]
mod tests;
