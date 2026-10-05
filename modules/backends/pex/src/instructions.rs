//! Opcode and control-flow encoding for validated MIR functions.
use super::{
    BTreeMap, Diagnostic, Emitter, Function, Op, PexInstruction, PexOpcode, PexValue, SourceSpan,
    UnaryOp, Value, binary_opcode, error,
};

impl Emitter<'_> {
    pub(super) fn instructions(
        &mut self,
        function: &Function,
    ) -> Result<(Vec<PexInstruction>, Vec<u16>), Diagnostic> {
        let mut labels = BTreeMap::new();
        let mut address = 0usize;
        for item in &function.instructions {
            match item.op {
                Op::Label(id) => {
                    labels.insert(id, address);
                },
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
                },
                Op::Binary {
                    operator,
                    dest,
                    left,
                    right,
                } => {
                    let opcode = binary_opcode(*operator);
                    (
                        opcode,
                        vec![v(self, dest)?, v(self, left)?, v(self, right)?],
                        vec![],
                    )
                },
                Op::CallMethod { .. }
                | Op::CallParent { .. }
                | Op::CallStatic { .. }
                | Op::PropertyGet { .. }
                | Op::PropertySet { .. } => self.call_instruction(&item.op, source)?,
                Op::ArrayCreate { .. }
                | Op::ArrayLength { .. }
                | Op::ArrayGet { .. }
                | Op::ArraySet { .. }
                | Op::ArrayFind { .. } => self.array_instruction(&item.op, source)?,
                Op::Jump(target) => (
                    PexOpcode::Jmp,
                    vec![PexValue::Integer(Self::branch_offset(
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
                        PexValue::Integer(Self::branch_offset(
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
            instructions.push(Self::instruction(opcode, args, extra, source)?);
            lines.push(if self.options.debug_info {
                self.line(source)?
            } else {
                0
            });
        }
        Ok((instructions, lines))
    }

    /// Map call operands without interpreting names or types again.
    fn call_instruction(
        &mut self,
        op: &Op,
        source: SourceSpan,
    ) -> Result<(PexOpcode, Vec<PexValue>, Vec<PexValue>), Diagnostic> {
        let v = |this: &mut Self, value: &Value| this.value(value, source);
        Ok(match op {
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
            },
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
            _ => unreachable!("opcode group selected by instruction dispatch"),
        })
    }

    /// Map array operands without interpreting names or types again.
    fn array_instruction(
        &mut self,
        op: &Op,
        source: SourceSpan,
    ) -> Result<(PexOpcode, Vec<PexValue>, Vec<PexValue>), Diagnostic> {
        let v = |this: &mut Self, value: &Value| this.value(value, source);
        Ok(match op {
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
            _ => unreachable!("opcode group selected by instruction dispatch"),
        })
    }

    fn branch_offset(
        labels: &BTreeMap<u32, usize>,
        current: usize,
        target: u32,
        source: SourceSpan,
    ) -> Result<i32, Diagnostic> {
        let destination = labels
            .get(&target)
            .ok_or_else(|| error(source, "pex.branch", "unknown branch target"))?;
        let delta = i128::try_from(*destination).expect("usize fits in i128")
            - i128::try_from(current).expect("usize fits in i128");
        i32::try_from(delta).map_err(|_range_error| {
            error(
                source,
                "pex.branch",
                "branch offset exceeds 32-bit PEX range",
            )
        })
    }
}

#[cfg(test)]
mod tests;
