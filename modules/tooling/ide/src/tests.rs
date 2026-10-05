use super::*;

pub(crate) fn project(texts: &[(&str, &str)]) -> IdeSnapshot {
    let mut project = folio_build::ProjectAnalysis::new();
    project
        .sync_sources(texts.iter().map(|(name, text)| folio_build::ProjectSource {
            package_key: "root".into(),
            canonical_path: PathBuf::from(format!("{name}.psc")),
            display_path: format!("{name}.psc"),
            script_candidate: (*name).into(),
            dialect: folio_papyrus::PapyrusDialect::Skyrim,
            text: Arc::from(*text),
        }))
        .unwrap()
        .into()
}

pub(crate) fn file(view: &IdeSnapshot, name: &str) -> FileId {
    *view
        .sources
        .iter()
        .find(|(_, source)| source.script_candidate == name)
        .unwrap()
        .0
}

#[test]
fn indexed_positions_preserve_unicode_crlf_eof_and_invalid_boundaries() {
    let text = "🦊雪\r\nA\n";
    let index = PositionIndex::new(text);
    assert_eq!(
        index.position(4, PositionEncoding::Utf16),
        Some(Position {
            line: 0,
            character: 2
        })
    );
    assert_eq!(
        index.position(7, PositionEncoding::Utf16),
        Some(Position {
            line: 0,
            character: 3
        })
    );
    assert_eq!(
        index.position(7, PositionEncoding::Utf8),
        Some(Position {
            line: 0,
            character: 7
        })
    );
    assert_eq!(
        index.position(9, PositionEncoding::Utf16),
        Some(Position {
            line: 1,
            character: 0
        })
    );
    assert_eq!(
        index.position(11, PositionEncoding::Utf16),
        Some(Position {
            line: 2,
            character: 0
        })
    );
    for offset in [1, 2, 3, 5, 6, 12] {
        assert_eq!(index.position(offset, PositionEncoding::Utf16), None);
    }
    assert_eq!(
        index.range(TextRange { start: 7, end: 4 }, PositionEncoding::Utf16),
        None
    );
    assert_eq!(
        index.range(TextRange { start: 4, end: 7 }, PositionEncoding::Utf16),
        Some(Range {
            start: Position {
                line: 0,
                character: 2
            },
            end: Position {
                line: 0,
                character: 3
            }
        })
    );
}

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
