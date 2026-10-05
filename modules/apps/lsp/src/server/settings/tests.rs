use serde_json::json;

use super::*;

#[test]
fn preferences_default_and_override_independently() {
    let defaults = EditorSettings::read(&Value::Null);
    assert!(defaults.hover.documentation && defaults.lenses.enabled && defaults.parameter_names);
    let settings = EditorSettings::read(&json!({"folio":{"editor":{
        "hover":{"documentation":false},
        "codeLens":{"references":false},
        "inlayHints":{"parameterNames":false}
    }}}));
    assert!(
        !settings.hover.documentation
            && !settings.lenses.kinds.references
            && !settings.parameter_names
    );
    assert!(
        settings.hover.details && settings.lenses.enabled && settings.lenses.kinds.implementations
    );
}
