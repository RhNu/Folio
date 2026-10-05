use folio_mir::{Instruction, Property, Variable};
use folio_profiles::TargetProfile;
use folio_source::TextRange;

use super::*;

fn source() -> SourceSpan {
    SourceSpan {
        file: FileId(1),
        range: TextRange { start: 0, end: 0 },
    }
}

fn script() -> Script {
    Script {
        target: TargetProfile::skyrim_se(),
        name: "Example".into(),
        parent: String::new(),
        flags: 0,
        auto_state: String::new(),
        variables: vec![],
        external_slots: vec![],
        properties: vec![],
        functions: vec![],
        state_names: vec![],
        source: source(),
        decisions: vec![],
    }
}

fn getter(value: Value) -> Function {
    Function {
        name: "Get".into(),
        state: String::new(),
        return_type: "Int".into(),
        parameters: vec![],
        locals: vec![],
        instructions: vec![Instruction {
            op: Op::Return(value),
            source: source(),
        }],
        flags: 0,
        is_global: false,
        is_native: false,
        is_event: false,
        source: source(),
    }
}

fn decoded(script: &Script) -> PexFile {
    PexFile::read_from_slice(&emit(script, &EmissionOptions::default()).unwrap()).unwrap()
}

#[test]
fn signed_mask_values_survive_pex_encoding() {
    for value in [i32::MIN, -1] {
        let mut input = script();
        input.functions.push(getter(Value::Int(value)));
        let output = decoded(&input);
        let function = output.objects[0]
            .states
            .iter()
            .flat_map(|state| &state.functions)
            .find(|function| output.resolve_string(function.name) == Some("Get"))
            .expect("emitted function");
        let instruction = &function.instructions[0];
        assert_eq!(instruction.opcode, PexOpcode::Return);
        assert_eq!(instruction.arguments, [PexValue::Integer(value)]);
    }
}

#[test]
fn literal_getter_serializes_without_an_auto_or_saved_variable() {
    let mut input = script();
    input.properties.push(Property {
        name: "Answer".into(),
        ty: "Int".into(),
        auto_var: None,
        read_only: true,
        getter: Some(getter(Value::Int(42))),
        setter: None,
        flags: 0,
        source: source(),
    });
    let output = decoded(&input);
    let object = &output.objects[0];
    assert_eq!(object.variables, [] as [folio_format_pex::PexVariable; 0]);
    let property = &object.properties[0];
    assert!(property.is_readable);
    assert!(!property.is_writable);
    assert!(!property.is_auto);
    assert!(property.auto_var.is_none());
    let instructions = &property.read_function.as_ref().unwrap().instructions;
    assert_eq!(instructions.len(), 1);
    assert_eq!(instructions[0].opcode, PexOpcode::Return);
    assert_eq!(instructions[0].arguments, vec![PexValue::Integer(42)]);
}

#[test]
fn conditional_is_serialized_on_auto_storage_separately_from_property_metadata() {
    let mut input = script();
    input.variables.push(Variable {
        name: "::Answer_var".into(),
        ty: "Int".into(),
        initial: Value::Int(42),
        flags: 2,
        source: source(),
    });
    input.properties.push(Property {
        name: "Answer".into(),
        ty: "Int".into(),
        auto_var: Some("::Answer_var".into()),
        read_only: false,
        getter: None,
        setter: None,
        flags: 1,
        source: source(),
    });
    let output = decoded(&input);
    assert_eq!(output.objects[0].variables[0].user_flags, 2);
    assert_eq!(output.objects[0].properties[0].user_flags, 1);
}

#[test]
fn state_runtime_preserves_callback_order_and_has_a_complete_void_return() {
    let output = decoded(&script());
    let function = output.objects[0].states[0]
        .functions
        .iter()
        .find(|function| output.string_table()[function.name.index() as usize] == "GotoState")
        .unwrap();
    assert_eq!(
        output.string_table()[function.parameters[0].name.index() as usize],
        "asNewState"
    );
    let instructions = &function.instructions;
    assert_eq!(instructions.len(), 4);
    assert_eq!(instructions[0].opcode, PexOpcode::CallMethod);
    assert_eq!(instructions[1].opcode, PexOpcode::Assign);
    assert_eq!(instructions[2].opcode, PexOpcode::CallMethod);
    assert_eq!(instructions[3].opcode, PexOpcode::Return);
    for (index, expected) in [(0, "onEndState"), (2, "onBeginState")] {
        let PexValue::Identifier(name) = instructions[index].arguments[0] else {
            panic!("callback must have a method name")
        };
        assert_eq!(output.string_table()[name.index() as usize], expected);
    }
    assert_eq!(instructions[3].arguments, vec![PexValue::None]);
}

#[test]
fn emission_rejects_local_state_overflow_before_encoding() {
    let mut input = script();
    input.state_names = (0..127).map(|index| format!("S{index}")).collect();
    assert_eq!(decoded(&input).objects[0].states.len(), 128);
    input.state_names.push("TooMany".into());
    assert!(
        emit(&input, &EmissionOptions::default())
            .unwrap_err()
            .iter()
            .any(|error| error.code == "pex.mir")
    );
}
