use super::LineIndex;

#[test]
fn positions_keep_crlf_and_utf8_byte_offsets() {
    let text = "a\r\n雪\n";
    let index = LineIndex::new(text);
    assert_eq!(index.line_count(), 3);
    assert_eq!(index.line_col(3), Some((1, 0)));
    assert_eq!(index.line_col(6), Some((1, 3)));
    assert_eq!(index.line_col(4), None);
    assert_eq!(index.offset(1, 3), Some(6));
    assert_eq!(index.offset(1, 1), None);
    assert_eq!(index.offset(0, 3), None);
    assert_eq!(index.line_col(text.len()), Some((2, 0)));
    assert_eq!(index.offset(2, 0), Some(text.len()));
    assert_eq!(index.offset(2, 1), None);
}

#[test]
fn non_bmp_boundaries_are_byte_based() {
    let index = LineIndex::new("🦊x");
    assert_eq!(index.line_col(4), Some((0, 4)));
    assert_eq!(index.line_col(2), None);
    assert_eq!(index.offset(0, 4), Some(4));
    assert_eq!(index.offset(0, 2), None);
    assert_eq!(index.offset(1, 0), None);
}
