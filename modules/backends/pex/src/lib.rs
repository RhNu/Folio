//! Skyrim PEX emission from validated, target-legal MIR.

use std::collections::{BTreeMap, BTreeSet};

use folio_diagnostics::{Diagnostic, Severity};
use folio_format_pex::{
    PexDebugFunctionInfo, PexDebugFunctionType, PexDebugInfo, PexFile, PexFunction, PexHeader,
    PexInstruction, PexLocal, PexObject, PexOpcode, PexParameter, PexProperty, PexState,
    PexStringId, PexUserFlag, PexValue, PexVariable,
};
use folio_mir::{BinaryOp, Function, Op, Script, UnaryOp, Value};
use folio_source::{FileId, LineIndex, SourceSpan};

/// Explicit metadata supplied by project build planning for reproducible PEX.
#[derive(Clone, Debug, Default)]
pub struct EmissionOptions {
    pub source_file_name: String,
    pub user_name: String,
    pub computer_name: String,
    pub compilation_time: u64,
    pub debug_info: bool,
    pub source_text: BTreeMap<FileId, String>,
    pub user_flags: Vec<(String, u8)>,
}

/// Encode one script after target legalization and MIR validation.
pub fn emit(script: &Script, options: &EmissionOptions) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let span = tracing::debug_span!("emit_pex", script = %script.name, target = script.target.id);
    let _entered = span.enter();
    if script.target.id != "skyrim-se"
        || (script.target.pex_major, script.target.pex_minor) != (3, 2)
    {
        return Err(vec![error(
            script.source,
            "pex.target",
            "Skyrim SE PEX 3.2 backend is unavailable for this target",
        )]);
    }
    if let Err(errors) = folio_mir::validate(script) {
        return Err(errors
            .into_iter()
            .map(|entry| error(entry.source, "pex.mir", entry.kind.to_string()))
            .collect());
    }
    let mut emitter = Emitter::new(script, options);
    let object = match emitter.object() {
        Ok(object) => object,
        Err(err) => return Err(vec![err]),
    };
    emitter.file.objects.push(object);
    if options.debug_info {
        emitter.file.debug_info = Some(PexDebugInfo {
            modification_time: options.compilation_time,
            functions: emitter.debug_functions,
        });
    }
    emitter
        .file
        .write_to_vec()
        .map_err(|err| vec![error(script.source, "pex.encode", err.to_string())])
}

fn error(source: SourceSpan, code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, Severity::Error, message).at(source)
}

struct Emitter<'a> {
    script: &'a Script,
    options: &'a EmissionOptions,
    file: PexFile,
    debug_functions: Vec<PexDebugFunctionInfo>,
    line_indices: BTreeMap<FileId, LineIndex>,
}

impl<'a> Emitter<'a> {
    fn new(script: &'a Script, options: &'a EmissionOptions) -> Self {
        let file = PexFile::new(PexHeader::skyrim(
            options.compilation_time,
            &options.source_file_name,
            &options.user_name,
            &options.computer_name,
        ));
        let line_indices = options
            .source_text
            .iter()
            .map(|(id, text)| (*id, LineIndex::new(text)))
            .collect();
        Self {
            script,
            options,
            file,
            debug_functions: Vec::new(),
            line_indices,
        }
    }

    fn intern(&mut self, text: &str, source: SourceSpan) -> Result<PexStringId, Diagnostic> {
        self.file
            .intern(text)
            .map_err(|err| error(source, "pex.resource", err.to_string()))
    }

    fn value(&mut self, value: &Value, source: SourceSpan) -> Result<PexValue, Diagnostic> {
        Ok(match value {
            Value::None => PexValue::None,
            Value::Bool(v) => PexValue::Bool(*v),
            Value::Int(v) => PexValue::Integer(*v),
            Value::Float(v) => PexValue::Float(*v),
            Value::String(v) => PexValue::String(self.intern(v, source)?),
            Value::Identifier(v) => PexValue::Identifier(self.intern(v, source)?),
        })
    }

    fn instruction(
        &mut self,
        opcode: PexOpcode,
        args: Vec<PexValue>,
        extra: Vec<PexValue>,
        source: SourceSpan,
    ) -> Result<PexInstruction, Diagnostic> {
        PexInstruction::new_variadic(opcode, args, extra)
            .map_err(|err| error(source, "pex.instruction", err.to_string()))
    }

    fn object(&mut self) -> Result<PexObject, Diagnostic> {
        let source = self.script.source;
        let empty = self.intern("", source)?;
        let name = self.intern(&self.script.name, source)?;
        let parent_class_name = self.intern(&self.script.parent, source)?;
        let auto_state_name = self.intern(&self.script.auto_state, source)?;
        let mut represented_flags = 0b11u32;
        let mut flag_names = BTreeSet::from(["hidden".to_owned(), "conditional".to_owned()]);
        for (flag_name, bit_index) in [("Hidden", 0), ("Conditional", 1)] {
            let flag_name = self.intern(flag_name, source)?;
            self.file.user_flags.push(PexUserFlag {
                name: flag_name,
                bit_index,
            });
        }
        for (flag_name, bit_index) in &self.options.user_flags {
            if !flag_names.insert(flag_name.to_ascii_lowercase()) {
                return Err(error(
                    source,
                    "pex.flag",
                    "duplicate or reserved user flag name",
                ));
            }
            let mask = 1u32
                .checked_shl(u32::from(*bit_index))
                .ok_or_else(|| error(source, "pex.flag", "user flag bit must be below 32"))?;
            if represented_flags & mask != 0 {
                return Err(error(source, "pex.flag", "duplicate user flag bit"));
            }
            represented_flags |= mask;
            let name = self.intern(flag_name, source)?;
            self.file.user_flags.push(PexUserFlag {
                name,
                bit_index: *bit_index,
            });
        }
        let used_flags = self.script.flags
            | self
                .script
                .variables
                .iter()
                .fold(0, |bits, var| bits | var.flags)
            | self
                .script
                .functions
                .iter()
                .fold(0, |bits, fun| bits | fun.flags)
            | self.script.properties.iter().fold(0, |bits, prop| {
                bits | prop.flags
                    | prop.getter.as_ref().map_or(0, |fun| fun.flags)
                    | prop.setter.as_ref().map_or(0, |fun| fun.flags)
            });
        if used_flags & !represented_flags != 0 {
            return Err(error(
                source,
                "pex.flag",
                "MIR uses user flag bits missing from the PEX flag table",
            ));
        }

        let mut variables = Vec::new();
        for var in &self.script.variables {
            variables.push(PexVariable {
                name: self.intern(&var.name, var.source)?,
                type_name: self.intern(&var.ty, var.source)?,
                user_flags: var.flags,
                default_value: self.value(&var.initial, var.source)?,
            });
        }
        let mut states = vec![PexState {
            name: empty,
            functions: self.state_runtime()?,
        }];
        for function in self.script.functions.iter().filter(|f| f.state.is_empty()) {
            let (compiled, lines) = self.function(function)?;
            self.push_debug(
                name,
                empty,
                compiled.name,
                PexDebugFunctionType::Normal,
                lines,
            );
            states[0].functions.push(compiled);
        }
        for state_name in &self.script.state_names {
            let state_id = self.intern(state_name, source)?;
            let mut functions = Vec::new();
            for function in self
                .script
                .functions
                .iter()
                .filter(|f| f.state.eq_ignore_ascii_case(state_name))
            {
                let (compiled, lines) = self.function(function)?;
                self.push_debug(
                    name,
                    state_id,
                    compiled.name,
                    PexDebugFunctionType::Normal,
                    lines,
                );
                functions.push(compiled);
            }
            states.push(PexState {
                name: state_id,
                functions,
            });
        }
        let mut properties = Vec::new();
        for prop in &self.script.properties {
            let prop_name = self.intern(&prop.name, prop.source)?;
            let type_name = self.intern(&prop.ty, prop.source)?;
            let (read_function, write_function, is_auto, auto_var) =
                if let Some(var) = &prop.auto_var {
                    if !self
                        .script
                        .variables
                        .iter()
                        .any(|candidate| candidate.name.eq_ignore_ascii_case(var))
                    {
                        return Err(error(
                            prop.source,
                            "pex.property",
                            "auto property backing variable is missing",
                        ));
                    }
                    let var_id = self.intern(var, prop.source)?;
                    if prop.read_only {
                        let return_instruction = self.instruction(
                            PexOpcode::Return,
                            vec![PexValue::Identifier(var_id)],
                            vec![],
                            prop.source,
                        )?;
                        let getter = PexFunction {
                            name: prop_name,
                            return_type_name: type_name,
                            documentation_string: empty,
                            user_flags: 0,
                            is_global: false,
                            is_native: false,
                            parameters: vec![],
                            locals: vec![],
                            instructions: vec![return_instruction],
                        };
                        self.push_debug(
                            name,
                            empty,
                            prop_name,
                            PexDebugFunctionType::Getter,
                            vec![self.line(prop.source)?],
                        );
                        (Some(getter), None, false, None)
                    } else {
                        (None, None, true, Some(var_id))
                    }
                } else {
                    let getter = match &prop.getter {
                        Some(function) => {
                            let (compiled, lines) = self.function(function)?;
                            self.push_debug(
                                name,
                                empty,
                                prop_name,
                                PexDebugFunctionType::Getter,
                                lines,
                            );
                            Some(compiled)
                        }
                        None => None,
                    };
                    let setter = match &prop.setter {
                        Some(function) => {
                            let (compiled, lines) = self.function(function)?;
                            self.push_debug(
                                name,
                                empty,
                                prop_name,
                                PexDebugFunctionType::Setter,
                                lines,
                            );
                            Some(compiled)
                        }
                        None => None,
                    };
                    (getter, setter, false, None)
                };
            properties.push(PexProperty {
                name: prop_name,
                type_name,
                documentation_string: empty,
                user_flags: prop.flags,
                is_readable: is_auto || read_function.is_some(),
                is_writable: (is_auto && !prop.read_only) || write_function.is_some(),
                is_auto,
                auto_var,
                read_function,
                write_function,
            });
        }
        Ok(PexObject {
            name,
            parent_class_name,
            documentation_string: empty,
            user_flags: self.script.flags,
            auto_state_name,
            variables,
            properties,
            states,
        })
    }

    fn push_debug(
        &mut self,
        object_name: PexStringId,
        state_name: PexStringId,
        function_name: PexStringId,
        function_type: PexDebugFunctionType,
        lines: Vec<u16>,
    ) {
        if self.options.debug_info {
            self.debug_functions.push(PexDebugFunctionInfo {
                object_name,
                state_name,
                function_name,
                function_type,
                instruction_line_map: lines,
            });
        }
    }

    fn function(&mut self, function: &Function) -> Result<(PexFunction, Vec<u16>), Diagnostic> {
        let source = function.source;
        let name = self.intern(&function.name, source)?;
        let return_type_name = self.intern(&function.return_type, source)?;
        let documentation_string = self.intern("", source)?;
        let mut parameters = Vec::new();
        for parameter in &function.parameters {
            parameters.push(PexParameter {
                name: self.intern(&parameter.name, source)?,
                type_name: self.intern(&parameter.ty, source)?,
            });
        }
        let mut locals = Vec::new();
        for local in &function.locals {
            locals.push(PexLocal {
                name: self.intern(&local.name, source)?,
                type_name: self.intern(&local.ty, source)?,
            });
        }
        let (instructions, lines) = self.instructions(function)?;
        Ok((
            PexFunction {
                name,
                return_type_name,
                documentation_string,
                user_flags: function.flags,
                is_global: function.is_global,
                is_native: function.is_native,
                parameters,
                locals,
                instructions,
            },
            lines,
        ))
    }

    fn instructions(
        &mut self,
        function: &Function,
    ) -> Result<(Vec<PexInstruction>, Vec<u16>), Diagnostic> {
        let mut labels = BTreeMap::new();
        let mut address = 0usize;
        for item in &function.instructions {
            match item.op {
                Op::Label(id) => {
                    labels.insert(id, address);
                }
                _ => address += 1,
            }
        }
        let mut instructions = Vec::new();
        let mut lines = Vec::new();
        for item in &function.instructions {
            let source = item.source;
            let v = |this: &mut Self, value: &Value| this.value(value, source);
            let (opcode, args, extra) = match &item.op {
                Op::Label(_) => continue,
                Op::Assign(dst, src) => (
                    PexOpcode::Assign,
                    vec![v(self, dst)?, v(self, src)?],
                    vec![],
                ),
                Op::Cast(dst, src) => (PexOpcode::Cast, vec![v(self, dst)?, v(self, src)?], vec![]),
                Op::Unary {
                    operator,
                    dest,
                    value,
                } => {
                    let opcode = match operator {
                        UnaryOp::Not => PexOpcode::Not,
                        UnaryOp::NegInt => PexOpcode::INeg,
                        UnaryOp::NegFloat => PexOpcode::FNeg,
                    };
                    (opcode, vec![v(self, dest)?, v(self, value)?], vec![])
                }
                Op::Binary {
                    operator,
                    dest,
                    left,
                    right,
                } => {
                    let opcode = match operator {
                        BinaryOp::AddInt => PexOpcode::IAdd,
                        BinaryOp::AddFloat => PexOpcode::FAdd,
                        BinaryOp::AddString => PexOpcode::StrCat,
                        BinaryOp::SubInt => PexOpcode::ISub,
                        BinaryOp::SubFloat => PexOpcode::FSub,
                        BinaryOp::MulInt => PexOpcode::IMul,
                        BinaryOp::MulFloat => PexOpcode::FMul,
                        BinaryOp::DivInt => PexOpcode::IDiv,
                        BinaryOp::DivFloat => PexOpcode::FDiv,
                        BinaryOp::ModInt => PexOpcode::IMod,
                        BinaryOp::Eq => PexOpcode::CmpEq,
                        BinaryOp::Lt => PexOpcode::CmpLt,
                        BinaryOp::Lte => PexOpcode::CmpLte,
                        BinaryOp::Gt => PexOpcode::CmpGt,
                        BinaryOp::Gte => PexOpcode::CmpGte,
                    };
                    (
                        opcode,
                        vec![v(self, dest)?, v(self, left)?, v(self, right)?],
                        vec![],
                    )
                }
                Op::CallMethod {
                    name,
                    receiver,
                    dest,
                    args,
                } => {
                    let mut values = vec![
                        PexValue::Identifier(self.intern(name, source)?),
                        v(self, receiver)?,
                        v(self, dest)?,
                    ];
                    (
                        PexOpcode::CallMethod,
                        std::mem::take(&mut values),
                        args.iter()
                            .map(|arg| v(self, arg))
                            .collect::<Result<_, _>>()?,
                    )
                }
                Op::CallParent { name, dest, args } => (
                    PexOpcode::CallParent,
                    vec![
                        PexValue::Identifier(self.intern(name, source)?),
                        v(self, dest)?,
                    ],
                    args.iter()
                        .map(|arg| v(self, arg))
                        .collect::<Result<_, _>>()?,
                ),
                Op::CallStatic {
                    script,
                    name,
                    dest,
                    args,
                } => (
                    PexOpcode::CallStatic,
                    vec![
                        PexValue::Identifier(self.intern(script, source)?),
                        PexValue::Identifier(self.intern(name, source)?),
                        v(self, dest)?,
                    ],
                    args.iter()
                        .map(|arg| v(self, arg))
                        .collect::<Result<_, _>>()?,
                ),
                Op::PropertyGet {
                    name,
                    receiver,
                    dest,
                } => (
                    PexOpcode::PropGet,
                    vec![
                        PexValue::Identifier(self.intern(name, source)?),
                        v(self, receiver)?,
                        v(self, dest)?,
                    ],
                    vec![],
                ),
                Op::PropertySet {
                    name,
                    receiver,
                    value,
                } => (
                    PexOpcode::PropSet,
                    vec![
                        PexValue::Identifier(self.intern(name, source)?),
                        v(self, receiver)?,
                        v(self, value)?,
                    ],
                    vec![],
                ),
                Op::ArrayCreate { dest, length } => (
                    PexOpcode::ArrayCreate,
                    vec![v(self, dest)?, v(self, length)?],
                    vec![],
                ),
                Op::ArrayLength { dest, array } => (
                    PexOpcode::ArrayLength,
                    vec![v(self, dest)?, v(self, array)?],
                    vec![],
                ),
                Op::ArrayGet { dest, array, index } => (
                    PexOpcode::ArrayGetElement,
                    vec![v(self, dest)?, v(self, array)?, v(self, index)?],
                    vec![],
                ),
                Op::ArraySet {
                    array,
                    index,
                    value,
                } => (
                    PexOpcode::ArraySetElement,
                    vec![v(self, array)?, v(self, index)?, v(self, value)?],
                    vec![],
                ),
                Op::ArrayFind {
                    reverse,
                    dest,
                    array,
                    value,
                    start,
                } => (
                    if *reverse {
                        PexOpcode::ArrayRFindElement
                    } else {
                        PexOpcode::ArrayFindElement
                    },
                    vec![
                        v(self, array)?,
                        v(self, dest)?,
                        v(self, value)?,
                        v(self, start)?,
                    ],
                    vec![],
                ),
                Op::Jump(target) => (
                    PexOpcode::Jmp,
                    vec![PexValue::Integer(self.branch_offset(
                        &labels,
                        instructions.len(),
                        *target,
                        source,
                    )?)],
                    vec![],
                ),
                Op::JumpIf {
                    when_true,
                    condition,
                    target,
                } => (
                    if *when_true {
                        PexOpcode::JmpT
                    } else {
                        PexOpcode::JmpF
                    },
                    vec![
                        v(self, condition)?,
                        PexValue::Integer(self.branch_offset(
                            &labels,
                            instructions.len(),
                            *target,
                            source,
                        )?),
                    ],
                    vec![],
                ),
                Op::Return(value) => (PexOpcode::Return, vec![v(self, value)?], vec![]),
            };
            instructions.push(self.instruction(opcode, args, extra, source)?);
            lines.push(if self.options.debug_info {
                self.line(source)?
            } else {
                0
            });
        }
        Ok((instructions, lines))
    }

    fn branch_offset(
        &self,
        labels: &BTreeMap<u32, usize>,
        current: usize,
        target: u32,
        source: SourceSpan,
    ) -> Result<i32, Diagnostic> {
        let destination = labels
            .get(&target)
            .ok_or_else(|| error(source, "pex.branch", "unknown branch target"))?;
        let delta = (*destination as i64) - (current as i64);
        i32::try_from(delta).map_err(|_| {
            error(
                source,
                "pex.branch",
                "branch offset exceeds 32-bit PEX range",
            )
        })
    }

    fn line(&self, source: SourceSpan) -> Result<u16, Diagnostic> {
        let Some(index) = self.line_indices.get(&source.file) else {
            return Ok(0);
        };
        let Some((line, _)) = index.line_col(source.range.start) else {
            return Err(error(
                source,
                "pex.debug",
                "source span lies outside debug source text",
            ));
        };
        u16::try_from(line + 1)
            .map_err(|_| error(source, "pex.debug", "source line exceeds 16-bit PEX range"))
    }

    fn state_runtime(&mut self) -> Result<Vec<PexFunction>, Diagnostic> {
        let source = self.script.source;
        let state = self.intern("::State", source)?;
        let none_type = self.intern("None", source)?;
        let string_type = self.intern("String", source)?;
        let get_name = self.intern("GetState", source)?;
        let goto_name = self.intern("GotoState", source)?;
        let get_doc = self.intern("Function that returns the current state", source)?;
        let goto_doc = self.intern(
            "Function that switches this object to the specified state",
            source,
        )?;
        let get_return = self.instruction(
            PexOpcode::Return,
            vec![PexValue::Identifier(state)],
            vec![],
            source,
        )?;
        let get = PexFunction {
            name: get_name,
            return_type_name: string_type,
            documentation_string: get_doc,
            user_flags: 0,
            is_global: false,
            is_native: false,
            parameters: vec![],
            locals: vec![],
            instructions: vec![get_return],
        };
        let new_state = self.intern("newState", source)?;
        let none_var = self.intern("::NoneVar", source)?;
        let self_name = self.intern("self", source)?;
        let on_end = self.intern("onEndState", source)?;
        let on_begin = self.intern("onBeginState", source)?;
        let goto_instructions = vec![
            self.instruction(
                PexOpcode::CallMethod,
                vec![
                    PexValue::Identifier(on_end),
                    PexValue::Identifier(self_name),
                    PexValue::Identifier(none_var),
                ],
                vec![],
                source,
            )?,
            self.instruction(
                PexOpcode::Assign,
                vec![PexValue::Identifier(state), PexValue::Identifier(new_state)],
                vec![],
                source,
            )?,
            self.instruction(
                PexOpcode::CallMethod,
                vec![
                    PexValue::Identifier(on_begin),
                    PexValue::Identifier(self_name),
                    PexValue::Identifier(none_var),
                ],
                vec![],
                source,
            )?,
        ];
        let goto = PexFunction {
            name: goto_name,
            return_type_name: none_type,
            documentation_string: goto_doc,
            user_flags: 0,
            is_global: false,
            is_native: false,
            parameters: vec![PexParameter {
                name: new_state,
                type_name: string_type,
            }],
            locals: vec![PexLocal {
                name: none_var,
                type_name: none_type,
            }],
            instructions: goto_instructions,
        };
        Ok(vec![get, goto])
    }
}
