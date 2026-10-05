//! Public validation of complete scripts, properties, and inherited storage.
#[path = "common/support.rs"]
mod common;
use common::{function, source};
use folio_mir::{
    ExternalSlot, Function, Instruction, Local, Op, Property, Script, ValidationErrorKind, Value,
    validate,
};

#[test]
fn validates_accessor_bodies_and_explicit_inherited_storage() {
    let mut getter = function(vec![Op::Return(Value::Identifier("inherited".into()))]);
    getter.return_type = "Int".into();
    let mut script = Script {
        target: folio_profiles::TargetProfile::skyrim_se(),
        name: "Child".into(),
        parent: "Base".into(),
        flags: 0,
        auto_state: String::new(),
        variables: vec![],
        external_slots: vec![ExternalSlot {
            owner: "Base".into(),
            name: "inherited".into(),
            ty: "Int".into(),
        }],
        properties: vec![crate::Property {
            name: "Value".into(),
            ty: "Int".into(),
            auto_var: None,
            getter: Some(getter),
            setter: None,
            read_only: true,
            flags: 0,
            source: source(),
        }],
        functions: vec![],
        state_names: vec![],
        source: source(),
        decisions: vec![],
    };
    assert!(validate(&script).is_ok());
    script.external_slots.clear();
    assert!(
        validate(&script)
            .unwrap_err()
            .iter()
            .any(|error| error.kind == ValidationErrorKind::UnknownSlot("inherited".into()))
    );
}
