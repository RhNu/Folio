use super::*;

impl PexFile {
    pub fn read_from_slice(bytes: &[u8]) -> Result<Self, PexReadError> {
        let _span = tracing::debug_span!("read_pex", bytes = bytes.len()).entered();
        let mut reader = BinaryReader::new(bytes);
        let magic = reader.read_magic()?;
        if magic != PEX_MAGIC {
            return Err(PexReadError::InvalidMagic { value: magic });
        }

        let major = reader.read_u8("major version")?;
        let minor = reader.read_u8("minor version")?;
        let pex_version = PexVersion::new(major, minor);
        let file_game_id = reader.read_u16("game id")?;
        let target =
            PexTarget::from_pex_game_id(file_game_id).ok_or(PexReadError::UnsupportedGame {
                game_id: file_game_id,
            })?;
        if !target.supports_pex_version(pex_version) {
            return Err(PexReadError::UnsupportedVersion { major, minor });
        }

        let compilation_time = reader.read_u64("compilation time")?;
        let source_file_name = reader.read_counted_str("source file name")?;
        let user_name = reader.read_counted_str("user name")?;
        let computer_name = reader.read_counted_str("computer name")?;
        let string_count = usize::from(reader.read_u16("string table count")?);
        let mut strings = Vec::with_capacity(string_count);
        let mut string_lookup = HashMap::new();
        for _ in 0..string_count {
            let string = reader.read_counted_str("string table entry")?;
            let id = PexStringId::new(strings.len() as u16);
            string_lookup.insert(string.clone(), id);
            strings.push(string);
        }

        let debug_flag_offset = reader.offset();
        let has_debug_info = match reader.read_u8("debug info flag")? {
            0 => false,
            1 => true,
            value => {
                return Err(PexReadError::InvalidField {
                    offset: debug_flag_offset,
                    what: "debug info flag",
                    value,
                });
            }
        };
        let debug_info = has_debug_info
            .then(|| read_debug_info(&mut reader, strings.len()))
            .transpose()?;

        let user_flag_count = usize::from(reader.read_u16("user flag table count")?);
        let mut user_flags = Vec::with_capacity(user_flag_count);
        for _ in 0..user_flag_count {
            user_flags.push(PexUserFlag {
                name: reader.read_string_id(strings.len(), "user flag name")?,
                bit_index: reader.read_u8("user flag bit index")?,
            });
        }

        let object_count = usize::from(reader.read_u16("object table count")?);
        let mut objects = Vec::with_capacity(object_count);
        for _ in 0..object_count {
            objects.push(read_object(&mut reader, strings.len())?);
        }

        if reader.remaining() != 0 {
            return Err(PexReadError::TrailingBytes {
                offset: reader.offset(),
                len: bytes.len(),
            });
        }

        let file = Self {
            header: PexHeader::read(
                target,
                pex_version,
                compilation_time,
                source_file_name,
                user_name,
                computer_name,
            ),
            strings,
            string_lookup,
            debug_info,
            user_flags,
            objects,
        };
        validate_for_write(&file).map_err(|error| PexReadError::InvalidStructure {
            reason: error.to_string(),
        })?;
        tracing::debug!(
            target = %file.header().target().id(),
            objects = file.objects.len(),
            strings = file.string_table().len(),
            user_flags = file.user_flags.len(),
            debug_info = file.debug_info.is_some(),
            "finished reading PEX"
        );
        Ok(file)
    }
}

pub(crate) fn read_debug_info(
    reader: &mut BinaryReader<'_>,
    string_table_len: usize,
) -> Result<PexDebugInfo, PexReadError> {
    let modification_time = reader.read_u64("debug modification time")?;
    let count = usize::from(reader.read_u16("debug function count")?);
    let mut functions = Vec::with_capacity(count);
    for _ in 0..count {
        let object_name = reader.read_string_id(string_table_len, "debug object name")?;
        let state_name = reader.read_string_id(string_table_len, "debug state name")?;
        let function_name = reader.read_string_id(string_table_len, "debug function name")?;
        let function_type_offset = reader.offset();
        let function_type = match reader.read_u8("debug function type")? {
            0 => PexDebugFunctionType::Normal,
            1 => PexDebugFunctionType::Getter,
            2 => PexDebugFunctionType::Setter,
            tag => {
                return Err(PexReadError::InvalidDebugFunctionType {
                    offset: function_type_offset,
                    tag,
                });
            }
        };
        let line_count = usize::from(reader.read_u16("debug line map count")?);
        let mut instruction_line_map = Vec::with_capacity(line_count);
        for _ in 0..line_count {
            instruction_line_map.push(reader.read_u16("debug line")?);
        }
        functions.push(PexDebugFunctionInfo {
            object_name,
            state_name,
            function_name,
            function_type,
            instruction_line_map,
        });
    }
    Ok(PexDebugInfo {
        modification_time,
        functions,
    })
}

pub(crate) fn read_object(
    reader: &mut BinaryReader<'_>,
    string_table_len: usize,
) -> Result<PexObject, PexReadError> {
    let object_offset = reader.offset();
    let name = reader.read_string_id(string_table_len, "object name")?;
    let raw_body_len = reader.read_u32("object body length")? as usize;
    let body_start = reader.offset();
    let body_end =
        body_start
            .checked_add(raw_body_len)
            .ok_or(PexReadError::ObjectSizeMismatch {
                offset: object_offset,
                expected_end: usize::MAX,
                actual_end: body_start,
            })?;

    let parent_class_name = reader.read_string_id(string_table_len, "object parent class")?;
    let documentation_string = reader.read_string_id(string_table_len, "object documentation")?;
    let user_flags = reader.read_u32("object user flags")?;
    let auto_state_name = reader.read_string_id(string_table_len, "object auto state")?;

    let variable_count = usize::from(reader.read_u16("variable count")?);
    let mut variables = Vec::with_capacity(variable_count);
    for _ in 0..variable_count {
        variables.push(read_variable(reader, string_table_len)?);
    }

    let property_count = usize::from(reader.read_u16("property count")?);
    let mut properties = Vec::with_capacity(property_count);
    for _ in 0..property_count {
        properties.push(read_property(reader, string_table_len)?);
    }

    let state_count = usize::from(reader.read_u16("state count")?);
    let mut states = Vec::with_capacity(state_count);
    for _ in 0..state_count {
        states.push(read_state(reader, string_table_len)?);
    }

    // CK includes the four-byte size field in some object lengths. Compare
    // against bytes actually consumed so multi-object files work as well.
    let consumed = reader.offset() - body_start;
    if consumed != raw_body_len && consumed.checked_add(4) != Some(raw_body_len) {
        return Err(PexReadError::ObjectSizeMismatch {
            offset: object_offset,
            expected_end: body_end,
            actual_end: reader.offset(),
        });
    }

    Ok(PexObject {
        name,
        parent_class_name,
        documentation_string,
        user_flags,
        auto_state_name,
        variables,
        properties,
        states,
    })
}

pub(crate) fn read_variable(
    reader: &mut BinaryReader<'_>,
    string_table_len: usize,
) -> Result<PexVariable, PexReadError> {
    Ok(PexVariable {
        name: reader.read_string_id(string_table_len, "variable name")?,
        type_name: reader.read_string_id(string_table_len, "variable type")?,
        user_flags: reader.read_u32("variable user flags")?,
        default_value: read_value(reader, string_table_len)?,
    })
}

pub(crate) fn read_property(
    reader: &mut BinaryReader<'_>,
    string_table_len: usize,
) -> Result<PexProperty, PexReadError> {
    let name = reader.read_string_id(string_table_len, "property name")?;
    let type_name = reader.read_string_id(string_table_len, "property type")?;
    let documentation_string = reader.read_string_id(string_table_len, "property documentation")?;
    let user_flags = reader.read_u32("property user flags")?;
    let flags_offset = reader.offset();
    let flags = reader.read_u8("property flags")?;
    if flags & !0x07 != 0 {
        return Err(PexReadError::InvalidField {
            offset: flags_offset,
            what: "property flags",
            value: flags,
        });
    }
    let is_readable = flags & 0x01 != 0;
    let is_writable = flags & 0x02 != 0;
    let is_auto = flags & 0x04 != 0;
    let (auto_var, read_function, write_function) = if is_auto {
        (
            Some(reader.read_string_id(string_table_len, "property auto variable")?),
            None,
            None,
        )
    } else {
        let getter = is_readable
            .then(|| read_function(reader, string_table_len, false, Some(name)))
            .transpose()?;
        let setter = is_writable
            .then(|| read_function(reader, string_table_len, false, Some(name)))
            .transpose()?;
        (None, getter, setter)
    };

    Ok(PexProperty {
        name,
        type_name,
        documentation_string,
        user_flags,
        is_readable,
        is_writable,
        is_auto,
        auto_var,
        read_function,
        write_function,
    })
}

pub(crate) fn read_state(
    reader: &mut BinaryReader<'_>,
    string_table_len: usize,
) -> Result<PexState, PexReadError> {
    let name = reader.read_string_id(string_table_len, "state name")?;
    let count = usize::from(reader.read_u16("state function count")?);
    let mut functions = Vec::with_capacity(count);
    for _ in 0..count {
        functions.push(read_function(reader, string_table_len, true, None)?);
    }
    Ok(PexState { name, functions })
}

pub(crate) fn read_function(
    reader: &mut BinaryReader<'_>,
    string_table_len: usize,
    has_name: bool,
    property_name: Option<PexStringId>,
) -> Result<PexFunction, PexReadError> {
    let name = if has_name {
        reader.read_string_id(string_table_len, "function name")?
    } else {
        property_name.unwrap_or(PexStringId::new(0))
    };
    let return_type_name = reader.read_string_id(string_table_len, "function return type")?;
    let documentation_string = reader.read_string_id(string_table_len, "function documentation")?;
    let user_flags = reader.read_u32("function user flags")?;
    let flags_offset = reader.offset();
    let flags = reader.read_u8("function flags")?;
    if flags & !0x03 != 0 {
        return Err(PexReadError::InvalidField {
            offset: flags_offset,
            what: "function flags",
            value: flags,
        });
    }
    let is_global = flags & 0x01 != 0;
    let is_native = flags & 0x02 != 0;
    let parameter_count = usize::from(reader.read_u16("parameter count")?);
    let mut parameters = Vec::with_capacity(parameter_count);
    for _ in 0..parameter_count {
        parameters.push(PexParameter {
            name: reader.read_string_id(string_table_len, "parameter name")?,
            type_name: reader.read_string_id(string_table_len, "parameter type")?,
        });
    }
    let local_count = usize::from(reader.read_u16("local count")?);
    let mut locals = Vec::with_capacity(local_count);
    for _ in 0..local_count {
        locals.push(PexLocal {
            name: reader.read_string_id(string_table_len, "local name")?,
            type_name: reader.read_string_id(string_table_len, "local type")?,
        });
    }
    let instruction_count = usize::from(reader.read_u16("instruction count")?);
    let mut instructions = Vec::with_capacity(instruction_count);
    for _ in 0..instruction_count {
        instructions.push(read_instruction(reader, string_table_len)?);
    }
    Ok(PexFunction {
        name,
        return_type_name,
        documentation_string,
        user_flags,
        is_global,
        is_native,
        parameters,
        locals,
        instructions,
    })
}

pub(crate) fn read_instruction(
    reader: &mut BinaryReader<'_>,
    string_table_len: usize,
) -> Result<PexInstruction, PexReadError> {
    let opcode_offset = reader.offset();
    let opcode_byte = reader.read_u8("instruction opcode")?;
    let opcode = PexOpcode::from_byte(opcode_byte).ok_or(PexReadError::UnknownOpcode {
        offset: opcode_offset,
        opcode: opcode_byte,
    })?;
    let mut arguments = Vec::with_capacity(opcode.fixed_arg_count());
    for _ in 0..opcode.fixed_arg_count() {
        arguments.push(read_value(reader, string_table_len)?);
    }
    let mut variadic_arguments = Vec::new();
    if opcode.has_variadic_arguments() {
        let count_offset = reader.offset();
        let count = read_value(reader, string_table_len)?;
        let PexValue::Integer(count) = count else {
            return Err(PexReadError::MalformedVariadicCount {
                offset: count_offset,
                opcode,
            });
        };
        let count = usize::try_from(count).map_err(|_| PexReadError::MalformedVariadicCount {
            offset: count_offset,
            opcode,
        })?;
        if count > reader.remaining() {
            return Err(PexReadError::MalformedVariadicCount {
                offset: count_offset,
                opcode,
            });
        }
        for _ in 0..count {
            variadic_arguments.push(read_value(reader, string_table_len)?);
        }
    }
    Ok(PexInstruction {
        opcode,
        arguments,
        variadic_arguments,
    })
}

pub(crate) fn read_value(
    reader: &mut BinaryReader<'_>,
    string_table_len: usize,
) -> Result<PexValue, PexReadError> {
    let tag_offset = reader.offset();
    match reader.read_u8("value type")? {
        0 => Ok(PexValue::None),
        1 => Ok(PexValue::Identifier(
            reader.read_string_id(string_table_len, "identifier value")?,
        )),
        2 => Ok(PexValue::String(
            reader.read_string_id(string_table_len, "string value")?,
        )),
        3 => Ok(PexValue::Integer(reader.read_i32("integer value")?)),
        4 => Ok(PexValue::Float(reader.read_f32("float value")?)),
        5 => {
            let offset = reader.offset();
            match reader.read_u8("bool value")? {
                0 => Ok(PexValue::Bool(false)),
                1 => Ok(PexValue::Bool(true)),
                value => Err(PexReadError::InvalidField {
                    offset,
                    what: "bool",
                    value,
                }),
            }
        }
        tag => Err(PexReadError::UnknownValueType {
            offset: tag_offset,
            tag,
        }),
    }
}
