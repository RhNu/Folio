use super::*;
#[path = "../../tests/common/support.rs"]
mod common;
use common::{function, source};

fn issues(function: &Function) -> Vec<ValidationError> {
    let mut errors = vec![];
    validate_function(function, &BTreeSet::new(), &mut errors);
    errors
}

#[test]
fn accepts_closed_control_flow_and_declared_storage_case_insensitively() {
    let body = function(vec![
        Op::Assign(Value::Identifier("RESULT".into()), Value::Int(3)),
        Op::Label(1),
        Op::JumpIf {
            when_true: true,
            condition: Value::Bool(true),
            target: 1,
        },
        Op::Return(Value::None),
        Op::Label(2), // Not reachable from entry.
    ]);
    assert!(issues(&body).is_empty());
}

#[test]
fn rejects_unknown_storage_and_non_storage_destinations_at_original_source() {
    let errors = issues(&function(vec![
        Op::Assign(Value::Int(1), Value::Identifier("missing".into())),
        Op::Return(Value::None),
    ]));
    assert!(
        errors
            .iter()
            .any(|error| error.kind == ValidationErrorKind::InvalidDestination)
    );
    assert!(
        errors
            .iter()
            .any(|error| error.kind == ValidationErrorKind::UnknownSlot("missing".into()))
    );
    assert!(errors.iter().all(|error| error.source == source()));
}

#[test]
fn distinguishes_missing_targets_duplicate_labels_and_reachable_fallthrough() {
    let errors = issues(&function(vec![
        Op::Label(1),
        Op::Label(1),
        Op::JumpIf {
            when_true: true,
            condition: Value::Bool(false),
            target: 8,
        },
    ]));
    assert!(
        errors
            .iter()
            .any(|error| error.kind == ValidationErrorKind::DuplicateLabel(1))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.kind == ValidationErrorKind::MissingLabel(8))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.kind == ValidationErrorKind::ReachableFallthrough)
    );
    assert!(issues(&function(vec![Op::Label(1), Op::Jump(1)])).is_empty());
}

#[test]
fn rejects_instance_access_in_global_and_code_in_native_functions() {
    let mut body = function(vec![Op::Return(Value::Identifier("Self".into()))]);
    body.is_global = true;
    assert!(
        issues(&body)
            .iter()
            .any(|error| error.kind == ValidationErrorKind::InstanceInGlobal)
    );
    body.is_native = true;
    assert!(
        issues(&body)
            .iter()
            .any(|error| error.kind == ValidationErrorKind::NativeBody)
    );
    body.instructions.clear();
    assert!(issues(&body).is_empty());
}

#[test]
fn inherited_fields_are_unique_within_owners_and_do_not_merge_child_storage() {
    let mut script = Script {
        target: folio_profiles::TargetProfile::skyrim_se(),
        name: "Child".into(),
        parent: "Base".into(),
        flags: 0,
        auto_state: String::new(),
        variables: vec![crate::Variable {
            name: "x".into(),
            ty: "Int".into(),
            initial: Value::Int(1),
            flags: 0,
            source: source(),
        }],
        external_slots: vec![crate::ExternalSlot {
            owner: "Base".into(),
            name: "x".into(),
            ty: "Int".into(),
        }],
        properties: vec![],
        functions: vec![function(vec![Op::Return(Value::Identifier("x".into()))])],
        state_names: vec![],
        source: source(),
        decisions: vec![],
    };
    assert!(validate(&script).is_ok());
    script.external_slots.push(crate::ExternalSlot {
        owner: "Grandparent".into(),
        name: "x".into(),
        ty: "Int".into(),
    });
    assert!(validate(&script).is_ok());
    script.external_slots.push(crate::ExternalSlot {
        owner: "BASE".into(),
        name: "X".into(),
        ty: "Int".into(),
    });
    assert!(validate(&script).unwrap_err().iter().any(
        |error| matches!(&error.kind, ValidationErrorKind::DuplicateName(name) if name == "BASE/X")
    ));
}
