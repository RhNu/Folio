use super::*;
use crate::{PexOpcode, PexReadError};

/// Hand-assembled PEX: one Test.Run function, with a Return None instruction.
/// Debug records are independent of the production writer.
fn carrier(lines: &[u16], function: u16) -> Vec<u8> {
    let hex = "
        fa 57 c0 de 03 02 00 01 00 00 00 00 00 00 00 00
        00 00 00 00 00 00
        00 04
        00 00
        00 04 54 65 73 74
        00 04 4e 6f 6e 65
        00 03 52 75 6e
        01
        00 00 00 00 00 00 00 00
        00 01
        00 01 00 00
    ";
    let mut bytes = hex
        .split_whitespace()
        .map(|part| u8::from_str_radix(part, 16).unwrap())
        .collect::<Vec<_>>();
    bytes.extend_from_slice(&function.to_be_bytes());
    bytes.push(0); // Normal function, not a property accessor.
    bytes.extend_from_slice(
        &u16::try_from(lines.len())
            .expect("fixture line map fits u16")
            .to_be_bytes(),
    );
    for line in lines {
        bytes.extend_from_slice(&line.to_be_bytes());
    }
    let body = "
        00 00 00 01
        00 01 00 00 00 27
        00 00 00 00 00 00 00 00 00 00
        00 00 00 00 00 01
        00 00 00 01
        00 03 00 02 00 00 00 00 00 00 00
        00 00 00 00 00 01
        1a 00
    ";
    bytes.extend(
        body.split_whitespace()
            .map(|part| u8::from_str_radix(part, 16).unwrap()),
    );
    bytes
}

#[test]
fn empty_or_complete_line_maps_preserve_code_and_debug_records() {
    for lines in [vec![], vec![17]] {
        let bytes = carrier(&lines, 3);
        let file = PexFile::read_from_slice(&bytes).unwrap();
        assert_eq!(
            file.debug_info.as_ref().unwrap().functions[0].instruction_line_map,
            lines
        );
        let instructions = &file.objects[0].states[0].functions[0].instructions;
        assert_eq!(instructions.len(), 1);
        assert_eq!(instructions[0].opcode, PexOpcode::Return);
        assert_eq!(instructions[0].arguments, [PexValue::None]);
        assert_eq!(file.write_to_vec().unwrap(), bytes);
    }
}

#[test]
fn empty_line_maps_can_describe_multiple_instructions() {
    let mut file = PexFile::read_from_slice(&carrier(&[], 3)).unwrap();
    file.objects[0].states[0].functions[0]
        .instructions
        .insert(0, PexInstruction::new(PexOpcode::Nop, vec![]).unwrap());
    let decoded = PexFile::read_from_slice(&file.write_to_vec().unwrap()).unwrap();
    assert_eq!(
        decoded.objects[0].states[0].functions[0].instructions.len(),
        2
    );
    assert!(
        decoded.debug_info.unwrap().functions[0]
            .instruction_line_map
            .is_empty()
    );
}

#[test]
fn nonempty_line_maps_still_require_one_entry_per_instruction() {
    assert!(matches!(
        PexFile::read_from_slice(&carrier(&[17, 18], 3)),
        Err(PexReadError::InvalidStructure { .. })
    ));
    let mut file = PexFile::read_from_slice(&carrier(&[17], 3)).unwrap();
    file.objects[0].states[0].functions[0]
        .instructions
        .insert(0, PexInstruction::new(PexOpcode::Nop, vec![]).unwrap());
    assert!(matches!(
        file.write_to_vec(),
        Err(PexWriteError::DebugLineMapLengthMismatch {
            expected: 2,
            actual: 1,
            ..
        })
    ));
}

#[test]
fn empty_line_maps_do_not_bypass_function_or_instruction_validation() {
    // Valid string IDs that do not name a function remain invalid debug references.
    for name in [0, 1, 2] {
        assert!(matches!(
            PexFile::read_from_slice(&carrier(&[], name)),
            Err(PexReadError::InvalidStructure { .. })
        ));
    }
    let mut bytes = carrier(&[], 3);
    bytes.pop(); // Truncate Return's operand.
    assert!(matches!(
        PexFile::read_from_slice(&bytes),
        Err(PexReadError::Truncated { .. })
    ));
    let mut file = PexFile::read_from_slice(&carrier(&[], 3)).unwrap();
    file.objects[0].states[0].functions[0].instructions[0]
        .arguments
        .clear();
    assert!(matches!(
        file.write_to_vec(),
        Err(PexWriteError::InvalidInstructionArity { .. })
    ));
}
