//! Parameter binding and statement checking for one shared semantic scope.
use super::*;

mod call;
mod expression;
mod resolution;

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
        // Visibility ends at the block boundary; HIR retains every storage identity.
        let outer_locals = self.locals.clone();
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
        self.locals = outer_locals;
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
                let target = self.expr_with_access(&expressions[0], false);
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
                    self.check_readable(&target);
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
            identity: definition.range.start,
        };
        let declaration = DeclarationFact {
            symbol: symbol.clone(),
            ty: ty.clone(),
            span: definition,
        };
        if self
            .script
            .members
            .get(&key(&name))
            .is_some_and(|member| member.kind == MemberKind::Variable)
        {
            self.issue(
                "semantic.local-member-conflict",
                format!("local {name} conflicts with a script variable"),
                definition,
            );
        }
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
}
