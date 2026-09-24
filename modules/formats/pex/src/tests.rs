use super::*;

// Hand-assembled Skyrim PEX header and empty tables. This is deliberately
// independent of the writer so a shared encoder/decoder bug cannot pass it.
const EMPTY_SKYRIM_PEX: &[u8] = &[
    0xfa, 0x57, 0xc0, 0xde, // magic
    3, 2, 0, 1, // version and game
    0, 0, 0, 0, 0, 0, 0, 0, // time
    0, 0, 0, 0, 0, 0, // source/user/computer strings
    0, 0, // string table
    0, // no debug info
    0, 0, // user flags
    0, 0, // objects
];

#[test]
fn reads_independent_minimal_bytes() {
    let file = PexFile::read_from_slice(EMPTY_SKYRIM_PEX).unwrap();
    assert_eq!(file.header().pex_version(), PexVersion::new(3, 2));
    assert!(file.objects.is_empty());
    assert_eq!(file.write_to_vec().unwrap(), EMPTY_SKYRIM_PEX);
}

#[test]
fn reads_hand_assembled_object_and_return_instruction() {
    // One object named Test, default state, Run() returning None. Its object
    // body is 39 bytes; no encoder routine contributes to this fixture.
    let hex = "
        fa 57 c0 de 03 02 00 01 00 00 00 00 00 00 00 00
        00 00 00 00 00 00
        00 04
        00 00
        00 04 54 65 73 74
        00 04 4e 6f 6e 65
        00 03 52 75 6e
        00 00 00 00 01
        00 01 00 00 00 27
        00 00 00 00 00 00 00 00 00 00
        00 00 00 00 00 01
        00 00 00 01
        00 03 00 02 00 00 00 00 00 00 00
        00 00 00 00 00 01
        1a 00
    ";
    let bytes = hex
        .split_whitespace()
        .map(|part| u8::from_str_radix(part, 16).unwrap())
        .collect::<Vec<_>>();
    let file = PexFile::read_from_slice(&bytes).unwrap();
    assert_eq!(file.string_table(), ["", "Test", "None", "Run"]);
    assert_eq!(
        file.objects[0].states[0].functions[0].instructions[0].opcode,
        PexOpcode::Return
    );
    assert_eq!(file.write_to_vec().unwrap(), bytes);
}

#[test]
fn rejects_other_game_layout_and_truncation() {
    let mut fallout = EMPTY_SKYRIM_PEX.to_vec();
    fallout[7] = 2;
    assert!(matches!(
        PexFile::read_from_slice(&fallout),
        Err(PexReadError::UnsupportedGame { game_id: 2 })
    ));

    for end in [0, 4, 8, 16, EMPTY_SKYRIM_PEX.len() - 1] {
        assert!(matches!(
            PexFile::read_from_slice(&EMPTY_SKYRIM_PEX[..end]),
            Err(PexReadError::Truncated { .. })
        ));
    }
}

#[test]
fn rejects_invalid_header_flags_and_extra_bytes() {
    let mut bytes = EMPTY_SKYRIM_PEX.to_vec();
    bytes[24] = 2;
    assert!(matches!(
        PexFile::read_from_slice(&bytes),
        Err(PexReadError::InvalidField {
            what: "debug info flag",
            ..
        })
    ));

    let mut bytes = EMPTY_SKYRIM_PEX.to_vec();
    bytes.push(0);
    assert!(matches!(
        PexFile::read_from_slice(&bytes),
        Err(PexReadError::TrailingBytes { .. })
    ));
}

#[test]
fn writes_and_reads_object_function_and_return_instruction() {
    let mut file = PexFile::new(PexHeader::skyrim(0, "Test.psc", "", ""));
    let empty = file.intern("").unwrap();
    let name = file.intern("Test").unwrap();
    let none = file.intern("None").unwrap();
    let run = file.intern("Run").unwrap();
    file.objects.push(PexObject {
        name,
        parent_class_name: empty,
        documentation_string: empty,
        user_flags: 0,
        auto_state_name: empty,
        variables: vec![],
        properties: vec![],
        states: vec![PexState {
            name: empty,
            functions: vec![PexFunction {
                name: run,
                return_type_name: none,
                documentation_string: empty,
                user_flags: 0,
                is_global: false,
                is_native: false,
                parameters: vec![],
                locals: vec![],
                instructions: vec![
                    PexInstruction::new(PexOpcode::Return, vec![PexValue::None]).unwrap(),
                ],
            }],
        }],
    });
    let bytes = file.write_to_vec().unwrap();
    let decoded = PexFile::read_from_slice(&bytes).unwrap();
    assert_eq!(decoded.objects.len(), 1);
    assert_eq!(
        decoded.objects[0].states[0].functions[0].instructions[0].opcode,
        PexOpcode::Return
    );
    assert_eq!(
        decoded.objects[0].states[0].functions[0].instructions[0].arguments,
        vec![PexValue::None]
    );
}

#[test]
fn rejects_invalid_string_reference_before_encoding() {
    let mut file = PexFile::default();
    file.objects.push(PexObject {
        name: PexStringId::new(0),
        parent_class_name: PexStringId::new(0),
        documentation_string: PexStringId::new(0),
        user_flags: 0,
        auto_state_name: PexStringId::new(0),
        variables: vec![],
        properties: vec![],
        states: vec![],
    });
    assert!(matches!(
        file.write_to_vec(),
        Err(PexWriteError::StringIdOutOfRange { .. })
    ));
}
