//! Callable argument binding, defaults, and intrinsic signatures.
use super::{
    BTreeSet, CheckedCall, Diagnostic, ExpressionFact, ExpressionKind, MemberInfo, MemberKind,
    ParameterDefault, PropertyForm, Scope, Severity, SourceSpan, Symbol, Type,
    lookup_callable_member, lookup_state_member, missing_argument_literal,
};

impl Scope<'_> {
    pub(super) fn check_call(
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
            },
            Symbol::StateMember {
                script,
                state,
                name,
            } => lookup_state_member(self.world, script, state, name).map(|(_, member)| member),
            Symbol::Intrinsic { name } => {
                return self.resolve_intrinsic_call(
                    callee,
                    name,
                    args,
                    names,
                    location,
                    binding.symbol.clone(),
                );
            },
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
                    .position(|(parameter, ..)| parameter.eq_ignore_ascii_case(name))
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

    /// Materialize the intrinsic signature before binding call arguments.
    fn resolve_intrinsic_call(
        &mut self,
        callee: &ExpressionFact,
        name: &str,
        args: &mut [ExpressionFact],
        names: &[Option<String>],
        location: SourceSpan,
        symbol: Symbol,
    ) -> CheckedCall {
        let receiver = match &callee.kind {
            ExpressionKind::Member { owner, .. } => Some(&owner.ty),
            _ => None,
        };
        let Some(signature) = crate::intrinsic_signature(name, receiver) else {
            return CheckedCall::error();
        };
        let result = signature.result;
        let params = signature
            .parameters
            .into_iter()
            .map(|(name, ty, default)| {
                (
                    name,
                    ty,
                    default.map_or(ParameterDefault::Required, ParameterDefault::Literal),
                )
            })
            .collect();
        let intrinsic = MemberInfo {
            name: name.to_owned(),
            ty: result,
            kind: MemberKind::Function,
            parameters: params,
            global: false,
            property: PropertyForm {
                auto: false,
                read_only: false,
            },
            readable: true,
            writable: true,
            definition: None,
        };
        self.check_intrinsic_call(&intrinsic, args, names, location, symbol)
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
                    .position(|(parameter, ..)| parameter.eq_ignore_ascii_case(name))
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
}
