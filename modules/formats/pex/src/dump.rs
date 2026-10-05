use super::{PexFile, PexFunction, PexStringId, PexValue, escape_dump_text};

impl PexFile {
    pub fn metadata_dump(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "header target={} version={}.{} game={} source={}",
            self.header.target().id(),
            self.header.pex_version().major(),
            self.header.pex_version().minor(),
            self.header.target().pex_game_id(),
            self.header.source_file_name()
        ));
        lines.push(format!("strings {}", self.strings.len()));
        if let Some(debug_info) = &self.debug_info {
            for function in &debug_info.functions {
                lines.push(format!(
                    "debug object={} state={} function={} kind={:?} lines={:?}",
                    self.string(function.object_name),
                    self.string(function.state_name),
                    self.string(function.function_name),
                    function.function_type,
                    function.instruction_line_map
                ));
            }
        }
        for user_flag in &self.user_flags {
            lines.push(format!(
                "user_flag {} bit={}",
                self.string(user_flag.name),
                user_flag.bit_index
            ));
        }
        for object in &self.objects {
            lines.push(format!(
                "object {} flags=0x{:08x} parent={} doc=\"{}\" auto_state={}",
                self.string(object.name),
                object.user_flags,
                self.string(object.parent_class_name),
                escape_dump_text(self.string(object.documentation_string)),
                self.string(object.auto_state_name)
            ));
            for variable in &object.variables {
                lines.push(format!(
                    "variable {} type={} flags=0x{:08x} default={}",
                    self.string(variable.name),
                    self.string(variable.type_name),
                    variable.user_flags,
                    self.value_text(variable.default_value)
                ));
            }
            for property in &object.properties {
                lines.push(format!(
                    "property {} type={} flags=0x{:08x} readable={} writable={} auto={} doc=\"{}\"",
                    self.string(property.name),
                    self.string(property.type_name),
                    property.user_flags,
                    property.is_readable,
                    property.is_writable,
                    property.is_auto,
                    escape_dump_text(self.string(property.documentation_string))
                ));
                if let Some(auto_var) = property.auto_var {
                    lines.push(format!("auto_var {}", self.string(auto_var)));
                }
                if let Some(function) = &property.read_function {
                    self.push_function_dump("getter", function, &mut lines);
                }
                if let Some(function) = &property.write_function {
                    self.push_function_dump("setter", function, &mut lines);
                }
            }
            for state in &object.states {
                lines.push(format!("state {}", self.string(state.name)));
                for function in &state.functions {
                    self.push_function_dump("function", function, &mut lines);
                }
            }
        }
        lines.join("\n")
    }

    fn push_function_dump(&self, label: &str, function: &PexFunction, lines: &mut Vec<String>) {
        lines.push(format!(
            "{label} {} returns={} flags=0x{:08x} global={} native={} doc=\"{}\"",
            self.string(function.name),
            self.string(function.return_type_name),
            function.user_flags,
            function.is_global,
            function.is_native,
            escape_dump_text(self.string(function.documentation_string))
        ));
        for parameter in &function.parameters {
            lines.push(format!(
                "parameter {}:{}",
                self.string(parameter.name),
                self.string(parameter.type_name)
            ));
        }
        for local in &function.locals {
            lines.push(format!(
                "local {}:{}",
                self.string(local.name),
                self.string(local.type_name)
            ));
        }
        for instruction in &function.instructions {
            let mut args = instruction
                .arguments
                .iter()
                .map(|value| self.value_text(*value))
                .collect::<Vec<_>>();
            args.extend(
                instruction
                    .variadic_arguments
                    .iter()
                    .map(|value| self.value_text(*value)),
            );
            lines.push(format!(
                "opcode {} {}",
                instruction.opcode.name(),
                args.join(", ")
            ));
        }
    }

    fn string(&self, id: PexStringId) -> &str {
        self.strings
            .get(usize::from(id.index()))
            .map_or("<invalid-string>", String::as_str)
    }

    fn value_text(&self, value: PexValue) -> String {
        match value {
            PexValue::None => "None".to_owned(),
            PexValue::Identifier(id) => self.string(id).to_owned(),
            PexValue::String(id) => format!("\"{}\"", escape_dump_text(self.string(id))),
            PexValue::Integer(value) => value.to_string(),
            PexValue::Float(value) => value.to_string(),
            PexValue::Bool(value) => value.to_string(),
        }
    }
}
