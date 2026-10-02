//! Statement control flow and single-evaluation assignment places.
use super::*;

/// A writable location whose receiver and index have already been evaluated.
enum Place {
    Slot(Value),
    Property { name: String, receiver: Value },
    Array { array: Value, index: Value },
}
impl<'a> FunctionLowerer<'a> {
    pub(super) fn statement(&mut self, statement: &Statement) {
        match statement {
            Statement::Return { span, value } => {
                let value = value
                    .as_ref()
                    .and_then(|value| self.expr(value))
                    .unwrap_or(Value::None);
                self.emit(Op::Return(value), *span);
            }
            Statement::Variable { declaration, value } => {
                if let Some(slot) =
                    self.slot(&declaration.symbol, &declaration.ty, declaration.span)
                    && let Some(value) = value.as_ref().and_then(|value| self.expr(value))
                {
                    self.emit(Op::Assign(Value::Identifier(slot), value), declaration.span);
                }
            }
            Statement::Assignment {
                span,
                target,
                value,
                operator,
            } => self.assignment(*span, target, value, operator),
            Statement::Expression(value) => {
                let _ = self.expr(value);
            }
            Statement::If {
                span,
                condition,
                then_branch,
                else_if,
                else_branch,
            } => {
                let end = self.label();
                let mut branches = Vec::with_capacity(1 + else_if.len());
                branches.push((condition, then_branch));
                branches.extend(else_if.iter().map(|(test, body)| (test, body)));
                for (test, body) in branches {
                    let next = self.label();
                    if let Some(value) = self.expr(test) {
                        self.emit(
                            Op::JumpIf {
                                when_true: false,
                                condition: value,
                                target: next,
                            },
                            test.span,
                        );
                    }
                    for stmt in body {
                        self.statement(stmt);
                    }
                    self.emit(Op::Jump(end), *span);
                    self.emit(Op::Label(next), *span);
                }
                for stmt in else_branch {
                    self.statement(stmt);
                }
                self.emit(Op::Label(end), *span);
            }
            Statement::While {
                span,
                condition,
                body,
            } => {
                let start = self.label();
                let end = self.label();
                self.emit(Op::Label(start), *span);
                if let Some(value) = self.expr(condition) {
                    self.emit(
                        Op::JumpIf {
                            when_true: false,
                            condition: value,
                            target: end,
                        },
                        condition.span,
                    );
                }
                for stmt in body {
                    self.statement(stmt);
                }
                self.emit(Op::Jump(start), *span);
                self.emit(Op::Label(end), *span);
            }
            Statement::Error(span) => self.issue(
                "lowering.invalid-statement",
                "cannot emit an erroneous statement",
                *span,
            ),
        }
    }

    fn assignment(
        &mut self,
        span: SourceSpan,
        target: &ExpressionFact,
        value: &ExpressionFact,
        operator: &str,
    ) {
        // Destination components are evaluated before the right side, once each.
        let place = match &target.kind {
            ExpressionKind::Reference(_) => {
                let Some(binding) = &target.binding else {
                    self.issue(
                        "lowering.unbound-target",
                        "assignment target is unbound",
                        span,
                    );
                    return;
                };
                match &binding.symbol {
                    Symbol::Local { .. } | Symbol::Parameter { .. } => self
                        .slot(&binding.symbol, &target.ty, span)
                        .map(|name| Place::Slot(Value::Identifier(name))),
                    Symbol::Member { name, .. } => {
                        match find_member(self.source, &binding.symbol).map(|member| &member.kind) {
                            Some(MemberKind::Property {
                                read_only: true, ..
                            }) => {
                                self.issue(
                                    "lowering.read-only-property",
                                    "read-only property cannot be assigned",
                                    span,
                                );
                                None
                            }
                            Some(MemberKind::Property {
                                auto: true,
                                read_only: false,
                            }) if self
                                .source
                                .members
                                .iter()
                                .any(|item| item.symbol == binding.symbol) =>
                            {
                                Some(Place::Slot(Value::Identifier(format!("::{name}_var"))))
                            }
                            Some(MemberKind::Property { .. }) => Some(Place::Property {
                                name: name.clone(),
                                receiver: Value::Identifier("self".into()),
                            }),
                            _ => Some(Place::Slot(Value::Identifier(name.clone()))),
                        }
                    }
                    _ => None,
                }
            }
            ExpressionKind::Member { owner, name } => {
                let own_receiver = matches!(&owner.kind, ExpressionKind::Reference(reference) if reference.text.eq_ignore_ascii_case("self") || reference.text.eq_ignore_ascii_case("parent"));
                if let Some(member) = target
                    .binding
                    .as_ref()
                    .and_then(|binding| find_member(self.source, &binding.symbol))
                    && matches!(member.kind, MemberKind::Variable)
                {
                    if !own_receiver {
                        self.issue(
                            "target.foreign-variable",
                            "script variable cannot be assigned through another instance",
                            target.span,
                        );
                        return;
                    }
                    Some(Place::Slot(Value::Identifier(name.text.clone())))
                } else {
                    let value = self.expr(owner);
                    value.map(|value| Place::Property {
                        name: name.text.clone(),
                        receiver: self.capture(value, &owner.ty, owner.span),
                    })
                }
            }
            ExpressionKind::Index { owner, index } => {
                let Some(array) = self.expr(owner) else {
                    return;
                };
                let array = self.capture(array, &owner.ty, owner.span);
                let Some(index_value) = self.expr(index) else {
                    return;
                };
                let index_value = self.capture(index_value, &Type::Int, index.span);
                Some(Place::Array {
                    array,
                    index: index_value,
                })
            }
            _ => None,
        };
        let Some(place) = place else {
            self.issue(
                "lowering.invalid-target",
                "assignment target is not representable",
                span,
            );
            return;
        };
        let prior = if operator != "=" {
            let read = self.read_place(&place, &target.ty, span);
            read.map(|read| self.capture(read, &target.ty, span))
        } else {
            None
        };
        let Some(mut result) = self.expr(value) else {
            return;
        };
        if let Some(prior) = prior {
            let operator = operator.trim_end_matches('=');
            if target.ty == Type::Float && value.ty == Type::Int {
                let converted = self.temp(&Type::Float);
                self.emit(Op::Cast(converted.clone(), result), value.span);
                result = converted;
            }
            let dest = self.temp(&target.ty);
            if let Some(binary) = binary_op(operator, &target.ty) {
                self.emit(
                    Op::Binary {
                        operator: binary,
                        dest: dest.clone(),
                        left: prior,
                        right: result,
                    },
                    span,
                );
                result = dest;
                self.decisions.push(Decision {
                    feature: "compound-assignment",
                    outcome: Outcome::Lowered {
                        rule: "read-compute-write",
                    },
                    source: span,
                });
            } else {
                self.issue(
                    "target.compound-operator",
                    format!("unsupported compound operator {operator}"),
                    span,
                );
                return;
            }
        }
        match place {
            Place::Slot(dest) => self.emit(Op::Assign(dest, result), span),
            Place::Property { name, receiver } => self.emit(
                Op::PropertySet {
                    name,
                    receiver,
                    value: result,
                },
                span,
            ),
            Place::Array { array, index } => self.emit(
                Op::ArraySet {
                    array,
                    index,
                    value: result,
                },
                span,
            ),
        }
    }

    fn read_place(&mut self, place: &Place, ty: &Type, span: SourceSpan) -> Option<Value> {
        match place {
            Place::Slot(value) => Some(value.clone()),
            Place::Property { name, receiver } => {
                let dest = self.temp(ty);
                self.emit(
                    Op::PropertyGet {
                        name: name.clone(),
                        receiver: receiver.clone(),
                        dest: dest.clone(),
                    },
                    span,
                );
                Some(dest)
            }
            Place::Array { array, index } => {
                let dest = self.temp(ty);
                self.emit(
                    Op::ArrayGet {
                        dest: dest.clone(),
                        array: array.clone(),
                        index: index.clone(),
                    },
                    span,
                );
                Some(dest)
            }
        }
    }
}
