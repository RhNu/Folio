//! Optional protocol support negotiated during initialization.
use serde_json::Value;

#[derive(Default)]
pub(super) struct Presentation {
    pub markdown: bool,
    pub commands: bool,
    pub declaration_documents: bool,
}

#[derive(Default)]
pub(super) struct Refresh {
    pub semantic_tokens: bool,
    pub code_lens: bool,
    pub inlay_hint: bool,
}

#[derive(Default)]
pub(super) struct Client {
    pub status: bool,
    pub dynamic_watches: bool,
    pub presentation: Presentation,
    pub refresh: Refresh,
}

impl Client {
    /// Unsupported or malformed capabilities stay disabled.
    pub(super) fn read(params: &Value) -> Self {
        let options = &params["initializationOptions"]["folio"];
        let workspace = &params["capabilities"]["workspace"];
        let watches = &workspace["didChangeWatchedFiles"];
        Self {
            status: options["status"].as_bool() == Some(true),
            dynamic_watches: watches["dynamicRegistration"].as_bool() == Some(true)
                && watches["relativePatternSupport"].as_bool() == Some(true),
            presentation: Presentation {
                commands: options["clientCommands"].as_bool() == Some(true),
                declaration_documents: options["declarationDocuments"].as_bool() == Some(true),
                markdown: params["capabilities"]["textDocument"]["hover"]["contentFormat"]
                    .as_array()
                    .is_some_and(|formats| {
                        formats
                            .iter()
                            .any(|format| format.as_str() == Some("markdown"))
                    }),
            },
            refresh: Refresh {
                semantic_tokens: workspace["semanticTokens"]["refreshSupport"].as_bool()
                    == Some(true),
                code_lens: workspace["codeLens"]["refreshSupport"].as_bool() == Some(true),
                inlay_hint: workspace["inlayHint"]["refreshSupport"].as_bool() == Some(true),
            },
        }
    }
}

/// Lifecycle flags remain independent because shutdown may precede readiness.
#[derive(Default)]
pub(super) struct Session {
    pub initialized: bool,
    pub ready: bool,
    pub shutdown: bool,
}

#[cfg(test)]
mod tests;
