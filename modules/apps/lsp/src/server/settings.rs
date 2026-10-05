//! Client preferences affect presentation without changing compiler inputs.
use serde_json::Value;

#[derive(Clone, Debug)]
pub(super) struct HoverSettings {
    pub documentation: bool,
    pub details: bool,
}

#[derive(Clone, Debug)]
pub(super) struct LensKinds {
    pub references: bool,
    pub implementations: bool,
    pub source: bool,
}

#[derive(Clone, Debug)]
pub(super) struct LensSettings {
    pub enabled: bool,
    pub kinds: LensKinds,
}

#[derive(Clone, Debug)]
pub(super) struct EditorSettings {
    pub hover: HoverSettings,
    pub lenses: LensSettings,
    pub parameter_names: bool,
}

impl Default for EditorSettings {
    fn default() -> Self { Self::read(&Value::Null) }
}

impl EditorSettings {
    /// Missing fields restore defaults; malformed values cannot disable a feature.
    pub(super) fn read(value: &Value) -> Self {
        let value = &value["folio"]["editor"];
        let read = |group: &str, name: &str| value[group][name].as_bool().unwrap_or(true);
        Self {
            hover: HoverSettings {
                documentation: read("hover", "documentation"),
                details: read("hover", "details"),
            },
            lenses: LensSettings {
                enabled: read("codeLens", "enabled"),
                kinds: LensKinds {
                    references: read("codeLens", "references"),
                    implementations: read("codeLens", "implementations"),
                    source: read("codeLens", "source"),
                },
            },
            parameter_names: read("inlayHints", "parameterNames"),
        }
    }
}

#[cfg(test)]
mod tests;
