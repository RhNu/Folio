//! Call argument evaluation and proven redundant capture removal.
use super::{
    BTreeSet, ExpressionFact, ExpressionKind, FunctionLowerer, Op, Symbol, Type, Value, literal,
    member_name,
};

/// A captured argument operand and the instruction that stored its original value.
struct CallCapture {
    instruction: usize,
    original: Value,
    copied: Value,
    ordinal: Option<usize>,
}

impl FunctionLowerer<'_> {
    /// A copied operand can be read directly when later argument evaluation cannot change it.
    fn call_source_is_stable(&self, capture: &CallCapture) -> bool {
        let Value::Identifier(source) = &capture.original else {
            return true;
        };
        let local = source.eq_ignore_ascii_case("self")
            || self
                .function
                .parameters
                .iter()
                .chain(&self.function.locals)
                .any(|slot| slot.name.eq_ignore_ascii_case(source));
        self.function.instructions[capture.instruction + 1..]
            .iter()
            .all(|instruction| {
                let destination = match &instruction.op {
                    Op::Assign(dest, _)
                    | Op::Cast(dest, _)
                    | Op::Unary { dest, .. }
                    | Op::Binary { dest, .. }
                    | Op::CallMethod { dest, .. }
                    | Op::CallParent { dest, .. }
                    | Op::CallStatic { dest, .. }
                    | Op::PropertyGet { dest, .. }
                    | Op::ArrayCreate { dest, .. }
                    | Op::ArrayLength { dest, .. }
                    | Op::ArrayGet { dest, .. }
                    | Op::ArrayFind { dest, .. } => Some(dest),
                    _ => None,
                };
                let writes_source = destination.is_some_and(|value| {
                    matches!(value, Value::Identifier(name) if name.eq_ignore_ascii_case(source))
                });
                let may_write_field = !local
                    && matches!(
                        instruction.op,
                        Op::CallMethod { .. }
                            | Op::CallParent { .. }
                            | Op::CallStatic { .. }
                            | Op::PropertyGet { .. }
                            | Op::PropertySet { .. }
                    );
                !writes_source && !may_write_field
            })
    }

    /// Evaluate a receiver before arguments and preserve its original storage value.
    fn call_receiver(
        &mut self,
        callee: &ExpressionFact,
        is_global: bool,
        captures: &mut Vec<CallCapture>,
    ) -> Result<Option<Value>, ()> {
        let receiver = if let ExpressionKind::Member { owner, .. } = &callee.kind {
            if is_global
                || owner
                    .binding
                    .as_ref()
                    .is_some_and(|binding| matches!(binding.symbol, Symbol::ParentReceiver { .. }))
            {
                None
            } else {
                let value = self.expr(owner).ok_or(())?;
                if matches!(value, Value::Identifier(_)) {
                    let instruction = self.function.instructions.len();
                    let copied = self.capture(value.clone(), &owner.ty, owner.span);
                    if copied != value {
                        captures.push(CallCapture {
                            instruction,
                            original: value,
                            copied: copied.clone(),
                            ordinal: None,
                        });
                    }
                    Some(copied)
                } else {
                    Some(value)
                }
            }
        } else {
            None
        };
        Ok(receiver)
    }

    /// Remove only captures whose source cannot be changed by later operands.
    fn simplify_call_captures(
        &mut self,
        captures: &[CallCapture],
        ordered: &mut [Option<Value>],
        receiver: &mut Option<Value>,
    ) {
        let mut removable = Vec::new();
        let mut redundant_slots = BTreeSet::new();
        for capture in captures {
            if self.call_source_is_stable(capture) {
                if let Some(ordinal) = capture.ordinal {
                    ordered[ordinal] = Some(capture.original.clone());
                } else {
                    *receiver = Some(capture.original.clone());
                }
                removable.push(capture.instruction);
                if let Value::Identifier(name) = &capture.copied {
                    redundant_slots.insert(name.clone());
                }
            }
        }
        for index in removable.iter().rev() {
            self.function.instructions.remove(*index);
        }
        self.function
            .locals
            .retain(|local| !redundant_slots.contains(&local.name));
        tracing::trace!(
            removed_captures = removable.len(),
            "simplified call operands"
        );
    }

    /// Evaluate arguments in source order, then fill their bound parameter positions.
    fn call_arguments(
        &mut self,
        expr: &ExpressionFact,
        arguments: &[ExpressionFact],
        ordinals: &[usize],
        defaults: &[Option<(Type, String)>],
        captures: &mut Vec<CallCapture>,
    ) -> Option<Vec<Option<Value>>> {
        if arguments.len() != ordinals.len() {
            self.issue(
                "lowering.call-arity",
                "call argument binding is incomplete",
                expr.span,
            );
            return None;
        }
        let mut ordered = vec![None; defaults.len()];
        for (argument, &ordinal) in arguments.iter().zip(ordinals) {
            let value = self.expr(argument)?;
            // Keep an early copy until later argument effects are known.
            let passed = if matches!(value, Value::Identifier(_)) {
                let copied = self.temp(
                    &argument
                        .conversion
                        .clone()
                        .unwrap_or_else(|| argument.ty.clone()),
                );
                let instruction = self.function.instructions.len();
                self.emit(Op::Assign(copied.clone(), value.clone()), argument.span);
                captures.push(CallCapture {
                    instruction,
                    original: value,
                    copied: copied.clone(),
                    ordinal: Some(ordinal),
                });
                copied
            } else {
                value
            };
            if ordinal >= ordered.len() || ordered[ordinal].replace(passed).is_some() {
                self.issue(
                    "lowering.call-binding",
                    "call argument mapping is invalid",
                    argument.span,
                );
                return None;
            }
        }
        for (ordinal, value) in ordered.iter_mut().enumerate() {
            if value.is_none() {
                let Some((type_hint, default)) = defaults[ordinal].as_ref() else {
                    self.issue(
                        "lowering.call-default",
                        "required argument is missing",
                        expr.span,
                    );
                    return None;
                };
                let Some(default) = literal(default, type_hint) else {
                    self.issue(
                        "target.call-default",
                        "default argument literal is not representable",
                        expr.span,
                    );
                    return None;
                };
                *value = Some(default);
            }
        }
        Some(ordered)
    }

    pub(super) fn call(
        &mut self,
        expr: &ExpressionFact,
        callee: &ExpressionFact,
        arguments: &[ExpressionFact],
        ordinals: &[usize],
        defaults: &[Option<(Type, String)>],
        is_global: bool,
    ) -> Option<Value> {
        let Some(binding) = &callee.binding else {
            self.issue("lowering.unbound-call", "call target is unbound", expr.span);
            return None;
        };
        let Some(name) = (match &binding.symbol {
            Symbol::Intrinsic { name } => Some(name.clone()),
            other => member_name(other).map(str::to_owned),
        }) else {
            self.issue(
                "lowering.invalid-call",
                "call target is not callable",
                expr.span,
            );
            return None;
        };
        let mut captures = Vec::new();
        let mut receiver = self.call_receiver(callee, is_global, &mut captures).ok()?;
        let mut ordered =
            self.call_arguments(expr, arguments, ordinals, defaults, &mut captures)?;
        self.simplify_call_captures(&captures, &mut ordered, &mut receiver);
        let args: Vec<Value> = ordered.into_iter().flatten().collect();
        let dest = self.temp(&expr.ty);
        let op = if matches!(&binding.symbol, Symbol::Intrinsic { name } if name.eq_ignore_ascii_case("Find") || name.eq_ignore_ascii_case("RFind"))
        {
            let Some(array) = receiver else {
                self.issue(
                    "lowering.array-call",
                    "array intrinsic requires an array receiver",
                    expr.span,
                );
                return None;
            };
            if args.len() != 2 {
                self.issue(
                    "lowering.array-call",
                    "array search requires value and start index",
                    expr.span,
                );
                return None;
            }
            Op::ArrayFind {
                reverse: name.eq_ignore_ascii_case("RFind"),
                dest: dest.clone(),
                array,
                value: args[0].clone(),
                start: args[1].clone(),
            }
        } else if is_global {
            let script = match &binding.symbol {
                Symbol::Member { script, .. } | Symbol::StateMember { script, .. } => {
                    script.clone()
                },
                _ => return None,
            };
            Op::CallStatic {
                script,
                name,
                dest: dest.clone(),
                args,
            }
        } else if matches!(&callee.kind, ExpressionKind::Member { owner, .. } if owner.binding.as_ref().is_some_and(|binding| matches!(binding.symbol, Symbol::ParentReceiver { .. })))
        {
            Op::CallParent {
                name,
                dest: dest.clone(),
                args,
            }
        } else {
            Op::CallMethod {
                name,
                receiver: receiver.unwrap_or_else(|| Value::Identifier("self".into())),
                dest: dest.clone(),
                args,
            }
        };
        self.emit(op, expr.span);
        Some(dest)
    }
}
