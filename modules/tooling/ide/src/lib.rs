//! Pure editor document state and navigation over shared project analysis facts.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use folio_build::ProjectAnalysisView;
use folio_hir::Type;
use folio_source::{FileId, SourceSpan, TextRange};

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
}

/// Returns a typed hover from the same semantic facts used by project checks.
pub fn hover(view: &ProjectAnalysisView, file: FileId, byte: usize) -> Option<Hover> {
    let script = view.analysis.hir(file)?;
    if let Some(declaration) = script.declarations.iter().find(|declaration| {
        declaration.span.range.start <= byte && byte < declaration.span.range.end
    }) {
        let text = view.analysis.text(file)?;
        let name = text.get(declaration.span.range.start..declaration.span.range.end)?;
        return Some(Hover {
            content: format!("{name}: {}", display_type(&declaration.ty)),
            span: declaration.span,
        });
    }
    let fact = script.expression_at(byte)?;
    let ty = match &fact.ty {
        Type::Void => "None".to_owned(),
        Type::Int => "Int".to_owned(),
        Type::Float => "Float".to_owned(),
        Type::Bool => "Bool".to_owned(),
        Type::String => "String".to_owned(),
        Type::Script(name) => name.clone(),
        Type::Array(item) => format!("{}[]", display_type(item)),
        Type::None => "None".to_owned(),
        Type::Error => return None,
    };
    let content = fact.binding.as_ref().map_or_else(
        || ty.clone(),
        |binding| format!("{}: {ty}", binding.name.text),
    );
    let span = fact
        .binding
        .as_ref()
        .map_or(fact.span, |binding| binding.name.span);
    Some(Hover { content, span })
}

fn display_type(ty: &Type) -> String {
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
mod tests {
    use super::*;

    #[test]
    fn positions_handle_non_bmp_and_crlf() {
        let text = "A🦊\r\n雪";
        assert_eq!(
            offset(
                text,
                Position {
                    line: 0,
                    character: 3
                },
                PositionEncoding::Utf16
            ),
            Some(5)
        );
        assert_eq!(
            offset(
                text,
                Position {
                    line: 0,
                    character: 2
                },
                PositionEncoding::Utf16
            ),
            None
        );
        assert_eq!(
            position(text, 5, PositionEncoding::Utf16),
            Some(Position {
                line: 0,
                character: 3
            })
        );
        assert_eq!(
            offset(
                text,
                Position {
                    line: 1,
                    character: 0
                },
                PositionEncoding::Utf8
            ),
            Some(7)
        );
    }

    #[test]
    fn overlay_survives_disk_update_and_close_reveals_disk() {
        let path = PathBuf::from("a.psc");
        let mut docs = Documents::default();
        docs.disk_update(path.clone(), Arc::from("disk"));
        docs.open(&path, 1, Arc::from("buffer")).unwrap();
        docs.disk_update(path.clone(), Arc::from("new disk"));
        assert_eq!(docs.text(&path), Some("buffer"));
        docs.close(&path).unwrap();
        assert_eq!(docs.text(&path), Some("new disk"));
    }

    #[test]
    fn changes_are_atomic_and_versions_advance() {
        let path = PathBuf::from("a.psc");
        let mut docs = Documents::default();
        docs.disk_update(path.clone(), Arc::from("abc"));
        docs.open(&path, 2, Arc::from("abc")).unwrap();
        let changes = [(
            Some(Range {
                start: Position {
                    line: 0,
                    character: 1,
                },
                end: Position {
                    line: 0,
                    character: 2,
                },
            }),
            "X".to_owned(),
        )];
        docs.change(&path, 3, &changes, PositionEncoding::Utf16)
            .unwrap();
        assert_eq!(docs.text(&path), Some("aXc"));
        assert_eq!(
            docs.change(&path, 3, &changes, PositionEncoding::Utf16),
            Err(EditError::StaleVersion {
                current: 3,
                proposed: 3
            })
        );
        assert_eq!(docs.text(&path), Some("aXc"));
    }

    #[test]
    fn new_unsaved_document_can_open_then_disappear_on_close() {
        let path = PathBuf::from("new.psc");
        let mut docs = Documents::default();
        docs.open(&path, 1, Arc::from("Scriptname New")).unwrap();
        assert_eq!(docs.text(&path), Some("Scriptname New"));
        docs.close(&path).unwrap();
        docs.retain_disk_paths(&BTreeSet::new());
        assert_eq!(docs.text(&path), None);
    }

    #[test]
    fn invalid_later_change_keeps_prior_buffer_and_version() {
        let path = PathBuf::from("a.psc");
        let mut docs = Documents::default();
        docs.open(&path, 1, Arc::from("abc")).unwrap();
        let changes = [
            (None, "first".to_owned()),
            (
                Some(Range {
                    start: Position {
                        line: 8,
                        character: 0,
                    },
                    end: Position {
                        line: 8,
                        character: 1,
                    },
                }),
                "bad".to_owned(),
            ),
        ];
        assert_eq!(
            docs.change(&path, 2, &changes, PositionEncoding::Utf16),
            Err(EditError::InvalidRange)
        );
        assert_eq!(docs.text(&path), Some("abc"));
        assert_eq!(docs.version(&path), Some(1));
    }
}
