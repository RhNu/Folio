//! Public crate behavior over in-memory inputs.
use folio_backend_pex::{EmissionOptions, emit};
use folio_format_pex::{PexFile, PexOpcode, PexStringId, PexValue};
use folio_mir::{Function, Op, Script, Value};
use folio_source::{FileId, SourceSpan};

use folio_mir::{Instruction, Local, Variable};
use folio_source::TextRange;

fn source(start: usize) -> SourceSpan {
    SourceSpan {
        file: FileId(1),
        range: TextRange { start, end: start },
    }
}

fn script(ops: Vec<Op>) -> Script {
    Script {
        target: folio_profiles::TargetProfile::skyrim_se(),
        name: "Sample".into(),
        parent: String::new(),
        flags: 0,
        auto_state: String::new(),
        variables: vec![],
        properties: vec![],
        functions: vec![Function {
            name: "Run".into(),
            state: String::new(),
            return_type: "None".into(),
            parameters: vec![],
            locals: vec![Local {
                name: "tmp".into(),
                ty: "Int".into(),
            }],
            instructions: ops
                .into_iter()
                .enumerate()
                .map(|(i, op)| Instruction {
                    op,
                    source: source(i),
                })
                .collect(),
            is_global: false,
            is_native: false,
            is_event: false,
            flags: 0,
            source: source(0),
        }],
        state_names: vec![],
        source: source(0),
        decisions: vec![],
        external_slots: vec![],
    }
}

#[test]
fn emits_branch_relative_to_instruction_and_state_runtime() {
    let script = script(vec![
        Op::Jump(9),
        Op::Assign(Value::Identifier("tmp".into()), Value::Int(5)),
        Op::Label(9),
        Op::Return(Value::None),
    ]);
    let bytes = emit(&script, &EmissionOptions::default()).unwrap();
    let pex = PexFile::read_from_slice(&bytes).unwrap();
    let object = &pex.objects[0];
    let root = &object.states[0];
    assert_eq!(root.functions.len(), 3);
    let get_state = root
        .functions
        .iter()
        .find(|fun| pex.string_table()[fun.name.index() as usize] == "GetState")
        .unwrap();
    assert_eq!(
        get_state.instructions[0].arguments,
        vec![PexValue::Identifier(PexStringId::new(
            pex.string_table()
                .iter()
                .position(|s| s == "::State")
                .unwrap() as u16
        ))]
    );
    let run = root
        .functions
        .iter()
        .find(|fun| pex.string_table()[fun.name.index() as usize] == "Run")
        .unwrap();
    assert_eq!(run.instructions[0].opcode, PexOpcode::Jmp);
    assert_eq!(run.instructions[0].arguments, vec![PexValue::Integer(2)]);
    assert_eq!(run.instructions.last().unwrap().opcode, PexOpcode::Return);
}

#[test]
fn emits_debug_lines_for_source_instructions() {
    let mut options = EmissionOptions {
        debug_info: true,
        ..EmissionOptions::default()
    };
    options.source_text.insert(FileId(1), "a\nb\n".into());
    let mut script = script(vec![Op::Return(Value::None)]);
    script.functions[0].instructions[0].source = source(2);
    let bytes = emit(&script, &options).unwrap();
    let pex = PexFile::read_from_slice(&bytes).unwrap();
    let debug = pex.debug_info.as_ref().unwrap();
    let run = debug
        .functions
        .iter()
        .find(|fun| pex.string_table()[fun.function_name.index() as usize] == "Run")
        .unwrap();
    assert_eq!(run.instruction_line_map, vec![2]);
    options.debug_info = false;
    let bytes = emit(&script, &options).unwrap();
    let pex = PexFile::read_from_slice(&bytes).unwrap();
    assert!(pex.debug_info.is_none());
}

#[test]
fn preserves_auto_property_and_declared_flag_table() {
    let mut script = script(vec![Op::Return(Value::None)]);
    script.variables.push(Variable {
        name: "::Count_var".into(),
        ty: "Int".into(),
        initial: Value::Int(4),
        flags: 0,
        source: source(0),
    });
    script.properties.push(folio_mir::Property {
        name: "Count".into(),
        ty: "Int".into(),
        auto_var: Some("::Count_var".into()),
        getter: None,
        setter: None,
        read_only: false,
        flags: 1,
        source: source(0),
    });
    let options = EmissionOptions::default();
    let bytes = emit(&script, &options).unwrap();
    let pex = PexFile::read_from_slice(&bytes).unwrap();
    let property = &pex.objects[0].properties[0];
    assert!(property.is_auto && property.is_readable && property.is_writable);
    assert_eq!(pex.user_flags.len(), 2);
    assert_eq!(
        pex.string_table()[pex.user_flags[0].name.index() as usize],
        "Hidden"
    );
}

#[test]
fn array_find_uses_array_before_destination() {
    let mut script = script(vec![
        Op::ArrayFind {
            reverse: false,
            dest: Value::Identifier("tmp".into()),
            array: Value::Identifier("items".into()),
            value: Value::Int(7),
            start: Value::Int(0),
        },
        Op::Return(Value::None),
    ]);
    script.variables.push(Variable {
        name: "items".into(),
        ty: "Int[]".into(),
        initial: Value::None,
        flags: 0,
        source: source(0),
    });
    let pex =
        PexFile::read_from_slice(&emit(&script, &EmissionOptions::default()).unwrap()).unwrap();
    let instruction = &pex.objects[0].states[0].functions[2].instructions[0];
    assert_eq!(instruction.opcode, PexOpcode::ArrayFindElement);
    let strings = pex.string_table();
    assert_eq!(
        instruction.arguments[0],
        PexValue::Identifier(PexStringId::new(
            strings.iter().position(|s| s == "items").unwrap() as u16
        ))
    );
    assert_eq!(
        instruction.arguments[1],
        PexValue::Identifier(PexStringId::new(
            strings.iter().position(|s| s == "tmp").unwrap() as u16
        ))
    );
}

#[test]
fn readonly_backed_property_uses_getter_without_auto_write_flag() {
    let mut script = script(vec![Op::Return(Value::None)]);
    script.variables.push(Variable {
        name: "::Answer_var".into(),
        ty: "Int".into(),
        initial: Value::Int(42),
        flags: 0,
        source: source(0),
    });
    script.properties.push(folio_mir::Property {
        name: "Answer".into(),
        ty: "Int".into(),
        auto_var: Some("::Answer_var".into()),
        getter: None,
        setter: None,
        read_only: true,
        flags: 0,
        source: source(0),
    });
    let pex =
        PexFile::read_from_slice(&emit(&script, &EmissionOptions::default()).unwrap()).unwrap();
    let property = &pex.objects[0].properties[0];
    assert!(property.is_readable);
    assert!(!property.is_writable);
    assert!(!property.is_auto);
    assert_eq!(
        property.read_function.as_ref().unwrap().instructions[0].opcode,
        PexOpcode::Return
    );
}

#[test]
fn rejects_custom_flag_that_reuses_builtin_name() {
    let script = script(vec![Op::Return(Value::None)]);
    let options = EmissionOptions {
        user_flags: vec![("hidden".into(), 2)],
        ..EmissionOptions::default()
    };
    let errors = emit(&script, &options).unwrap_err();
    assert_eq!(errors[0].code, "pex.flag");
}
