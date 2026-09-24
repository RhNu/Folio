//! Lower typed function bodies to source-mapped MIR.
use super::*;

pub(super) struct FunctionLowerer<'a> {
    source: &'a folio_hir::Script,
    member: &'a MemberFact,
    body: &'a folio_hir::Body,
    target: TargetProfile,
    function: Function,
    slots: HashMap<Symbol, String>,
    used_names: BTreeSet<String>,
    generated_temps: BTreeSet<String>,
    next_temp: u32,
    next_label: u32,
    errors: Vec<Diagnostic>,
    decisions: Vec<Decision>,
    user_flags: &'a [(String, u8)],
}

struct CallCapture {
    instruction: usize,
    original: Value,
    copied: Value,
    ordinal: Option<usize>,
}

impl<'a> FunctionLowerer<'a> {
    pub(super) fn new(
        source: &'a folio_hir::Script,
        member: &'a MemberFact,
        body: &'a folio_hir::Body,
        target: TargetProfile,
        user_flags: &'a [(String, u8)],
    ) -> Self {
        let (event, global, native) = match &member.kind {
            MemberKind::Function {
                event,
                global,
                native,
            } => (*event, *global, *native),
            _ => (false, false, false),
        };
        let name = member_name(&body.symbol).unwrap_or_default().to_owned();
        let state = match &body.symbol {
            Symbol::StateMember { state, .. } => state.clone(),
            _ => String::new(),
        };
        let mut function = Function {
            name,
            state,
            return_type: type_name(&body.return_type).unwrap_or_else(|| "None".into()),
            parameters: Vec::new(),
            locals: Vec::new(),
            instructions: Vec::new(),
            flags: 0,
            is_global: global,
            is_native: native,
            is_event: event,
            source: member.span,
        };
        let mut slots = HashMap::new();
        let mut used_names = BTreeSet::new();
        let parameters = body
            .parameters
            .iter()
            .map(|parameter| (parameter.name.clone(), parameter.ty.clone()));
        for (parameter_name, parameter_ty) in parameters {
            let slot = parameter_name.clone();
            used_names.insert(slot.to_lowercase());
            function.parameters.push(Local {
                name: slot.clone(),
                ty: type_name(&parameter_ty).unwrap_or_else(|| "None".into()),
            });
            slots.insert(
                Symbol::Parameter {
                    owner: Box::new(body.symbol.clone()),
                    name: parameter_name,
                },
                slot,
            );
        }
        Self {
            source,
            member,
            body,
            target,
            function,
            slots,
            used_names,
            generated_temps: BTreeSet::new(),
            next_temp: 0,
            next_label: 0,
            errors: Vec::new(),
            decisions: Vec::new(),
            user_flags,
        }
    }

    fn issue(&mut self, code: &str, message: impl Into<String>, span: SourceSpan) {
        self.errors.push(reject(code, message, span));
    }

    fn emit(&mut self, op: Op, source: SourceSpan) {
        self.function.instructions.push(Instruction { op, source });
    }

    fn temp(&mut self, ty: &Type) -> Value {
        let name = loop {
            let name = format!("::folio_temp{}", self.next_temp);
            self.next_temp += 1;
            if self.used_names.insert(name.to_lowercase()) {
                break name;
            }
        };
        self.function.locals.push(Local {
            name: name.clone(),
            ty: type_name(ty).unwrap_or_else(|| "None".into()),
        });
        self.generated_temps.insert(name.clone());
        Value::Identifier(name)
    }

    fn capture(&mut self, value: Value, ty: &Type, span: SourceSpan) -> Value {
        // Generated slots belong to completed expressions and are never written again.
        if !matches!(&value, Value::Identifier(name) if !name.eq_ignore_ascii_case("self") && !self.generated_temps.contains(name))
        {
            return value;
        }
        let dest = self.temp(ty);
        self.emit(Op::Assign(dest.clone(), value), span);
        dest
    }

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

    fn label(&mut self) -> u32 {
        let value = self.next_label;
        self.next_label += 1;
        value
    }

    pub(super) fn lower(&mut self) {
        self.function.flags = flag_bits(
            &self.member.flags,
            self.user_flags,
            self.member.span,
            &mut self.errors,
        );
        if self.function.is_native && !self.body.statements.is_empty() {
            self.issue(
                "target.native-body",
                "native function cannot contain a body",
                self.member.span,
            );
            return;
        }
        if !self.function.is_native
            && self.body.return_type != Type::Void
            && !returns_on_all_paths(&self.body.statements)
        {
            self.issue(
                "target.missing-return",
                "non-void function may complete without a return value",
                self.member.span,
            );
            return;
        }
        for statement in &self.body.statements {
            self.statement(statement);
        }
        if !self.function.is_native && folio_mir::reaches_end(&self.function.instructions) {
            if self.body.return_type == Type::Void {
                self.emit(Op::Return(Value::None), self.member.span);
            } else {
                self.issue(
                    "target.missing-return",
                    "non-void function may complete without a return value",
                    self.member.span,
                );
            }
        }
    }

    pub(super) fn finish(self) -> (Option<Function>, Vec<Diagnostic>, Vec<Decision>) {
        let function = self.errors.is_empty().then_some(self.function);
        (function, self.errors, self.decisions)
    }

    fn slot(&mut self, symbol: &Symbol, ty: &Type, span: SourceSpan) -> Option<String> {
        if let Some(value) = self.slots.get(symbol) {
            return Some(value.clone());
        }
        if let Symbol::Local { name, .. } = symbol {
            let mut slot = name.clone();
            if !self.used_names.insert(slot.to_lowercase()) {
                slot = format!("::folio_local{}", self.next_temp);
                self.next_temp += 1;
                self.used_names.insert(slot.to_lowercase());
            }
            self.function.locals.push(Local {
                name: slot.clone(),
                ty: type_name(ty).unwrap_or_else(|| "None".into()),
            });
            self.slots.insert(symbol.clone(), slot.clone());
            return Some(slot);
        }
        self.issue(
            "lowering.missing-slot",
            "local or parameter slot is unavailable",
            span,
        );
        None
    }

    fn statement(&mut self, statement: &Statement) {
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
                            Some(MemberKind::Property { auto: true, .. })
                                if self
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

    fn expr(&mut self, expr: &ExpressionFact) -> Option<Value> {
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
                            Some(MemberKind::Property { auto: true, .. })
                                if self
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
                if operator == "-"
                    && expr.ty == Type::Int
                    && matches!(&operand.kind, ExpressionKind::Literal(text) if text.trim() == "2147483648")
                {
                    Value::Int(i32::MIN)
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

    fn call(
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
        let mut receiver = if let ExpressionKind::Member { owner, .. } = &callee.kind {
            if is_global {
                None
            } else {
                let value = self.expr(owner)?;
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
        let mut removable = Vec::new();
        let mut redundant_slots = BTreeSet::new();
        for capture in &captures {
            if self.call_source_is_stable(capture) {
                if let Some(ordinal) = capture.ordinal {
                    ordered[ordinal] = Some(capture.original.clone());
                } else {
                    receiver = Some(capture.original.clone());
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
                }
                _ => return None,
            };
            Op::CallStatic {
                script,
                name,
                dest: dest.clone(),
                args,
            }
        } else if matches!(&callee.kind, ExpressionKind::Member { owner, .. } if matches!(&owner.kind, ExpressionKind::Reference(reference) if reference.text.eq_ignore_ascii_case("parent")))
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

fn returns_on_all_paths(statements: &[Statement]) -> bool {
    statements.iter().any(|statement| match statement {
        Statement::Return { .. } => true,
        Statement::If {
            then_branch,
            else_if,
            else_branch,
            ..
        } => {
            !else_branch.is_empty()
                && returns_on_all_paths(then_branch)
                && else_if
                    .iter()
                    .all(|(_, branch)| returns_on_all_paths(branch))
                && returns_on_all_paths(else_branch)
        }
        _ => false,
    })
}

enum Place {
    Slot(Value),
    Property { name: String, receiver: Value },
    Array { array: Value, index: Value },
}

fn binary_op(operator: &str, ty: &Type) -> Option<BinaryOp> {
    let float = ty == &Type::Float;
    Some(match operator {
        "+" if ty == &Type::String => BinaryOp::AddString,
        "+" if float => BinaryOp::AddFloat,
        "+" => BinaryOp::AddInt,
        "-" if float => BinaryOp::SubFloat,
        "-" => BinaryOp::SubInt,
        "*" if float => BinaryOp::MulFloat,
        "*" => BinaryOp::MulInt,
        "/" if float => BinaryOp::DivFloat,
        "/" => BinaryOp::DivInt,
        "%" if !float => BinaryOp::ModInt,
        "==" | "!=" => BinaryOp::Eq,
        "<" => BinaryOp::Lt,
        "<=" => BinaryOp::Lte,
        ">" => BinaryOp::Gt,
        ">=" => BinaryOp::Gte,
        _ => return None,
    })
}
