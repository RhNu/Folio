use super::*;

pub(crate) fn validate_for_write(file: &PexFile) -> Result<(), PexWriteError> {
    if !file
        .header
        .target
        .supports_pex_version(file.header.pex_version)
    {
        return Err(PexWriteError::UnsupportedVersion {
            major: file.header.pex_version.major(),
            minor: file.header.pex_version.minor(),
        });
    }

    validate_counted_str("source file name", &file.header.source_file_name)?;
    validate_counted_str("user name", &file.header.user_name)?;
    validate_counted_str("computer name", &file.header.computer_name)?;
    ensure_u16("string table", file.strings.len())?;
    for string in &file.strings {
        validate_counted_str("string table entry", string)?;
    }

    let string_table_len = file.strings.len();
    ensure_u16("user flag table", file.user_flags.len())?;
    for user_flag in &file.user_flags {
        validate_string_id(user_flag.name, string_table_len)?;
        if user_flag.bit_index >= 32 {
            return Err(PexWriteError::InvalidUserFlagBit {
                bit_index: user_flag.bit_index,
            });
        }
    }

    ensure_u16("object table", file.objects.len())?;
    for object in &file.objects {
        validate_object_for_write(object, string_table_len, &file.strings)?;
    }
    if let Some(debug_info) = &file.debug_info {
        validate_debug_info_for_write(debug_info, &file.objects, &file.strings)?;
    }

    Ok(())
}

pub(crate) fn validate_counted_str(what: &'static str, text: &str) -> Result<(), PexWriteError> {
    if text.len() > u16::MAX as usize {
        return Err(PexWriteError::StringTooLong {
            what,
            len: text.len(),
        });
    }
    Ok(())
}

pub(crate) fn validate_string_id(id: PexStringId, table_len: usize) -> Result<(), PexWriteError> {
    if id.index() as usize >= table_len {
        return Err(PexWriteError::StringIdOutOfRange { id, table_len });
    }
    Ok(())
}

pub(crate) fn string_id_eq(strings: &[String], id: PexStringId, expected: &str) -> bool {
    strings
        .get(id.index() as usize)
        .is_some_and(|text| text == expected)
}

pub(crate) fn validate_debug_info_for_write(
    debug_info: &PexDebugInfo,
    objects: &[PexObject],
    strings: &[String],
) -> Result<(), PexWriteError> {
    ensure_u16("debug function table", debug_info.functions.len())?;
    for function in &debug_info.functions {
        validate_string_id(function.object_name, strings.len())?;
        validate_string_id(function.state_name, strings.len())?;
        validate_string_id(function.function_name, strings.len())?;
        ensure_u16("debug line map", function.instruction_line_map.len())?;
        let target = resolve_debug_function(function, objects, strings)?;
        // Distributed SKSE PEX leaves generated state helpers without source lines.
        // An empty map means unavailable debug locations, not missing instructions.
        if !function.instruction_line_map.is_empty()
            && function.instruction_line_map.len() != target.instructions.len()
        {
            return Err(PexWriteError::DebugLineMapLengthMismatch {
                function_name: function.function_name,
                expected: target.instructions.len(),
                actual: function.instruction_line_map.len(),
            });
        }
    }
    Ok(())
}

pub(crate) fn resolve_debug_function<'a>(
    debug_function: &PexDebugFunctionInfo,
    objects: &'a [PexObject],
    strings: &[String],
) -> Result<&'a PexFunction, PexWriteError> {
    let object = objects
        .iter()
        .find(|object| object.name == debug_function.object_name)
        .ok_or_else(|| invalid_debug_reference(debug_function))?;
    match debug_function.function_type {
        PexDebugFunctionType::Normal => object
            .states
            .iter()
            .find(|state| state.name == debug_function.state_name)
            .and_then(|state| {
                state
                    .functions
                    .iter()
                    .find(|function| function.name == debug_function.function_name)
            })
            .ok_or_else(|| invalid_debug_reference(debug_function)),
        PexDebugFunctionType::Getter => {
            if !string_id_eq(strings, debug_function.state_name, "") {
                return Err(invalid_debug_reference(debug_function));
            }
            object
                .properties
                .iter()
                .find(|property| property.name == debug_function.function_name)
                .and_then(|property| property.read_function.as_ref())
                .ok_or_else(|| invalid_debug_reference(debug_function))
        }
        PexDebugFunctionType::Setter => {
            if !string_id_eq(strings, debug_function.state_name, "") {
                return Err(invalid_debug_reference(debug_function));
            }
            object
                .properties
                .iter()
                .find(|property| property.name == debug_function.function_name)
                .and_then(|property| property.write_function.as_ref())
                .ok_or_else(|| invalid_debug_reference(debug_function))
        }
    }
}

pub(crate) fn invalid_debug_reference(function: &PexDebugFunctionInfo) -> PexWriteError {
    PexWriteError::InvalidDebugFunctionReference {
        object_name: function.object_name,
        state_name: function.state_name,
        function_name: function.function_name,
        function_type: function.function_type,
    }
}

pub(crate) fn validate_object_for_write(
    object: &PexObject,
    string_table_len: usize,
    strings: &[String],
) -> Result<(), PexWriteError> {
    validate_string_id(object.name, string_table_len)?;
    validate_string_id(object.parent_class_name, string_table_len)?;
    validate_string_id(object.documentation_string, string_table_len)?;
    validate_string_id(object.auto_state_name, string_table_len)?;
    let body_len = encoded_object_body_len(object, string_table_len, strings)?;
    if body_len > u32::MAX as usize {
        return Err(PexWriteError::ObjectTooLarge { len: body_len });
    }
    Ok(())
}

pub(crate) fn validate_variable_for_write(
    variable: &PexVariable,
    string_table_len: usize,
) -> Result<(), PexWriteError> {
    validate_string_id(variable.name, string_table_len)?;
    validate_string_id(variable.type_name, string_table_len)?;
    validate_value_for_write(variable.default_value, string_table_len)
}

pub(crate) fn validate_property_for_write(
    property: &PexProperty,
    string_table_len: usize,
    strings: &[String],
) -> Result<(), PexWriteError> {
    validate_string_id(property.name, string_table_len)?;
    validate_string_id(property.type_name, string_table_len)?;
    validate_string_id(property.documentation_string, string_table_len)?;
    if let Some(auto_var) = property.auto_var {
        validate_string_id(auto_var, string_table_len)?;
    }
    if let Some(function) = &property.read_function {
        validate_function_for_write(function, string_table_len)?;
    }
    if let Some(function) = &property.write_function {
        validate_function_for_write(function, string_table_len)?;
    }
    validate_property_model(property, strings)?;
    Ok(())
}

pub(crate) fn validate_state_for_write(
    state: &PexState,
    string_table_len: usize,
) -> Result<(), PexWriteError> {
    validate_string_id(state.name, string_table_len)?;
    ensure_u16("function table", state.functions.len())?;
    for function in &state.functions {
        validate_function_for_write(function, string_table_len)?;
    }
    Ok(())
}

pub(crate) fn validate_function_for_write(
    function: &PexFunction,
    string_table_len: usize,
) -> Result<(), PexWriteError> {
    validate_string_id(function.name, string_table_len)?;
    validate_string_id(function.return_type_name, string_table_len)?;
    validate_string_id(function.documentation_string, string_table_len)?;
    ensure_u16("parameter table", function.parameters.len())?;
    for parameter in &function.parameters {
        validate_string_id(parameter.name, string_table_len)?;
        validate_string_id(parameter.type_name, string_table_len)?;
    }
    ensure_u16("local table", function.locals.len())?;
    for local in &function.locals {
        validate_string_id(local.name, string_table_len)?;
        validate_string_id(local.type_name, string_table_len)?;
    }
    ensure_u16("instruction table", function.instructions.len())?;
    for instruction in &function.instructions {
        validate_instruction_for_write(instruction, string_table_len)?;
    }
    Ok(())
}

pub(crate) fn validate_instruction_for_write(
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
    if instruction.variadic_arguments.len() > i32::MAX as usize {
        return Err(PexWriteError::VariadicArgumentCountTooLarge {
            len: instruction.variadic_arguments.len(),
        });
    }
    for argument in &instruction.arguments {
        validate_value_for_write(*argument, string_table_len)?;
    }
    for argument in &instruction.variadic_arguments {
        validate_value_for_write(*argument, string_table_len)?;
    }
    Ok(())
}

pub(crate) fn validate_value_for_write(
    value: PexValue,
    string_table_len: usize,
) -> Result<(), PexWriteError> {
    match value {
        PexValue::Identifier(id) | PexValue::String(id) => validate_string_id(id, string_table_len),
        PexValue::None | PexValue::Integer(_) | PexValue::Float(_) | PexValue::Bool(_) => Ok(()),
    }
}

pub(crate) fn encoded_object_body_len(
    object: &PexObject,
    string_table_len: usize,
    strings: &[String],
) -> Result<usize, PexWriteError> {
    let mut len = 2 + 2 + 4 + 2;
    ensure_u16("object variable table", object.variables.len())?;
    len = add_len(len, 2)?;
    for variable in &object.variables {
        validate_variable_for_write(variable, string_table_len)?;
        len = add_len(len, encoded_variable_len(variable))?;
    }
    ensure_u16("property table", object.properties.len())?;
    len = add_len(len, 2)?;
    for property in &object.properties {
        validate_property_for_write(property, string_table_len, strings)?;
        len = add_len(len, encoded_property_len(property)?)?;
    }
    ensure_u16("state table", object.states.len())?;
    len = add_len(len, 2)?;
    for state in &object.states {
        validate_state_for_write(state, string_table_len)?;
        len = add_len(len, encoded_state_len(state)?)?;
    }
    Ok(len)
}

pub(crate) fn encoded_variable_len(variable: &PexVariable) -> usize {
    2 + 2 + 4 + encoded_value_len(variable.default_value)
}

pub(crate) fn encoded_property_len(property: &PexProperty) -> Result<usize, PexWriteError> {
    let mut len = 2 + 2 + 2 + 4 + 1;
    if property.is_auto {
        return add_len(len, 2);
    }
    if property.is_readable
        && let Some(function) = &property.read_function
    {
        len = add_len(len, encoded_function_len(function, false)?)?;
    }
    if property.is_writable
        && let Some(function) = &property.write_function
    {
        len = add_len(len, encoded_function_len(function, false)?)?;
    }
    Ok(len)
}

pub(crate) fn encoded_state_len(state: &PexState) -> Result<usize, PexWriteError> {
    let mut len = 2 + 2;
    for function in &state.functions {
        len = add_len(len, encoded_function_len(function, true)?)?;
    }
    Ok(len)
}

pub(crate) fn encoded_function_len(
    function: &PexFunction,
    include_name: bool,
) -> Result<usize, PexWriteError> {
    let mut len = if include_name { 2 } else { 0 };
    len = add_len(len, 2 + 2 + 4 + 1)?;
    len = add_len(len, 2 + function.parameters.len() * 4)?;
    len = add_len(len, 2 + function.locals.len() * 4)?;
    len = add_len(len, 2)?;
    for instruction in &function.instructions {
        len = add_len(len, encoded_instruction_len(instruction)?)?;
    }
    Ok(len)
}

pub(crate) fn encoded_instruction_len(
    instruction: &PexInstruction,
) -> Result<usize, PexWriteError> {
    let mut len = 1;
    for argument in &instruction.arguments {
        len = add_len(len, encoded_value_len(*argument))?;
    }
    if instruction.opcode.has_variadic_arguments() {
        len = add_len(
            len,
            encoded_value_len(PexValue::Integer(
                instruction.variadic_arguments.len() as i32
            )),
        )?;
        for argument in &instruction.variadic_arguments {
            len = add_len(len, encoded_value_len(*argument))?;
        }
    }
    Ok(len)
}

pub(crate) const fn encoded_value_len(value: PexValue) -> usize {
    match value {
        PexValue::None => 1,
        PexValue::Identifier(_) | PexValue::String(_) => 3,
        PexValue::Integer(_) | PexValue::Float(_) => 5,
        PexValue::Bool(_) => 2,
    }
}

pub(crate) fn add_len(lhs: usize, rhs: usize) -> Result<usize, PexWriteError> {
    lhs.checked_add(rhs)
        .ok_or(PexWriteError::ObjectTooLarge { len: usize::MAX })
}

pub(crate) fn escape_dump_text(text: &str) -> String {
    text.chars().fold(String::new(), |mut escaped, ch| {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
        escaped
    })
}

pub(crate) fn ensure_u16(what: &'static str, len: usize) -> Result<(), PexWriteError> {
    if len > u16::MAX as usize {
        return Err(PexWriteError::CountTooLarge { what, len });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
