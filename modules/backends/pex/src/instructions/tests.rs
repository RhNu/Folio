use std::collections::BTreeMap;

use folio_source::{FileId, SourceSpan, TextRange};

use super::Emitter;

fn source() -> SourceSpan {
    SourceSpan {
        file: FileId(7),
        range: TextRange { start: 3, end: 5 },
    }
}

#[test]
fn relative_branches_preserve_direction_and_signed_boundaries() {
    let maximum = usize::try_from(i32::MAX).expect("i32 maximum fits usize");
    let labels = BTreeMap::from([(1, 7), (2, maximum), (3, 0)]);
    for (current, target, expected) in [
        (2, 1, 5),
        (10, 1, -3),
        (0, 2, i32::MAX),
        (maximum + 1, 3, i32::MIN),
    ] {
        assert_eq!(
            Emitter::branch_offset(&labels, current, target, source()),
            Ok(expected)
        );
    }
}

#[test]
fn invalid_branches_retain_the_responsible_source_location() {
    let maximum = usize::try_from(i32::MAX).expect("i32 maximum fits usize");
    let labels = BTreeMap::from([(1, maximum + 1), (2, 0)]);
    for (current, target) in [(0, 1), (maximum + 2, 2), (0, 9)] {
        let diagnostic = Emitter::branch_offset(&labels, current, target, source())
            .expect_err("invalid branch must fail");
        assert_eq!(diagnostic.code, "pex.branch");
        assert_eq!(diagnostic.primary, Some(source()));
    }
}
