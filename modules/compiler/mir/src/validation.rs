//! Validate explicit storage and control flow before any backend allocates bytes.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Function, Op, Script, SourceSpan, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    pub source: SourceSpan,
    pub kind: ValidationErrorKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationErrorKind {
    DuplicateName(String),
    UnknownSlot(String),
    InvalidDestination,
    InstanceInGlobal,
    DuplicateLabel(u32),
    MissingLabel(u32),
    ReachableFallthrough,
    NativeBody,
    InvalidProperty,
    InvalidState(String),
    StateCapacity,
    NonConstantInitializer,
}

impl std::fmt::Display for ValidationErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateName(name) => write!(f, "duplicate MIR name {name}"),
            Self::UnknownSlot(name) => write!(f, "unknown MIR storage slot {name}"),
            Self::InvalidDestination => write!(f, "MIR destination is not writable storage"),
            Self::InstanceInGlobal => write!(f, "global MIR function accesses instance storage"),
            Self::DuplicateLabel(label) => write!(f, "duplicate MIR label {label}"),
            Self::MissingLabel(label) => write!(f, "undefined MIR label {label}"),
            Self::ReachableFallthrough => {
                write!(f, "MIR control flow reaches the end without return")
            },
            Self::NativeBody => write!(f, "native MIR function contains executable operations"),
            Self::InvalidProperty => write!(f, "invalid MIR property accessor or backing storage"),
            Self::InvalidState(state) => write!(f, "undefined MIR state {state}"),
            Self::StateCapacity => write!(f, "MIR local state table exceeds target capacity"),
            Self::NonConstantInitializer => write!(f, "MIR field initializer is not constant"),
        }
    }
}

fn error(errors: &mut Vec<ValidationError>, source: SourceSpan, kind: ValidationErrorKind) {
    errors.push(ValidationError { source, kind });
}

/// MIR names follow Papyrus's case-insensitive identity, independent of spelling.
fn unique(
    names: &mut BTreeSet<String>,
    name: &str,
    source: SourceSpan,
    errors: &mut Vec<ValidationError>,
) {
    if !names.insert(name.to_ascii_lowercase()) {
        error(
            errors,
            source,
            ValidationErrorKind::DuplicateName(name.into()),
        );
    }
}

/// Validate all functions, including property accessors and unreachable operands.
/// Definite assignment is not required: Papyrus storage has typed default values.
///
/// # Errors
/// Returns every invalid storage, property, state, or control-flow fact.
pub fn validate(script: &Script) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    let mut fields = BTreeSet::new();
    for variable in &script.variables {
        unique(&mut fields, &variable.name, variable.source, &mut errors);
        if matches!(variable.initial, Value::Identifier(_)) {
            error(
                &mut errors,
                variable.source,
                ValidationErrorKind::NonConstantInitializer,
            );
        }
    }
    // Fields belong to their declaring object. Identical child/parent names do
    // not conflict; lowering has already selected the visible declaring owner.
    let mut inherited_fields = BTreeSet::new();
    for slot in &script.external_slots {
        unique(
            &mut inherited_fields,
            &format!("{}/{}", slot.owner, slot.name),
            script.source,
            &mut errors,
        );
        fields.insert(slot.name.to_ascii_lowercase());
    }
    let mut states = BTreeSet::from([String::new()]);
    for state in &script.state_names {
        if !state.is_empty() {
            unique(&mut states, state, script.source, &mut errors);
        }
    }
    if states.len() > script.target.max_states as usize {
        error(
            &mut errors,
            script.source,
            ValidationErrorKind::StateCapacity,
        );
    }
    if !states.contains(&script.auto_state.to_ascii_lowercase()) {
        error(
            &mut errors,
            script.source,
            ValidationErrorKind::InvalidState(script.auto_state.clone()),
        );
    }
    let mut functions = BTreeSet::new();
    for function in &script.functions {
        unique(
            &mut functions,
            &format!("{}/{}", function.state, function.name),
            function.source,
            &mut errors,
        );
        if !states.contains(&function.state.to_ascii_lowercase()) {
            error(
                &mut errors,
                function.source,
                ValidationErrorKind::InvalidState(function.state.clone()),
            );
        }
        validate_function(function, &fields, &mut errors);
    }
    validate_properties(script, &fields, &mut errors);
    tracing::debug!(phase = "mir.validate", script = %script.name,
        target = script.target.id, functions = script.functions.len(),
        errors = errors.len(), "validated MIR storage and control flow");
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validate property backing storage and accessor signatures.
fn validate_properties(
    script: &Script,
    fields: &BTreeSet<String>,
    errors: &mut Vec<ValidationError>,
) {
    let mut properties = BTreeSet::new();
    for property in &script.properties {
        unique(&mut properties, &property.name, property.source, errors);
        let invalid = match &property.auto_var {
            Some(name) => {
                !fields.contains(&name.to_ascii_lowercase())
                    || property.read_only
                    || property.getter.is_some()
                    || property.setter.is_some()
            },
            None => property.getter.is_none() && property.setter.is_none(),
        };
        if invalid {
            error(
                errors,
                property.source,
                ValidationErrorKind::InvalidProperty,
            );
        }
        if property.read_only && property.setter.is_some() {
            error(
                errors,
                property.source,
                ValidationErrorKind::InvalidProperty,
            );
        }
        if let Some(getter) = &property.getter {
            if !getter.parameters.is_empty()
                || !getter.return_type.eq_ignore_ascii_case(&property.ty)
                || getter.is_global
            {
                error(errors, getter.source, ValidationErrorKind::InvalidProperty);
            }
            validate_function(getter, fields, errors);
        }
        if let Some(setter) = &property.setter {
            if setter.parameters.len() != 1
                || !setter.parameters[0].ty.eq_ignore_ascii_case(&property.ty)
                || !setter.return_type.eq_ignore_ascii_case("none")
                || setter.is_global
            {
                error(errors, setter.source, ValidationErrorKind::InvalidProperty);
            }
            validate_function(setter, fields, errors);
        }
    }
}

fn validate_function(
    function: &Function,
    fields: &BTreeSet<String>,
    errors: &mut Vec<ValidationError>,
) {
    let mut slots = BTreeSet::new();
    for local in function.parameters.iter().chain(&function.locals) {
        unique(&mut slots, &local.name, function.source, errors);
        if local.name.eq_ignore_ascii_case("self") {
            error(
                errors,
                function.source,
                ValidationErrorKind::InvalidDestination,
            );
        }
    }
    if function.is_native {
        if !function.instructions.is_empty() {
            error(errors, function.source, ValidationErrorKind::NativeBody);
        }
        return;
    }
    let mut labels = BTreeMap::new();
    for (index, instruction) in function.instructions.iter().enumerate() {
        if let Op::Label(label) = instruction.op
            && labels.insert(label, index).is_some()
        {
            error(
                errors,
                instruction.source,
                ValidationErrorKind::DuplicateLabel(label),
            );
        }
    }
    for instruction in &function.instructions {
        if let Op::Jump(target) | Op::JumpIf { target, .. } = instruction.op
            && !labels.contains_key(&target)
        {
            error(
                errors,
                instruction.source,
                ValidationErrorKind::MissingLabel(target),
            );
        }
        let (destination, inputs) = operands(&instruction.op);
        if let Some(destination) = destination {
            if matches!(destination, Value::Identifier(name) if !name.eq_ignore_ascii_case("self"))
            {
                check_slot(
                    destination,
                    instruction.source,
                    function.is_global,
                    &slots,
                    fields,
                    errors,
                );
            } else {
                error(
                    errors,
                    instruction.source,
                    ValidationErrorKind::InvalidDestination,
                );
            }
        }
        for input in inputs {
            check_slot(
                input,
                instruction.source,
                function.is_global,
                &slots,
                fields,
                errors,
            );
        }
        if function.is_global && matches!(instruction.op, Op::CallParent { .. }) {
            error(
                errors,
                instruction.source,
                ValidationErrorKind::InstanceInGlobal,
            );
        }
    }
    // Follow actual edges so an unreachable trailing label is harmless, while a
    // conditional path falling off the end is rejected even after a valid return.
    if reaches_end_with_labels(&function.instructions, &labels) {
        error(
            errors,
            function.source,
            ValidationErrorKind::ReachableFallthrough,
        );
    }
}

/// Whether a control-flow path can run past the last instruction.
pub fn reaches_end(instructions: &[crate::Instruction]) -> bool {
    let labels = instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| match instruction.op {
            Op::Label(label) => Some((label, index)),
            _ => None,
        })
        .collect();
    reaches_end_with_labels(instructions, &labels)
}

fn reaches_end_with_labels(
    instructions: &[crate::Instruction],
    labels: &BTreeMap<u32, usize>,
) -> bool {
    let mut pending = vec![0];
    let mut reached = BTreeSet::new();
    while let Some(index) = pending.pop() {
        if !reached.insert(index) {
            continue;
        }
        let Some(instruction) = instructions.get(index) else {
            return true;
        };
        match instruction.op {
            Op::Return(_) => {},
            Op::Jump(label) => {
                if let Some(&target) = labels.get(&label) {
                    pending.push(target);
                }
            },
            Op::JumpIf { target, .. } => {
                if let Some(&target) = labels.get(&target) {
                    pending.push(target);
                }
                pending.push(index + 1);
            },
            _ => pending.push(index + 1),
        }
    }
    false
}

fn check_slot(
    value: &Value,
    source: SourceSpan,
    global: bool,
    slots: &BTreeSet<String>,
    fields: &BTreeSet<String>,
    errors: &mut Vec<ValidationError>,
) {
    let Value::Identifier(name) = value else {
        return;
    };
    let key = name.to_ascii_lowercase();
    if slots.contains(&key) {
        return;
    }
    if key == "self" || fields.contains(&key) {
        if global {
            error(errors, source, ValidationErrorKind::InstanceInGlobal);
        }
    } else {
        error(
            errors,
            source,
            ValidationErrorKind::UnknownSlot(name.clone()),
        );
    }
}

/// Classify storage operands without treating method/type names as registers.
fn operands(op: &Op) -> (Option<&Value>, Vec<&Value>) {
    match op {
        Op::Assign(dest, value) | Op::Cast(dest, value) | Op::Unary { dest, value, .. } => {
            (Some(dest), vec![value])
        },
        Op::Binary {
            dest, left, right, ..
        } => (Some(dest), vec![left, right]),
        Op::CallMethod {
            dest,
            receiver,
            args,
            ..
        } => (Some(dest), std::iter::once(receiver).chain(args).collect()),
        Op::CallParent { dest, args, .. } | Op::CallStatic { dest, args, .. } => {
            (Some(dest), args.iter().collect())
        },
        Op::PropertyGet { dest, receiver, .. } => (Some(dest), vec![receiver]),
        Op::PropertySet {
            receiver, value, ..
        } => (None, vec![receiver, value]),
        Op::ArrayCreate { dest, length } => (Some(dest), vec![length]),
        Op::ArrayLength { dest, array } => (Some(dest), vec![array]),
        Op::ArrayGet { dest, array, index } => (Some(dest), vec![array, index]),
        Op::ArraySet {
            array,
            index,
            value,
        } => (None, vec![array, index, value]),
        Op::ArrayFind {
            dest,
            array,
            value,
            start,
            ..
        } => (Some(dest), vec![array, value, start]),
        Op::JumpIf { condition, .. } => (None, vec![condition]),
        Op::Return(value) => (None, vec![value]),
        Op::Label(_) | Op::Jump(_) => (None, vec![]),
    }
}

#[cfg(test)]
mod tests;
