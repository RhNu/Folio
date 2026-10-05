//! Shared function lowering state, lifecycle, and MIR allocation.
use super::{
    BTreeSet, BinaryOp, Decision, Diagnostic, ExpressionFact, ExpressionKind, FlagScope, Function,
    HashMap, Instruction, Local, MemberFact, MemberKind, Op, Outcome, SourceSpan, Statement,
    Symbol, TargetProfile, Type, UnaryOp, UserFlag, Value, find_member, flag_bits, literal,
    member_name, reject, type_name,
};

/// Owns one function's MIR, diagnostics, and allocation counters across lowering phases.
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
    user_flags: &'a [UserFlag],
}

mod call;
mod expression;
mod statement;

impl<'a> FunctionLowerer<'a> {
    pub(super) fn new(
        source: &'a folio_hir::Script,
        member: &'a MemberFact,
        body: &'a folio_hir::Body,
        target: TargetProfile,
        user_flags: &'a [UserFlag],
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

    fn label(&mut self) -> u32 {
        let value = self.next_label;
        self.next_label += 1;
        value
    }

    pub(super) fn lower(&mut self) {
        self.function.flags = flag_bits(
            &self.member.flags,
            self.user_flags,
            FlagScope::Function,
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
        },
        _ => false,
    })
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
