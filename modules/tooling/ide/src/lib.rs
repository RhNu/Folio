//! Pure editor document state and navigation over shared project analysis facts.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use folio_build::ProjectAnalysisView;
use folio_hir::Type;
use folio_source::{FileId, LineIndex, SourceSpan, TextRange};

mod external;
mod symbols;
pub use external::{external_declaration_range, referenced_symbol, symbol_script};
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
        }
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
        }
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
    pub fn generation(&self) -> u64 {
        self.generation
    }

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
            }
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
                updated = replacement.clone();
            }
        }
        document.overlay = Some((version, Arc::from(updated)));
        self.generation += 1;
        Ok(())
    }

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

    pub fn is_open(&self, path: &PathBuf) -> bool {
        self.version(path).is_some()
    }

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
    pub span: SourceSpan,
    pub owner_script: Option<String>,
}

/// Returns a typed hover from the same semantic facts used by project checks.
pub fn hover(view: &ProjectAnalysisView, file: FileId, byte: usize) -> Option<Hover> {
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
            content: format!("script {}{parent}", name.text),
            span: name.span,
            owner_script: Some(name.text.clone()),
        });
    }
    if let Some(parent) = &script.parent
        && parent.span.range.start <= byte
        && byte < parent.span.range.end
    {
        return Some(Hover {
            content: format!("script {}", parent.text),
            span: parent.span,
            owner_script: Some(parent.text.clone()),
        });
    }
    if let Some(declaration) = script.declarations.iter().find(|declaration| {
        declaration.span.range.start <= byte && byte < declaration.span.range.end
    }) {
        let content = symbols::describe_symbol(&script, &declaration.symbol, &declaration.ty);
        return Some(Hover {
            content,
            span: declaration.span,
            owner_script: symbols::owner_script(&declaration.symbol),
        });
    }
    if let Some((name, span)) = symbols::script_reference(view, file, byte) {
        return Some(Hover {
            content: format!("script {name}"),
            span,
            owner_script: Some(name),
        });
    }
    let fact = script.expression_at(byte)?;
    if fact.ty == Type::Error {
        return None;
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
        content,
        span,
        owner_script: fact
            .binding
            .as_ref()
            .and_then(|binding| symbols::owner_script(&binding.symbol)),
    })
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

pub fn definition(view: &ProjectAnalysisView, file: FileId, byte: usize) -> Option<SourceSpan> {
    view.analysis.definition(file, byte)
}

#[cfg(test)]
mod tests;
