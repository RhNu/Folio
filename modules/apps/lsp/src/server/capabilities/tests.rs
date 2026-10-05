use serde_json::json;

use super::Client;

#[test]
fn absent_and_malformed_support_is_disabled() {
    for params in [
        serde_json::Value::Null,
        json!({"initializationOptions":{"folio":{"status":"true"}}}),
    ] {
        let client = Client::read(&params);
        assert!(!client.status && !client.dynamic_watches);
        assert!(!client.presentation.markdown && !client.presentation.commands);
        assert!(!client.presentation.declaration_documents);
        assert!(
            !client.refresh.semantic_tokens
                && !client.refresh.code_lens
                && !client.refresh.inlay_hint
        );
    }
}

#[test]
fn capabilities_negotiate_independently_and_watches_require_both_flags() {
    let mut params = json!({"initializationOptions":{"folio":{"status":true,"clientCommands":true,
        "declarationDocuments":true}},"capabilities":{"textDocument":{"hover":{"contentFormat":["plaintext","markdown"]}},
        "workspace":{"semanticTokens":{"refreshSupport":true},"codeLens":{"refreshSupport":true},
        "didChangeWatchedFiles":{"dynamicRegistration":true}}}});
    let client = Client::read(&params);
    assert!(
        client.status && client.presentation.commands && client.presentation.declaration_documents
    );
    assert!(
        client.presentation.markdown && client.refresh.semantic_tokens && client.refresh.code_lens
    );
    assert!(!client.refresh.inlay_hint && !client.dynamic_watches);
    params["capabilities"]["workspace"]["didChangeWatchedFiles"]["relativePatternSupport"] =
        json!(true);
    assert!(Client::read(&params).dynamic_watches);
}
