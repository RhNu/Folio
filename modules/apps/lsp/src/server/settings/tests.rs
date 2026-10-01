use super::*;
use serde_json::json;

#[test]
fn preferences_default_and_override_independently() {
    let defaults = EditorSettings::read(&Value::Null);
    assert!(defaults.documentation && defaults.lenses && defaults.parameter_names);
    let settings = EditorSettings::read(&json!({"folio":{"editor":{
        "hover":{"documentation":false},
        "codeLens":{"references":false},
        "inlayHints":{"parameterNames":false}
    }}}));
    assert!(!settings.documentation && !settings.references && !settings.parameter_names);
    assert!(settings.details && settings.lenses && settings.implementations);
}
