//! Local binding, statement checking, and expression type analysis.
use super::*;

impl<'a> Scope<'a> {
    fn issue(&mut self, code: &str, message: impl Into<String>, at: SourceSpan) {
        self.result.diagnostics.push(diagnostic(code, message, at));
    }

    pub(super) fn collect_parameters(&mut self, node: &SyntaxNode) {
        let Some(list) = node
            .children()
            .find(|child| child.kind() == SyntaxKind::ParameterList)
        else {
            return;
        };
        for parameter in list
            .children()
            .filter(|child| child.kind() == SyntaxKind::Parameter)
        {
            let Some(token) = parameter
                .children_with_tokens()
                .filter_map(|item| item.into_token())
                .find(|token| token.kind() == SyntaxKind::Ident)
            else {
                continue;
            };
            let name = token.text().to_string();
            let ty = parameter
                .children()
                .find(|child| child.kind() == SyntaxKind::TypeRef)
                .map(|node| Type::from_spelling(&type_text(&node)))
                .unwrap_or(Type::Error);
            let definition = token_span(self.file, &token);
            let symbol = Symbol::Parameter {
                owner: Box::new(self.callable.clone()),
                name: name.clone(),
            };
            self.parameters.push(ParameterFact {
                name: name.clone(),
                ty: ty.clone(),
                default_literal: self
                    .member
                    .parameters
                    .iter()
                    .find(|(parameter, _, _)| parameter.eq_ignore_ascii_case(&name))
                    .and_then(|(_, _, default)| default.literal().map(str::to_owned)),
                span: definition,
            });
            if self
                .locals
                .insert(
                    key(&name),
                    Local {
                        ty: ty.clone(),
                        symbol: symbol.clone(),
                        definition,
                    },
                )
                .is_some()
            {
                self.issue(
                    "semantic.duplicate-parameter",
                    format!("duplicate parameter {name}"),
                    definition,
                );
            }
            self.result.script.declarations.push(DeclarationFact {
                symbol,
                ty,
                span: definition,
            });
        }
    }

    pub(super) fn block(&mut self, block: &SyntaxNode) -> Vec<Statement> {
        let mut statements = Vec::new();
        for node in block
            .children()
            .filter(|node| node.kind() == SyntaxKind::Statement)
        {
            if self.interrupted || (self.cancelled)() {
                self.interrupted = true;
                break;
            }
            if let Some(statement) = self.statement(&node) {
                statements.push(statement);
            }
        }
        statements
    }

    fn statement(&mut self, wrapper: &SyntaxNode) -> Option<Statement> {
        let node = wrapper.children().next()?;
        match node.kind() {
            SyntaxKind::ReturnStmt => {
                let mut value = direct_expression(&node).map(|expr| self.expr(&expr));
                if let Some(value) = &mut value {
                    self.expect(value, &self.member.ty, "semantic.return-type");
                } else if self.member.ty != Type::Void {
                    self.issue(
                        "semantic.missing-return-value",
                        "return value required",
                        span(self.file, &node),
                    );
                }
                Some(Statement::Return {
                    span: span(self.file, &node),
                    value,
                })
            }
            SyntaxKind::VariableDecl => self.local_variable(&node),
            SyntaxKind::AssignmentStmt => {
                let expressions = direct_expressions(&node).collect::<Vec<_>>();
                if expressions.len() < 2 {
                    return Some(Statement::Error(span(self.file, &node)));
                }
                let target = self.expr(&expressions[0]);
                let mut value = self.expr(&expressions[1]);
                let compound = node
                    .children_with_tokens()
                    .filter_map(|item| item.into_token())
                    .find(|token| {
                        matches!(
                            token.kind(),
                            SyntaxKind::PlusEq
                                | SyntaxKind::MinusEq
                                | SyntaxKind::StarEq
                                | SyntaxKind::SlashEq
                                | SyntaxKind::PercentEq
                        )
                    });
                if !matches!(
                    target.kind,
                    ExpressionKind::Reference(_)
                        | ExpressionKind::Member { .. }
                        | ExpressionKind::Index { .. }
                ) {
                    self.issue(
                        "semantic.not-assignable",
                        "assignment target is not writable",
                        target.span,
                    );
                }
                if let Some(binding) = &target.binding {
                    let member = match &binding.symbol {
                        Symbol::Member { script, name } => {
                            lookup_member(self.world, script, name).map(|(_, member)| member)
                        }
                        Symbol::StateMember {
                            script,
                            state,
                            name,
                        } => lookup_state_member(self.world, script, state, name)
                            .map(|(_, member)| member),
                        _ => None,
                    };
                    if member.is_some_and(|member| {
                        member.kind == MemberKind::Property && !member.writable
                    }) {
                        self.issue(
                            "semantic.read-only-property",
                            "property has no writable accessor",
                            target.span,
                        );
                    }
                }
                if let Some(operator) = &compound {
                    let operator = operator.text().trim_end_matches('=');
                    let result_type =
                        self.binary_type(operator, &target.ty, &value.ty, span(self.file, &node));
                    if operator == "+"
                        && result_type == Type::String
                        && matches!(value.ty, Type::Int | Type::Float)
                    {
                        value.conversion = Some(Type::String);
                    }
                    if !assignable(self.world, &result_type, &target.ty) {
                        self.issue(
                            "semantic.assignment-type",
                            format!(
                                "compound result {result_type:?} cannot be assigned to {:?}",
                                target.ty
                            ),
                            span(self.file, &node),
                        );
                    }
                } else {
                    self.expect(&mut value, &target.ty, "semantic.assignment-type");
                }
                Some(Statement::Assignment {
                    span: span(self.file, &node),
                    target: Box::new(target),
                    value: Box::new(value),
                    operator: compound
                        .map_or_else(|| "=".to_owned(), |token| token.text().to_owned()),
                })
            }
            SyntaxKind::IfStmt => {
                let mut condition = direct_expression(&node).map(|expr| self.expr(&expr))?;
                self.expect(&mut condition, &Type::Bool, "semantic.condition-type");
                let then_branch = node
                    .children()
                    .find(|child| child.kind() == SyntaxKind::Block)
                    .map(|block| self.block(&block))
                    .unwrap_or_default();
                let mut else_if = Vec::new();
                let mut else_branch = Vec::new();
                for clause in node.children().filter(|child| {
                    matches!(
                        child.kind(),
                        SyntaxKind::ElseIfClause | SyntaxKind::ElseClause
                    )
                }) {
                    if clause.kind() == SyntaxKind::ElseIfClause
                        && let Some(expr) = direct_expression(&clause)
                    {
                        let mut test = self.expr(&expr);
                        self.expect(&mut test, &Type::Bool, "semantic.condition-type");
                        let branch = clause
                            .children()
                            .find(|child| child.kind() == SyntaxKind::Block)
                            .map(|block| self.block(&block))
                            .unwrap_or_default();
                        else_if.push((test, branch));
                        continue;
                    }
                    if let Some(block) = clause
                        .children()
                        .find(|child| child.kind() == SyntaxKind::Block)
                    {
                        else_branch.extend(self.block(&block));
                    }
                }
                Some(Statement::If {
                    span: span(self.file, &node),
                    condition,
                    then_branch,
                    else_if,
                    else_branch,
                })
            }
            SyntaxKind::WhileStmt => {
                let mut condition = direct_expression(&node).map(|expr| self.expr(&expr))?;
                self.expect(&mut condition, &Type::Bool, "semantic.condition-type");
                let body = node
                    .children()
                    .find(|child| child.kind() == SyntaxKind::Block)
                    .map(|block| self.block(&block))
                    .unwrap_or_default();
                Some(Statement::While {
                    span: span(self.file, &node),
                    condition,
                    body,
                })
            }
            kind if is_expression(kind) => Some(Statement::Expression(self.expr(&node))),
            _ => Some(Statement::Error(span(self.file, &node))),
        }
    }

    fn local_variable(&mut self, node: &SyntaxNode) -> Option<Statement> {
        let ty = node
            .children()
            .find(|child| child.kind() == SyntaxKind::TypeRef)
            .map(|node| Type::from_spelling(&type_text(&node)))
            .unwrap_or(Type::Error);
        let token = node
            .children_with_tokens()
            .filter_map(|item| item.into_token())
            .find(|token| token.kind() == SyntaxKind::Ident)?;
        let name = token.text().to_string();
        let definition = token_span(self.file, &token);
        if !known_type(self.world, &ty) {
            self.issue(
                "semantic.unknown-type",
                format!("unknown type {ty:?}"),
                definition,
            );
        }
        let symbol = Symbol::Local {
            owner: Box::new(self.callable.clone()),
            name: name.clone(),
        };
        let declaration = DeclarationFact {
            symbol: symbol.clone(),
            ty: ty.clone(),
            span: definition,
        };
        if self
            .locals
            .insert(
                key(&name),
                Local {
                    ty: ty.clone(),
                    symbol,
                    definition,
                },
            )
            .is_some()
        {
            self.issue(
                "semantic.duplicate-local",
                format!("duplicate local {name}"),
                definition,
            );
        }
        self.result.script.declarations.push(declaration.clone());
        let mut value = direct_expression(node).map(|expr| self.expr(&expr));
        if let Some(value) = &mut value {
            self.expect(value, &ty, "semantic.initializer-type");
        }
        Some(Statement::Variable { declaration, value })
    }

    fn expect(&mut self, value: &mut ExpressionFact, expected: &Type, code: &str) {
        if !implicitly_convertible(self.world, &value.ty, expected) {
            self.issue(
                code,
                format!("expected {expected:?}, found {:?}", value.ty),
                value.span,
            );
        } else if value.ty != *expected && value.ty != Type::Error {
            value.conversion = Some(expected.clone());
        }
    }

    fn expr(&mut self, node: &SyntaxNode) -> ExpressionFact {
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
                let Some(right) = children.next() else {
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
                    let target = Type::from_spelling(&right.text().to_string());
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
        self.result.script.expressions.push(fact.clone());
        fact
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

    fn resolve_name(&mut self, name: &NameRef) -> (Type, Option<Binding>) {
        if let Some(local) = self.locals.get(&key(&name.text)) {
            return (
                local.ty.clone(),
                Some(Binding {
                    name: name.clone(),
                    symbol: local.symbol.clone(),
                    definition: Some(local.definition),
                }),
            );
        }
        if name.text.eq_ignore_ascii_case("self") {
            return (
                Type::Script(self.script.name.clone()),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Script(self.script.name.clone()),
                    definition: self.script.definition,
                }),
            );
        }
        if name.text.eq_ignore_ascii_case("parent")
            && let Some(parent) = &self.script.parent
        {
            return (
                Type::Script(parent.clone()),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Script(parent.clone()),
                    definition: self
                        .world
                        .scripts
                        .get(&key(parent))
                        .and_then(|script| script.definition),
                }),
            );
        }
        // These methods are emitted by the compiler and have fixed signatures;
        // External declarations cannot redefine their meaning.
        if let Some(ty) = state_runtime_intrinsic_type(&name.text) {
            if self.member.global {
                self.issue(
                    "semantic.instance-member",
                    "state runtime methods require an instance",
                    name.span,
                );
                return (Type::Error, None);
            }
            return (
                ty,
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Intrinsic {
                        name: name.text.clone(),
                    },
                    definition: None,
                }),
            );
        }
        if let Some(state) = &self.state
            && let Some((owner, member)) =
                lookup_state_member(self.world, &self.script.name, state, &name.text)
        {
            return (
                member.ty.clone(),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::StateMember {
                        script: owner.name.clone(),
                        state: state.clone(),
                        name: member.name.clone(),
                    },
                    definition: member.definition,
                }),
            );
        }
        if let Some((owner, member)) = lookup_member(self.world, &self.script.name, &name.text) {
            return (
                member.ty.clone(),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Member {
                        script: owner.name.clone(),
                        name: member.name.clone(),
                    },
                    definition: member.definition,
                }),
            );
        }
        if let Some(script) = self.world.scripts.get(&key(&name.text)) {
            return (
                Type::Script(script.name.clone()),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Script(script.name.clone()),
                    definition: script.definition,
                }),
            );
        }
        let mut imported = self
            .imports
            .iter()
            .filter_map(|script| lookup_member(self.world, script, &name.text))
            .filter(|(_, member)| member.global)
            .collect::<Vec<_>>();
        if imported.len() == 1 {
            let (owner, member) = imported.pop().unwrap();
            return (
                member.ty.clone(),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Member {
                        script: owner.name.clone(),
                        name: member.name.clone(),
                    },
                    definition: member.definition,
                }),
            );
        }
        if imported.len() > 1 {
            self.issue(
                "semantic.ambiguous-import",
                format!("ambiguous imported member {}", name.text),
                name.span,
            );
        } else {
            self.issue(
                "semantic.unknown-name",
                format!("unknown name {}", name.text),
                name.span,
            );
        }
        (Type::Error, None)
    }

    fn resolve_member(
        &mut self,
        owner: &ExpressionFact,
        name: &NameRef,
    ) -> (Type, Option<Binding>) {
        if let Type::Array(_) = &owner.ty
            && name.text.eq_ignore_ascii_case("length")
        {
            return (Type::Int, None);
        }
        if let Type::Array(_) = &owner.ty
            && (name.text.eq_ignore_ascii_case("find") || name.text.eq_ignore_ascii_case("rfind"))
        {
            return (
                Type::Int,
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Intrinsic {
                        name: name.text.clone(),
                    },
                    definition: None,
                }),
            );
        }
        let Type::Script(script_name) = &owner.ty else {
            if owner.ty != Type::Error {
                self.issue(
                    "semantic.no-member",
                    format!("type {:?} has no member {}", owner.ty, name.text),
                    name.span,
                );
            }
            return (Type::Error, None);
        };
        if let Some(ty) = state_runtime_intrinsic_type(&name.text) {
            let static_script = matches!(
                owner.binding.as_ref().map(|binding| &binding.symbol),
                Some(Symbol::Script(_))
            ) && !matches!(&owner.kind, ExpressionKind::Reference(reference) if reference.text.eq_ignore_ascii_case("self") || reference.text.eq_ignore_ascii_case("parent"));
            if static_script {
                self.issue(
                    "semantic.instance-member",
                    "state runtime methods require an instance",
                    name.span,
                );
                return (Type::Error, None);
            }
            return (
                ty,
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Intrinsic {
                        name: name.text.clone(),
                    },
                    definition: None,
                }),
            );
        }
        let Some((script, member)) = lookup_member(self.world, script_name, &name.text) else {
            if self.world.scripts.contains_key(&key(script_name)) {
                self.issue(
                    "semantic.unknown-member",
                    format!("unknown member {}", name.text),
                    name.span,
                );
            }
            return (Type::Error, None);
        };
        let static_script = matches!(
            owner.binding.as_ref().map(|binding| &binding.symbol),
            Some(Symbol::Script(_))
        ) && !matches!(&owner.kind, ExpressionKind::Reference(reference) if reference.text.eq_ignore_ascii_case("self") || reference.text.eq_ignore_ascii_case("parent"));
        if static_script
            && !member.global
            && matches!(
                member.kind,
                MemberKind::Function | MemberKind::UnknownCallable
            )
        {
            self.issue(
                "semantic.instance-member",
                format!("{} requires an instance", name.text),
                name.span,
            );
        }
        if !static_script
            && member.global
            && matches!(
                member.kind,
                MemberKind::Function | MemberKind::UnknownCallable
            )
        {
            self.issue(
                "semantic.global-member",
                "global function requires a script qualifier",
                name.span,
            );
            return (Type::Error, None);
        }
        (
            member.ty.clone(),
            Some(Binding {
                name: name.clone(),
                symbol: Symbol::Member {
                    script: script.name.clone(),
                    name: member.name.clone(),
                },
                definition: member.definition,
            }),
        )
    }

    fn check_call(
        &mut self,
        callee: &ExpressionFact,
        args: &mut [ExpressionFact],
        names: &[Option<String>],
        location: SourceSpan,
    ) -> CheckedCall {
        let Some(binding) = &callee.binding else {
            return CheckedCall::error();
        };
        let member = match &binding.symbol {
            Symbol::Member { script, name } => {
                lookup_callable_member(self.world, script, name).map(|(_, member)| member)
            }
            Symbol::StateMember {
                script,
                state,
                name,
            } => lookup_state_member(self.world, script, state, name).map(|(_, member)| member),
            Symbol::Intrinsic { name } => {
                let (result, params): (Type, Vec<(String, Type, ParameterDefault)>) = if name
                    .eq_ignore_ascii_case("GetState")
                {
                    (Type::String, Vec::new())
                } else if name.eq_ignore_ascii_case("GotoState") {
                    (
                        Type::Void,
                        vec![("newState".into(), Type::String, ParameterDefault::Required)],
                    )
                } else if name.eq_ignore_ascii_case("Find") || name.eq_ignore_ascii_case("RFind") {
                    let element = match &callee.kind {
                        ExpressionKind::Member { owner, .. } => match &owner.ty {
                            Type::Array(element) => *element.clone(),
                            _ => Type::Error,
                        },
                        _ => Type::Error,
                    };
                    (
                        Type::Int,
                        vec![
                            ("value".into(), element, ParameterDefault::Required),
                            (
                                "startIndex".into(),
                                Type::Int,
                                ParameterDefault::Literal(
                                    if name.eq_ignore_ascii_case("Find") {
                                        "0"
                                    } else {
                                        "-1"
                                    }
                                    .into(),
                                ),
                            ),
                        ],
                    )
                } else {
                    (Type::Error, Vec::new())
                };
                let intrinsic = MemberInfo {
                    name: name.clone(),
                    ty: result,
                    kind: MemberKind::Function,
                    parameters: params,
                    global: false,
                    auto: false,
                    read_only: false,
                    writable: true,
                    definition: None,
                };
                return self.check_intrinsic_call(
                    &intrinsic,
                    args,
                    names,
                    location,
                    binding.symbol.clone(),
                );
            }
            _ => None,
        };
        let Some(member) = member else {
            self.issue(
                "semantic.not-callable",
                "expression is not callable",
                location,
            );
            return CheckedCall::error();
        };
        if !matches!(
            member.kind,
            MemberKind::Function | MemberKind::Event | MemberKind::UnknownCallable
        ) {
            self.issue("semantic.not-callable", "member is not callable", location);
            return CheckedCall::error();
        }
        let mut bound = Vec::new();
        let mut used = BTreeSet::new();
        let mut saw_named = false;
        for (index, (arg, name)) in args.iter_mut().zip(names).enumerate() {
            let ordinal = if let Some(name) = name {
                saw_named = true;
                member
                    .parameters
                    .iter()
                    .position(|(parameter, _, _)| parameter.eq_ignore_ascii_case(name))
            } else {
                if saw_named {
                    self.issue(
                        "semantic.positional-after-named",
                        "positional argument follows named argument",
                        arg.span,
                    );
                }
                Some(index)
            };
            let Some(ordinal) = ordinal.filter(|ordinal| *ordinal < member.parameters.len()) else {
                self.issue(
                    "semantic.unknown-argument",
                    format!(
                        "unknown or extra argument {}",
                        name.as_deref().unwrap_or("")
                    ),
                    arg.span,
                );
                continue;
            };
            if !used.insert(ordinal) {
                self.issue(
                    "semantic.duplicate-argument",
                    "parameter supplied more than once",
                    arg.span,
                );
                continue;
            }
            let expected = &member.parameters[ordinal].1;
            self.expect(arg, expected, "semantic.argument-type");
            bound.push(ordinal);
        }
        let defaults = self.call_defaults(&member.parameters, &used, location);
        CheckedCall {
            result: member.ty.clone(),
            target: Some(binding.symbol.clone()),
            ordinals: bound,
            defaults,
        }
    }

    fn check_intrinsic_call(
        &mut self,
        member: &MemberInfo,
        args: &mut [ExpressionFact],
        names: &[Option<String>],
        location: SourceSpan,
        symbol: Symbol,
    ) -> CheckedCall {
        let mut bound = Vec::new();
        let mut used = BTreeSet::new();
        for (index, (arg, name)) in args.iter_mut().zip(names).enumerate() {
            let ordinal = name.as_ref().map_or(Some(index), |name| {
                member
                    .parameters
                    .iter()
                    .position(|(parameter, _, _)| parameter.eq_ignore_ascii_case(name))
            });
            let Some(ordinal) = ordinal
                .filter(|ordinal| *ordinal < member.parameters.len() && used.insert(*ordinal))
            else {
                self.issue(
                    "semantic.unknown-argument",
                    "invalid intrinsic argument",
                    arg.span,
                );
                continue;
            };
            self.expect(arg, &member.parameters[ordinal].1, "semantic.argument-type");
            bound.push(ordinal);
        }
        let defaults = self.call_defaults(&member.parameters, &used, location);
        CheckedCall {
            result: member.ty.clone(),
            target: Some(symbol),
            ordinals: bound,
            defaults,
        }
    }

    /// Returns declaration-order values for omitted parameters and reports each required gap.
    fn call_defaults(
        &mut self,
        parameters: &[(String, Type, ParameterDefault)],
        used: &BTreeSet<usize>,
        location: SourceSpan,
    ) -> Vec<Option<(Type, String)>> {
        parameters
            .iter()
            .enumerate()
            .map(|(index, (name, ty, declared))| {
                if used.contains(&index) {
                    return None;
                }
                if matches!(declared, ParameterDefault::Unknown) {
                    self.issue(
                        "semantic.default-unavailable",
                        format!("declaration does not record whether argument {name} has a default; provide it explicitly"),
                        location,
                    );
                    return None;
                }
                if let ParameterDefault::Literal(value) = declared {
                    return Some((ty.clone(), value.clone()));
                }
                if self.fill_missing_arguments
                    && let Some(value) = missing_argument_literal(ty)
                {
                    tracing::debug!(parameter = %name, ?ty, "filled required call argument");
                    self.result.diagnostics.push(
                        Diagnostic::new(
                            "semantic.argument-defaulted",
                            Severity::Warning,
                            format!("required argument {name} omitted; using {value}"),
                        )
                        .at(location),
                    );
                    return Some((ty.clone(), value.into()));
                }
                self.issue(
                    "semantic.argument-count",
                    format!("required argument {name} missing"),
                    location,
                );
                None
            })
            .collect()
    }

    fn binary_type(
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
