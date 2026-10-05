use super::*;

#[test]
fn encoding_is_explicit_and_utf8_bom_is_removed() {
    assert_eq!(
        decode_source(&[0xEF, 0xBB, 0xBF, b'A'], SourceEncoding::Utf8).unwrap(),
        "A"
    );
    assert_eq!(
        decode_source(&[0x80], SourceEncoding::Windows1252).unwrap(),
        "€"
    );
    assert!(decode_source(&[0x80], SourceEncoding::Utf8).is_err());
}
