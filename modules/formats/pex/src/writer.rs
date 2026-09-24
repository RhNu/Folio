use super::*;

impl PexFile {
    pub fn write_to_vec(&self) -> Result<Vec<u8>, PexWriteError> {
        let _span = tracing::debug_span!(
            "write_pex",
            target = %self.header.target().id(),
            objects = self.objects.len(),
            strings = self.strings.len()
        )
        .entered();
        validate_for_write(self)?;

        let mut writer = BinaryWriter::new(self.header.target.endianness());
        writer.write_u32(PEX_MAGIC);
        writer.write_u8(self.header.pex_version.major());
        writer.write_u8(self.header.pex_version.minor());
        writer.write_u16(self.header.target.pex_game_id());
        writer.write_u64(self.header.compilation_time);
        writer.write_counted_str("source file name", &self.header.source_file_name)?;
        writer.write_counted_str("user name", &self.header.user_name)?;
        writer.write_counted_str("computer name", &self.header.computer_name)?;

        writer.write_len_u16("string table", self.strings.len())?;
        for string in &self.strings {
            writer.write_counted_str("string table entry", string)?;
        }

        if let Some(debug_info) = &self.debug_info {
            writer.write_u8(1);
            write_debug_info(&mut writer, debug_info, self.strings.len())?;
        } else {
            writer.write_u8(0);
        }

        writer.write_len_u16("user flag table", self.user_flags.len())?;
        for user_flag in &self.user_flags {
            writer.write_string_id(user_flag.name, self.strings.len())?;
            writer.write_u8(user_flag.bit_index);
        }

        writer.write_len_u16("object table", self.objects.len())?;
        for object in &self.objects {
            write_object(&mut writer, object, self.strings.len())?;
        }

        let bytes = writer.into_bytes();
        tracing::debug!(
            bytes = bytes.len(),
            objects = self.objects.len(),
            strings = self.strings.len(),
            "finished writing PEX"
        );
        Ok(bytes)
    }
}

pub(crate) fn write_debug_info(
    writer: &mut BinaryWriter,
    debug_info: &PexDebugInfo,
    string_table_len: usize,
) -> Result<(), PexWriteError> {
    writer.write_u64(debug_info.modification_time);
    writer.write_len_u16("debug function table", debug_info.functions.len())?;
    for function in &debug_info.functions {
        writer.write_string_id(function.object_name, string_table_len)?;
        writer.write_string_id(function.state_name, string_table_len)?;
        writer.write_string_id(function.function_name, string_table_len)?;
        writer.write_u8(function.function_type as u8);
        writer.write_len_u16("debug line map", function.instruction_line_map.len())?;
        for line in &function.instruction_line_map {
            writer.write_u16(*line);
        }
    }
    Ok(())
}

pub(crate) fn write_object(
    writer: &mut BinaryWriter,
    object: &PexObject,
    string_table_len: usize,
) -> Result<(), PexWriteError> {
    writer.write_string_id(object.name, string_table_len)?;

    let mut body = BinaryWriter::new(writer.endianness);
    body.write_string_id(object.parent_class_name, string_table_len)?;
    body.write_string_id(object.documentation_string, string_table_len)?;
    body.write_user_flags(object.user_flags);
    body.write_string_id(object.auto_state_name, string_table_len)?;

    body.write_len_u16("object variable table", object.variables.len())?;
    for variable in &object.variables {
        write_variable(&mut body, variable, string_table_len)?;
    }

    body.write_len_u16("property table", object.properties.len())?;
    for property in &object.properties {
        write_property(&mut body, property, string_table_len)?;
    }

    body.write_len_u16("state table", object.states.len())?;
    for state in &object.states {
        write_state(&mut body, state, string_table_len)?;
    }

    let body = body.into_bytes();
    if body.len() > u32::MAX as usize {
        return Err(PexWriteError::ObjectTooLarge { len: body.len() });
    }

    writer.write_u32(body.len() as u32);
    writer.bytes.extend(body);
    Ok(())
}

pub(crate) fn write_variable(
    writer: &mut BinaryWriter,
    variable: &PexVariable,
    string_table_len: usize,
) -> Result<(), PexWriteError> {
    writer.write_string_id(variable.name, string_table_len)?;
    writer.write_string_id(variable.type_name, string_table_len)?;
    writer.write_user_flags(variable.user_flags);
    write_value(writer, variable.default_value, string_table_len)
}

pub(crate) fn write_property(
    writer: &mut BinaryWriter,
    property: &PexProperty,
    string_table_len: usize,
) -> Result<(), PexWriteError> {
    writer.write_string_id(property.name, string_table_len)?;
    writer.write_string_id(property.type_name, string_table_len)?;
    writer.write_string_id(property.documentation_string, string_table_len)?;
    writer.write_user_flags(property.user_flags);

    let mut flags = 0u8;
    if property.is_readable {
        flags |= 0x01;
    }
    if property.is_writable {
        flags |= 0x02;
    }
    if property.is_auto {
        flags |= 0x04;
    }
    writer.write_u8(flags);

    if property.is_auto {
        let auto_var = property
            .auto_var
            .ok_or(PexWriteError::AutoPropertyMissingAutoVar)?;
        writer.write_string_id(auto_var, string_table_len)?;
    } else {
        if property.is_readable
            && let Some(function) = &property.read_function
        {
            write_function(writer, function, string_table_len, false)?;
        }
        if property.is_writable
            && let Some(function) = &property.write_function
        {
            write_function(writer, function, string_table_len, false)?;
        }
    }

    Ok(())
}

pub(crate) fn validate_property_model(
    property: &PexProperty,
    strings: &[String],
) -> Result<(), PexWriteError> {
    if property.is_auto {
        if property.auto_var.is_none() {
            return Err(PexWriteError::AutoPropertyMissingAutoVar);
        }
        if !property.is_readable || !property.is_writable {
            return Err(PexWriteError::InvalidPropertyModel {
                property: property.name,
                reason: "auto properties must be readable and writable",
            });
        }
        if property.read_function.is_some() || property.write_function.is_some() {
            return Err(PexWriteError::InvalidPropertyModel {
                property: property.name,
                reason: "auto properties cannot carry accessor functions",
            });
        }
        return Ok(());
    }

    if property.auto_var.is_some() {
        return Err(PexWriteError::InvalidPropertyModel {
            property: property.name,
            reason: "non-auto properties cannot carry an auto backing variable",
        });
    }
    if !property.is_readable && !property.is_writable {
        return Err(PexWriteError::InvalidPropertyModel {
            property: property.name,
            reason: "non-auto properties must expose at least one accessor",
        });
    }
    if property.is_readable && property.read_function.is_none() {
        return Err(PexWriteError::ReadablePropertyMissingGetter);
    }
    if !property.is_readable && property.read_function.is_some() {
        return Err(PexWriteError::InvalidPropertyModel {
            property: property.name,
            reason: "unreadable properties cannot carry a getter function",
        });
    }
    if property.is_writable && property.write_function.is_none() {
        return Err(PexWriteError::WritablePropertyMissingSetter);
    }
    if !property.is_writable && property.write_function.is_some() {
        return Err(PexWriteError::InvalidPropertyModel {
            property: property.name,
            reason: "unwritable properties cannot carry a setter function",
        });
    }
    if let Some(function) = &property.read_function {
        validate_getter_signature(property, function)?;
    }
    if let Some(function) = &property.write_function {
        validate_setter_signature(property, function, strings)?;
    }

    Ok(())
}

pub(crate) fn validate_getter_signature(
    property: &PexProperty,
    function: &PexFunction,
) -> Result<(), PexWriteError> {
    if function.return_type_name != property.type_name {
        return Err(PexWriteError::InvalidAccessorSignature {
            property: property.name,
            accessor: "getter",
            reason: "return type must match property type",
        });
    }
    if !function.parameters.is_empty() {
        return Err(PexWriteError::InvalidAccessorSignature {
            property: property.name,
            accessor: "getter",
            reason: "getter must not have parameters",
        });
    }
    Ok(())
}

pub(crate) fn validate_setter_signature(
    property: &PexProperty,
    function: &PexFunction,
    strings: &[String],
) -> Result<(), PexWriteError> {
    if !string_id_eq(strings, function.return_type_name, "None") {
        return Err(PexWriteError::InvalidAccessorSignature {
            property: property.name,
            accessor: "setter",
            reason: "setter must return None",
        });
    }
    if function.parameters.len() != 1 {
        return Err(PexWriteError::InvalidAccessorSignature {
            property: property.name,
            accessor: "setter",
            reason: "setter must have exactly one parameter",
        });
    }
    if function.parameters[0].type_name != property.type_name {
        return Err(PexWriteError::InvalidAccessorSignature {
            property: property.name,
            accessor: "setter",
            reason: "setter parameter type must match property type",
        });
    }
    Ok(())
}

pub(crate) fn write_state(
    writer: &mut BinaryWriter,
    state: &PexState,
    string_table_len: usize,
) -> Result<(), PexWriteError> {
    writer.write_string_id(state.name, string_table_len)?;
    writer.write_len_u16("function table", state.functions.len())?;
    for function in &state.functions {
        write_function(writer, function, string_table_len, true)?;
    }
    Ok(())
}

pub(crate) fn write_function(
    writer: &mut BinaryWriter,
    function: &PexFunction,
    string_table_len: usize,
    include_name: bool,
) -> Result<(), PexWriteError> {
    if include_name {
        writer.write_string_id(function.name, string_table_len)?;
    }
    writer.write_string_id(function.return_type_name, string_table_len)?;
    writer.write_string_id(function.documentation_string, string_table_len)?;
    writer.write_user_flags(function.user_flags);

    let mut flags = 0u8;
    if function.is_global {
        flags |= 0x01;
    }
    if function.is_native {
        flags |= 0x02;
    }
    writer.write_u8(flags);

    writer.write_len_u16("parameter table", function.parameters.len())?;
    for parameter in &function.parameters {
        writer.write_string_id(parameter.name, string_table_len)?;
        writer.write_string_id(parameter.type_name, string_table_len)?;
    }

    writer.write_len_u16("local table", function.locals.len())?;
    for local in &function.locals {
        writer.write_string_id(local.name, string_table_len)?;
        writer.write_string_id(local.type_name, string_table_len)?;
    }

    writer.write_len_u16("instruction table", function.instructions.len())?;
    for instruction in &function.instructions {
        write_instruction(writer, instruction, string_table_len)?;
    }

    Ok(())
}

pub(crate) fn write_instruction(
    writer: &mut BinaryWriter,
    instruction: &PexInstruction,
    string_table_len: usize,
) -> Result<(), PexWriteError> {
    let expected = instruction.opcode.fixed_arg_count();
    let actual = instruction.arguments.len();
    if actual != expected {
        return Err(PexWriteError::InvalidInstructionArity {
            opcode: instruction.opcode,
            expected,
            actual,
        });
    }

    if !instruction.opcode.has_variadic_arguments() && !instruction.variadic_arguments.is_empty() {
        return Err(PexWriteError::UnexpectedVariadicArguments {
            opcode: instruction.opcode,
        });
    }

    writer.write_u8(instruction.opcode as u8);
    for argument in &instruction.arguments {
        write_value(writer, *argument, string_table_len)?;
    }

    if instruction.opcode.has_variadic_arguments() {
        write_value(
            writer,
            PexValue::Integer(instruction.variadic_arguments.len() as i32),
            string_table_len,
        )?;
        for argument in &instruction.variadic_arguments {
            write_value(writer, *argument, string_table_len)?;
        }
    }

    Ok(())
}

pub(crate) fn write_value(
    writer: &mut BinaryWriter,
    value: PexValue,
    string_table_len: usize,
) -> Result<(), PexWriteError> {
    match value {
        PexValue::None => {
            writer.write_u8(0);
        }
        PexValue::Identifier(id) => {
            writer.write_u8(1);
            writer.write_string_id(id, string_table_len)?;
        }
        PexValue::String(id) => {
            writer.write_u8(2);
            writer.write_string_id(id, string_table_len)?;
        }
        PexValue::Integer(value) => {
            writer.write_u8(3);
            writer.write_i32(value);
        }
        PexValue::Float(value) => {
            writer.write_u8(4);
            writer.write_f32(value);
        }
        PexValue::Bool(value) => {
            writer.write_u8(5);
            writer.write_u8(u8::from(value));
        }
    }

    Ok(())
}
