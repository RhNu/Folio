//! Client preferences affect presentation without changing compiler inputs.
use serde_json::Value;

#[derive(Clone, Debug)]
pub(super) struct EditorSettings {
    pub documentation: bool,
    pub details: bool,
    pub lenses: bool,
    pub references: bool,
    pub implementations: bool,
    pub source: bool,
    pub parameter_names: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            documentation: true,
            details: true,
            lenses: true,
            references: true,
            implementations: true,
            source: true,
            parameter_names: true,
        }
    }
}

impl EditorSettings {
    /// Missing fields restore defaults; malformed values cannot disable a feature.
    pub(super) fn read(value: &Value) -> Self {
        let value = &value["folio"]["editor"];
        let read = |group: &str, name: &str| value[group][name].as_bool().unwrap_or(true);
        Self {
            documentation: read("hover", "documentation"),
            details: read("hover", "details"),
            lenses: read("codeLens", "enabled"),
            references: read("codeLens", "references"),
            implementations: read("codeLens", "implementations"),
            source: read("codeLens", "source"),
            parameter_names: read("inlayHints", "parameterNames"),
        }
    }
}

#[cfg(test)]
mod tests;
