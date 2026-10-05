use folio_ide::PositionEncoding;
use serde_json::json;

use super::negotiate_encoding;

#[test]
fn utf16_is_default_and_utf8_is_negotiated() {
    assert_eq!(negotiate_encoding(&json!({})), PositionEncoding::Utf16);
    assert_eq!(
        negotiate_encoding(
            &json!({"capabilities":{"general":{"positionEncodings":["utf-8","utf-16"]}}})
        ),
        PositionEncoding::Utf8
    );
}
