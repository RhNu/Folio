use folio_source::{FileId, SourceSpan, TextRange};

use super::{Diagnostic, Severity};

#[test]
fn keeps_file_identity_with_the_primary_range() {
    let span = SourceSpan {
        file: FileId(7),
        range: TextRange::new(3, 8).unwrap(),
    };
    let diagnostic = Diagnostic::new("syntax.error", Severity::Error, "invalid token").at(span);
    assert_eq!(diagnostic.primary, Some(span));
}
