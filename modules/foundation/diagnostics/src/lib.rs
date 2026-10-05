//! Structured diagnostics independent of CLI and editor presentation.

use folio_source::SourceSpan;

/// Severity of a source or project problem.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// An additional source location that explains a diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelatedLocation {
    pub span: SourceSpan,
    pub message: String,
}

/// A user-facing problem with a stable machine-readable code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub primary: Option<SourceSpan>,
    pub related: Vec<RelatedLocation>,
}

impl Diagnostic {
    /// Creates a diagnostic before optional locations are attached.
    pub fn new(code: impl Into<String>, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity,
            message: message.into(),
            primary: None,
            related: Vec::new(),
        }
    }

    /// Attaches the source range responsible for the problem.
    #[must_use]
    pub fn at(mut self, span: SourceSpan) -> Self {
        self.primary = Some(span);
        self
    }
}

#[cfg(test)]
mod tests;
