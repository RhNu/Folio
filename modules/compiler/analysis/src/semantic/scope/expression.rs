//! Expression facts, type checks, and implicit conversions.
use super::*;

impl<'a> Scope<'a> {
    pub(super) fn expr(&mut self, node: &SyntaxNode) -> ExpressionFact {
        self.expr_with_access(node, true)
    }

    /// Assignment destinations may omit Get; their receiver/index expressions still read values.
    pub(super) fn expr_with_access(&mut self, node: &SyntaxNode, read: bool) -> ExpressionFact {
        let location = span(self.file, node);
        let (ty, binding, kind) = match node.kind() {
            SyntaxKind::LiteralExpr => {
                let literal = node.text().to_string();
                let ty = match literal.to_ascii_lowercase().as_str() {
                    "true" | "false" => Type::Bool,
                    "none" => Type::None,
                    _ if literal.starts_with('"') => Type::String,
                    _ if literal.contains('.') => Type::Float,
                    _ => Type::Int,
                };
                (ty, None, ExpressionKind::Literal(literal))
            }
            SyntaxKind::NameExpr => {
                let Some(token) = node
                    .children_with_tokens()
                    .find_map(|item| item.into_token())
                else {
                    return self.error_expr(location);
                };
                let name = NameRef {
                    text: token.text().to_string(),
                    span: token_span(self.file, &token),
                };
                if name.text.eq_ignore_ascii_case("parent") {
                    let legal = node
                        .parent()
                        .filter(|parent| parent.kind() == SyntaxKind::MemberExpr)
                        .and_then(|member| {
                            member
                                .parent()
                                .filter(|parent| parent.kind() == SyntaxKind::CallExpr)
                                .map(|call| (member, call))
                        })
                        .is_some_and(|(member, call)| {
                            direct_expression(&call).as_ref() == Some(&member)
                        });
                    if !legal || self.script.parent.is_none() || self.member.global {
                        self.issue(
                            "semantic.parent-context",
                            "Parent requires an inherited instance function call",
                            location,
                        );
                        return self.error_expr(location);
                    }
                }
                let (ty, binding) = self.resolve_name(&name);
                (ty, binding, ExpressionKind::Reference(name))
            }
            SyntaxKind::ParenExpr => {
                let Some(child) = direct_expression(node) else {
                    return self.error_expr(location);
                };
                let inner = self.expr(&child);
                (
                    inner.ty.clone(),
                    inner.binding.clone(),
                    ExpressionKind::Parenthesized(Box::new(inner)),
                )
            }
            SyntaxKind::UnaryExpr => {
                let operator = node
                    .children_with_tokens()
                    .find_map(|item| item.into_token())
                    .map(|token| token.text().to_string())
                    .unwrap_or_default();
                let Some(child) = direct_expression(node) else {
                    return self.error_expr(location);
                };
                let mut operand = self.expr(&child);
                let ty = match operator.as_str() {
                    "!" if operand.ty != Type::Error
                        && implicitly_convertible(self.world, &operand.ty, &Type::Bool) =>
                    {
                        if operand.ty != Type::Bool && operand.ty != Type::Error {
                            operand.conversion = Some(Type::Bool);
                        }
                        Type::Bool
                    }
                    "+" | "-" if matches!(operand.ty, Type::Int | Type::Float) => {
                        operand.ty.clone()
                    }
                    _ if operand.ty == Type::Error => Type::Error,
                    _ => {
                        self.issue(
                            "semantic.operator-type",
                            format!("invalid operand for {operator}"),
                            location,
                        );
                        Type::Error
                    }
                };
                (
                    ty,
                    None,
                    ExpressionKind::Unary {
                        operator,
                        operand: Box::new(operand),
                    },
                )
            }
            SyntaxKind::BinaryExpr => {
                let mut children = direct_expressions(node);
                let Some(left) = children.next() else {
                    return self.error_expr(location);
                };
                let mut left = self.expr(&left);
                let operator = node
                    .children_with_tokens()
                    .filter_map(|item| item.into_token())
                    .find(|token| {
                        !matches!(token.kind(), SyntaxKind::Whitespace | SyntaxKind::Comment)
                    })
                    .map(|token| token.text().to_string())
                    .unwrap_or_default();
                if operator.eq_ignore_ascii_case("as") {
                    let Some(right) = node
                        .children()
                        .find(|child| child.kind() == SyntaxKind::TypeRef)
                    else {
                        return self.error_expr(location);
                    };
                    let target = Type::from_spelling(&type_text(&right));
                    if !known_type(self.world, &target) {
                        self.issue(
                            "semantic.unknown-type",
                            format!("unknown cast type {target:?}"),
                            span(self.file, &right),
                        );
                    }
                    let allowed = castable(self.world, &left.ty, &target);
                    if !allowed {
                        self.issue(
                            "semantic.invalid-cast",
                            format!("cannot cast {:?} to {target:?}", left.ty),
                            location,
                        );
                    }
                    (
                        if allowed { target.clone() } else { Type::Error },
                        None,
                        ExpressionKind::Cast {
                            value: Box::new(left),
                            target,
                        },
                    )
                } else {
                    let Some(right) = children.next() else {
                        return self.error_expr(location);
                    };
                    let mut right = self.expr(&right);
                    if matches!(operator.as_str(), "&&" | "||") {
                        for operand in [&mut left, &mut right] {
                            if implicitly_convertible(self.world, &operand.ty, &Type::Bool)
                                && operand.ty != Type::Bool
                                && operand.ty != Type::Error
                            {
                                operand.conversion = Some(Type::Bool);
                            }
                        }
                    }
                    let left_type = left.conversion.as_ref().unwrap_or(&left.ty);
                    let right_type = right.conversion.as_ref().unwrap_or(&right.ty);
                    let ty = self.binary_type(&operator, left_type, right_type, location);
                    if operator == "+" && ty == Type::String {
                        for operand in [&mut left, &mut right] {
                            if matches!(operand.ty, Type::Int | Type::Float) {
                                operand.conversion = Some(Type::String);
                            }
                        }
                    }
                    (
                        ty,
                        None,
                        ExpressionKind::Binary {
                            operator,
                            left: Box::new(left),
                            right: Box::new(right),
                        },
                    )
                }
            }
            SyntaxKind::MemberExpr => {
                let Some(owner_node) = direct_expression(node) else {
                    return self.error_expr(location);
                };
                let owner = self.expr(&owner_node);
                let Some(token) = node
                    .children_with_tokens()
                    .filter_map(|item| item.into_token())
                    .find(|token| token.kind() == SyntaxKind::Ident)
                else {
                    return self.error_expr(location);
                };
                let name = NameRef {
                    text: token.text().to_string(),
                    span: token_span(self.file, &token),
                };
                let (ty, binding) = self.resolve_member(&owner, &name);
                (
                    ty,
                    binding,
                    ExpressionKind::Member {
                        owner: Box::new(owner),
                        name,
                    },
                )
            }
            SyntaxKind::IndexExpr => {
                let mut children = direct_expressions(node);
                let Some(owner_node) = children.next() else {
                    return self.error_expr(location);
                };
                let Some(index_node) = children.next() else {
                    return self.error_expr(location);
                };
                let owner = self.expr(&owner_node);
                let mut index = self.expr(&index_node);
                self.expect(&mut index, &Type::Int, "semantic.index-type");
                let ty = match &owner.ty {
                    Type::Array(element) => *element.clone(),
                    Type::Error => Type::Error,
                    _ => {
                        self.issue(
                            "semantic.not-array",
                            "index target is not an array",
                            location,
                        );
                        Type::Error
                    }
                };
                (
                    ty,
                    None,
                    ExpressionKind::Index {
                        owner: Box::new(owner),
                        index: Box::new(index),
                    },
                )
            }
            SyntaxKind::NewArrayExpr => {
                let element_type = node
                    .children_with_tokens()
                    .filter_map(|item| item.into_token())
                    .filter(|token| token.kind() == SyntaxKind::Ident)
                    .nth(1)
                    .map(|token| Type::from_spelling(token.text()))
                    .unwrap_or(Type::Error);
                let Some(length_node) = direct_expression(node) else {
                    return self.error_expr(location);
                };
                let mut length = self.expr(&length_node);
                self.expect(&mut length, &Type::Int, "semantic.array-size-type");
                if !known_type(self.world, &element_type) {
                    self.issue(
                        "semantic.unknown-type",
                        format!("unknown type {element_type:?}"),
                        location,
                    );
                }
                (
                    Type::Array(Box::new(element_type.clone())),
                    None,
                    ExpressionKind::NewArray {
                        element_type,
                        length: Box::new(length),
                    },
                )
            }
            SyntaxKind::CallExpr => {
                let mut children = direct_expressions(node);
                let Some(callee_node) = children.next() else {
                    return self.error_expr(location);
                };
                let callee = self.expr(&callee_node);
                let mut names = Vec::new();
                let mut arguments = Vec::new();
                for child in node
                    .children()
                    .filter(|child| {
                        is_expression(child.kind()) || child.kind() == SyntaxKind::NamedArgument
                    })
                    .skip(1)
                {
                    if child.kind() == SyntaxKind::NamedArgument {
                        let mut parts = child.children().filter(|part| is_expression(part.kind()));
                        let name = parts.next().map(|part| part.text().to_string());
                        let value = parts
                            .next()
                            .map(|part| self.expr(&part))
                            .unwrap_or_else(|| self.error_expr(span(self.file, &child)));
                        names.push(name);
                        arguments.push(value);
                    } else {
                        names.push(None);
                        arguments.push(self.expr(&child));
                    }
                }
                let CheckedCall {
                    result: ty,
                    target,
                    ordinals: bound,
                    defaults,
                } = self.check_call(&callee, &mut arguments, &names, location);
                let is_global = target
                    .as_ref()
                    .and_then(|symbol| match symbol {
                        Symbol::Member { script, name } => {
                            lookup_callable_member(self.world, script, name)
                                .map(|(_, member)| member)
                        }
                        Symbol::StateMember {
                            script,
                            state,
                            name,
                        } => lookup_state_member(self.world, script, state, name)
                            .map(|(_, member)| member),
                        _ => None,
                    })
                    .is_some_and(|member| member.global);
                self.result.script.calls.push(CallFact {
                    span: location,
                    target,
                    arguments: bound.clone(),
                    result: ty.clone(),
                });
                (
                    ty,
                    callee.binding.clone(),
                    ExpressionKind::Call {
                        callee: Box::new(callee),
                        arguments,
                        argument_ordinals: bound,
                        parameter_defaults: defaults,
                        is_global,
                    },
                )
            }
            _ => return self.error_expr(location),
        };
        let fact = ExpressionFact {
            span: location,
            ty,
            binding,
            conversion: None,
            kind,
        };
        if read
            && matches!(
                fact.kind,
                ExpressionKind::Reference(_) | ExpressionKind::Member { .. }
            )
        {
            self.check_readable(&fact);
        }
        self.result.script.expressions.push(fact.clone());
        fact
    }

    pub(super) fn check_readable(&mut self, fact: &ExpressionFact) {
        let member = fact
            .binding
            .as_ref()
            .and_then(|binding| match &binding.symbol {
                Symbol::Member { script, name } => {
                    lookup_member(self.world, script, name).map(|(_, member)| member)
                }
                _ => None,
            });
        if member.is_some_and(|member| member.kind == MemberKind::Property && !member.readable) {
            self.issue(
                "semantic.write-only-property",
                "property has no readable accessor",
                fact.span,
            );
        }
    }

    fn error_expr(&mut self, span: SourceSpan) -> ExpressionFact {
        let fact = ExpressionFact {
            span,
            ty: Type::Error,
            binding: None,
            conversion: None,
            kind: ExpressionKind::Missing,
        };
        self.result.script.expressions.push(fact.clone());
        fact
    }

    pub(super) fn binary_type(
        &mut self,
        operator: &str,
        left: &Type,
        right: &Type,
        location: SourceSpan,
    ) -> Type {
        if matches!(left, Type::Error) || matches!(right, Type::Error) {
            return Type::Error;
        }
        let numeric =
            matches!(left, Type::Int | Type::Float) && matches!(right, Type::Int | Type::Float);
        match operator {
            "+" if (left == &Type::String
                && matches!(right, Type::String | Type::Int | Type::Float))
                || (right == &Type::String && matches!(left, Type::Int | Type::Float)) =>
            {
                Type::String
            }
            "+" | "-" | "*" | "/" | "%" if numeric => {
                if left == &Type::Float || right == &Type::Float {
                    Type::Float
                } else {
                    Type::Int
                }
            }
            "<" | ">" | "<=" | ">=" if numeric => Type::Bool,
            "==" | "!="
                if assignable(self.world, left, right) || assignable(self.world, right, left) =>
            {
                Type::Bool
            }
            "&&" | "||" if left == &Type::Bool && right == &Type::Bool => Type::Bool,
            _ => {
                self.issue(
                    "semantic.operator-type",
                    format!("invalid operands for {operator}"),
                    location,
                );
                Type::Error
            }
        }
    }
}
