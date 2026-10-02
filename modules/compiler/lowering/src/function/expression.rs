//! Expression lowering with source-order captures and conversions.
use super::*;

impl<'a> FunctionLowerer<'a> {
    pub(super) fn expr(&mut self, expr: &ExpressionFact) -> Option<Value> {
        let value = match &expr.kind {
            ExpressionKind::Missing => {
                self.issue(
                    "lowering.invalid-expression",
                    "cannot emit an erroneous expression",
                    expr.span,
                );
                return None;
            }
            ExpressionKind::Literal(text) => match literal(text, &expr.ty) {
                Some(value) => value,
                None => {
                    self.issue(
                        "target.invalid-literal",
                        "literal cannot be represented by target",
                        expr.span,
                    );
                    return None;
                }
            },
            ExpressionKind::Reference(name) => {
                let Some(binding) = &expr.binding else {
                    self.issue(
                        "lowering.unbound-reference",
                        "reference is unbound",
                        expr.span,
                    );
                    return None;
                };
                match &binding.symbol {
                    Symbol::ParentReceiver { .. } => {
                        self.issue(
                            "lowering.parent-value",
                            "Parent is only a function call receiver",
                            expr.span,
                        );
                        return None;
                    }
                    Symbol::Parameter { .. } | Symbol::Local { .. } => {
                        Value::Identifier(self.slot(&binding.symbol, &expr.ty, expr.span)?)
                    }
                    Symbol::Script(_) if name.text.eq_ignore_ascii_case("self") => {
                        Value::Identifier("self".into())
                    }
                    Symbol::Script(_) if name.text.eq_ignore_ascii_case("parent") => {
                        Value::Identifier("self".into())
                    }
                    Symbol::Script(_) => Value::Identifier(name.text.clone()),
                    Symbol::Member { name, .. } | Symbol::StateMember { name, .. } => {
                        match find_member(self.source, &binding.symbol).map(|member| &member.kind) {
                            Some(MemberKind::Property {
                                auto: true,
                                read_only: true,
                            }) if self
                                .source
                                .members
                                .iter()
                                .any(|item| item.symbol == binding.symbol) =>
                            {
                                let member = find_member(self.source, &binding.symbol)?;
                                let Some(value) = member
                                    .initial_literal
                                    .as_deref()
                                    .and_then(|text| literal(text, &member.ty))
                                else {
                                    self.issue(
                                        "target.property-initializer",
                                        "AutoReadOnly requires a representable constant",
                                        member.span,
                                    );
                                    return None;
                                };
                                value
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
                                Value::Identifier(format!("::{name}_var"))
                            }
                            Some(MemberKind::Property { .. }) => {
                                let dest = self.temp(&expr.ty);
                                self.emit(
                                    Op::PropertyGet {
                                        name: name.clone(),
                                        receiver: Value::Identifier("self".into()),
                                        dest: dest.clone(),
                                    },
                                    expr.span,
                                );
                                dest
                            }
                            Some(MemberKind::Function { .. }) => {
                                self.issue(
                                    "lowering.function-value",
                                    "function is not a first-class value",
                                    expr.span,
                                );
                                return None;
                            }
                            _ => Value::Identifier(name.clone()),
                        }
                    }
                    _ => {
                        self.issue(
                            "lowering.unsupported-reference",
                            "reference cannot be represented",
                            expr.span,
                        );
                        return None;
                    }
                }
            }
            ExpressionKind::Parenthesized(inner) => self.expr(inner)?,
            ExpressionKind::Unary { operator, operand } => {
                if operator == "+" {
                    self.decisions.push(Decision {
                        feature: "unary-plus",
                        outcome: Outcome::Lowered { rule: "identity" },
                        source: expr.span,
                    });
                }
                let signed_literal = if operator == "-" && expr.ty == Type::Int {
                    match &operand.kind {
                        ExpressionKind::Literal(text) => {
                            folio_hir::decode_integer_literal(&format!("-{}", text.trim()))
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                if let Some(value) = signed_literal {
                    Value::Int(value)
                } else {
                    let value = self.expr(operand)?;
                    if operator == "+" {
                        value
                    } else {
                        let op = match (operator.as_str(), &expr.ty) {
                            ("-", Type::Int) => UnaryOp::NegInt,
                            ("-", Type::Float) => UnaryOp::NegFloat,
                            ("!", Type::Bool) => UnaryOp::Not,
                            _ => {
                                self.issue(
                                    "target.unary-operator",
                                    format!("unsupported unary operator {operator}"),
                                    expr.span,
                                );
                                return None;
                            }
                        };
                        let dest = self.temp(&expr.ty);
                        self.emit(
                            Op::Unary {
                                operator: op,
                                dest: dest.clone(),
                                value,
                            },
                            expr.span,
                        );
                        dest
                    }
                }
            }
            ExpressionKind::Binary {
                operator,
                left,
                right,
            } if operator == "&&" || operator == "||" => {
                let dest = self.temp(&Type::Bool);
                let left_value = self.expr(left)?;
                self.emit(Op::Assign(dest.clone(), left_value), left.span);
                let end = self.label();
                self.emit(
                    Op::JumpIf {
                        when_true: operator == "||",
                        condition: dest.clone(),
                        target: end,
                    },
                    expr.span,
                );
                let right_value = self.expr(right)?;
                self.emit(Op::Assign(dest.clone(), right_value), right.span);
                self.emit(Op::Label(end), expr.span);
                self.decisions.push(Decision {
                    feature: "short-circuit-boolean",
                    outcome: Outcome::Lowered {
                        rule: "conditional-branch",
                    },
                    source: expr.span,
                });
                dest
            }
            ExpressionKind::Binary {
                operator,
                left,
                right,
            } => {
                let left_value = self.expr(left)?;
                let mut left_value = self.capture(
                    left_value,
                    left.conversion.as_ref().unwrap_or(&left.ty),
                    left.span,
                );
                let right_value = self.expr(right)?;
                // The right operand is consumed immediately; no later expression can rewrite it.
                let mut right_value = right_value;
                let operand_ty = if operator == "+" && expr.ty == Type::String {
                    &Type::String
                } else if matches!(left.ty, Type::Float) || matches!(right.ty, Type::Float) {
                    &Type::Float
                } else {
                    &left.ty
                };
                if operand_ty == &Type::Float && left.ty == Type::Int {
                    let dest = self.temp(&Type::Float);
                    self.emit(Op::Cast(dest.clone(), left_value), left.span);
                    left_value = dest;
                }
                if operand_ty == &Type::Float && right.ty == Type::Int {
                    let dest = self.temp(&Type::Float);
                    self.emit(Op::Cast(dest.clone(), right_value), right.span);
                    right_value = dest;
                }
                let Some(op) = binary_op(operator, operand_ty) else {
                    self.issue(
                        "target.binary-operator",
                        format!("unsupported binary operator {operator}"),
                        expr.span,
                    );
                    return None;
                };
                let dest = self.temp(&expr.ty);
                self.emit(
                    Op::Binary {
                        operator: op,
                        dest: dest.clone(),
                        left: left_value,
                        right: right_value,
                    },
                    expr.span,
                );
                if operator == "!=" {
                    let inverted = self.temp(&Type::Bool);
                    self.emit(
                        Op::Unary {
                            operator: UnaryOp::Not,
                            dest: inverted.clone(),
                            value: dest,
                        },
                        expr.span,
                    );
                    inverted
                } else {
                    dest
                }
            }
            ExpressionKind::Cast { value, target } => {
                let value = self.expr(value)?;
                if type_name(target).is_none() {
                    self.issue(
                        "target.cast-type",
                        "cast target cannot be represented",
                        expr.span,
                    );
                    return None;
                }
                let dest = self.temp(target);
                self.emit(Op::Cast(dest.clone(), value), expr.span);
                dest
            }
            ExpressionKind::Member { owner, name } => {
                if !matches!(owner.ty, Type::Array(_))
                    && let Some(member) = expr
                        .binding
                        .as_ref()
                        .and_then(|binding| find_member(self.source, &binding.symbol))
                {
                    match member.kind {
                        MemberKind::Function { .. } => {
                            self.issue(
                                "lowering.function-value",
                                "function is not a first-class value",
                                expr.span,
                            );
                            return None;
                        }
                        MemberKind::Variable => {
                            let own_receiver = matches!(&owner.kind, ExpressionKind::Reference(reference) if reference.text.eq_ignore_ascii_case("self") || reference.text.eq_ignore_ascii_case("parent"));
                            if own_receiver {
                                return Some(Value::Identifier(name.text.clone()));
                            }
                            self.issue(
                                "target.foreign-variable",
                                "script variable cannot be read through another instance",
                                expr.span,
                            );
                            return None;
                        }
                        _ => {}
                    }
                }
                let receiver = self.expr(owner)?;
                let dest = self.temp(&expr.ty);
                if matches!(owner.ty, Type::Array(_)) && name.text.eq_ignore_ascii_case("length") {
                    self.emit(
                        Op::ArrayLength {
                            dest: dest.clone(),
                            array: receiver,
                        },
                        expr.span,
                    );
                } else {
                    self.emit(
                        Op::PropertyGet {
                            name: name.text.clone(),
                            receiver,
                            dest: dest.clone(),
                        },
                        expr.span,
                    );
                }
                dest
            }
            ExpressionKind::Index { owner, index } => {
                let array = self.expr(owner)?;
                let array = self.capture(array, &owner.ty, owner.span);
                let index = self.expr(index)?;
                let dest = self.temp(&expr.ty);
                self.emit(
                    Op::ArrayGet {
                        dest: dest.clone(),
                        array,
                        index,
                    },
                    expr.span,
                );
                dest
            }
            ExpressionKind::NewArray {
                element_type,
                length,
            } => {
                if type_name(element_type).is_none() {
                    self.issue(
                        "target.array-element",
                        "array element type cannot be represented",
                        expr.span,
                    );
                    return None;
                }
                let Some(Value::Int(size)) = (match &length.kind {
                    ExpressionKind::Literal(text) => literal(text, &Type::Int),
                    _ => None,
                }) else {
                    self.issue(
                        "target.array-size",
                        "Skyrim array length must be an integer literal",
                        length.span,
                    );
                    return None;
                };
                if size < 1 || size as u32 > self.target.max_array_length {
                    self.issue(
                        "target.array-size",
                        format!(
                            "Skyrim array length must be between 1 and {}",
                            self.target.max_array_length
                        ),
                        length.span,
                    );
                    return None;
                }
                let dest = self.temp(&expr.ty);
                self.emit(
                    Op::ArrayCreate {
                        dest: dest.clone(),
                        length: Value::Int(size),
                    },
                    expr.span,
                );
                dest
            }
            ExpressionKind::Call {
                callee,
                arguments,
                argument_ordinals,
                parameter_defaults,
                is_global,
            } => self.call(
                expr,
                callee,
                arguments,
                argument_ordinals,
                parameter_defaults,
                *is_global,
            )?,
        };
        if let Some(target) = &expr.conversion {
            if type_name(target).is_none() {
                self.issue(
                    "target.conversion-type",
                    "conversion target cannot be represented",
                    expr.span,
                );
                return None;
            }
            let dest = self.temp(target);
            self.emit(Op::Cast(dest.clone(), value), expr.span);
            self.decisions.push(Decision {
                feature: "implicit-conversion",
                outcome: Outcome::Lowered {
                    rule: "explicit-cast",
                },
                source: expr.span,
            });
            Some(dest)
        } else {
            Some(value)
        }
    }
}
