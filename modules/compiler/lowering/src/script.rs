//! Script declaration, storage, and body assembly for target legalization.
use super::{
    BTreeSet, Decision, Diagnostic, FlagScope, Function, FunctionLowerer, Instruction, Local,
    MemberKind, Op, Outcome, Property, Script, SourceSpan, Symbol, TargetProfile, UserFlag,
    Variable, default_value, find_member, flag_bits, literal, member_name, reject, type_name,
};

/// Validate the stable flag allocation supplied by project planning.
pub(super) fn validate_flag_allocations(
    user_flags: &[UserFlag],
    span: SourceSpan,
    errors: &mut Vec<Diagnostic>,
) {
    let mut seen_flag_names = BTreeSet::new();
    let mut seen_flag_bits = BTreeSet::new();
    for definition in user_flags {
        let flag = &definition.name;
        let Some(bit) = definition.bit else {
            errors.push(reject(
                "target.flag-allocation",
                "user flag lacks an allocated bit",
                span,
            ));
            continue;
        };
        if flag.eq_ignore_ascii_case("hidden")
            || flag.eq_ignore_ascii_case("conditional")
            || !(2..32).contains(&bit)
            || !seen_flag_names.insert(flag.to_lowercase())
            || !seen_flag_bits.insert(bit)
        {
            errors.push(reject(
                "target.flag-allocation",
                format!("invalid or conflicting user flag allocation for {flag}"),
                span,
            ));
        }
    }
}

/// Lower variable and property declarations before attaching executable bodies.
pub(super) fn lower_members(
    source: &folio_hir::Script,
    user_flags: &[UserFlag],
    script: &mut Script,
    errors: &mut Vec<Diagnostic>,
) {
    for member in &source.members {
        let Some(ty) = type_name(&member.ty) else {
            errors.push(reject(
                "target.invalid-type",
                "type cannot be represented in Skyrim PEX",
                member.span,
            ));
            continue;
        };
        let Some(member_name) = member_name(&member.symbol) else {
            continue;
        };
        if matches!(member.kind, MemberKind::Function { .. })
            && (member_name.eq_ignore_ascii_case("GetState")
                || member_name.eq_ignore_ascii_case("GotoState"))
        {
            errors.push(reject(
                "target.reserved-state-method",
                "GetState and GotoState are compiler-provided methods",
                member.span,
            ));
            continue;
        }
        match &member.kind {
            MemberKind::Variable => {
                let initial = if let Some(text) = &member.initial_literal {
                    if let Some(value) = literal(text, &member.ty) {
                        value
                    } else {
                        errors.push(reject(
                            "target.variable-initializer",
                            "variable initial value must be a representable constant",
                            member.span,
                        ));
                        continue;
                    }
                } else {
                    default_value(&member.ty)
                };
                script.variables.push(Variable {
                    name: member_name.into(),
                    ty,
                    initial,
                    flags: flag_bits(
                        &member.flags,
                        user_flags,
                        FlagScope::Variable,
                        member.span,
                        errors,
                    ),
                    source: member.span,
                });
            },
            MemberKind::Property { auto, read_only } => lower_property(
                member,
                member_name,
                ty,
                (*auto, *read_only),
                user_flags,
                script,
                errors,
            ),
            MemberKind::Function { .. } => {},
        }
    }
}

/// Attach analyzed bodies to their already-created MIR declarations.
pub(super) fn lower_bodies(
    source: &folio_hir::Script,
    target: TargetProfile,
    user_flags: &[UserFlag],
    script: &mut Script,
    errors: &mut Vec<Diagnostic>,
) {
    for body in &source.bodies {
        let member = match find_member(source, &body.symbol) {
            Some(member) => member,
            None if matches!(body.symbol, Symbol::PropertyAccessor { .. }) => {
                // Accessors are emitted as property functions, not script members.
                let Symbol::PropertyAccessor { property, .. } = &body.symbol else {
                    unreachable!()
                };
                let Some(property_member) = source.members.iter().find(|member| {
                    member_name(&member.symbol)
                        .is_some_and(|name| name.eq_ignore_ascii_case(property))
                }) else {
                    errors.push(reject(
                        "lowering.missing-property",
                        "property accessor lacks a property declaration",
                        script.source,
                    ));
                    continue;
                };
                property_member
            },
            None => {
                errors.push(reject(
                    "lowering.missing-member",
                    "body lacks a declaration",
                    script.source,
                ));
                continue;
            },
        };
        let mut function = FunctionLowerer::new(source, member, body, target, user_flags);
        function.lower();
        let (function, mut function_errors, decisions) = function.finish();
        errors.append(&mut function_errors);
        script.decisions.extend(decisions);
        if let Some(function) = function {
            if let Symbol::PropertyAccessor { property, name, .. } = &body.symbol {
                if let Some(item) = script
                    .properties
                    .iter_mut()
                    .find(|item| item.name.eq_ignore_ascii_case(property))
                {
                    if name.eq_ignore_ascii_case("get") {
                        item.getter = Some(function);
                    } else if name.eq_ignore_ascii_case("set") {
                        item.setter = Some(function);
                    }
                }
            } else {
                script.functions.push(function);
            }
        }
    }
}

/// Native declarations need an ABI entry even though they carry no body.
pub(super) fn lower_native_members(
    source: &folio_hir::Script,
    user_flags: &[UserFlag],
    script: &mut Script,
    errors: &mut Vec<Diagnostic>,
) {
    // Native source declarations have no body but still need an ABI entry.
    for member in &source.members {
        let MemberKind::Function {
            event,
            global,
            native,
        } = &member.kind
        else {
            continue;
        };
        if !native
            || source
                .bodies
                .iter()
                .any(|body| body.symbol == member.symbol)
        {
            continue;
        }
        let Some(return_type) = type_name(&member.ty) else {
            continue;
        };
        let Some(name) = member_name(&member.symbol) else {
            continue;
        };
        script.functions.push(Function {
            name: name.into(),
            state: match &member.symbol {
                Symbol::StateMember { state, .. } => state.clone(),
                _ => String::new(),
            },
            return_type,
            parameters: member
                .parameters
                .iter()
                .filter_map(|item| {
                    type_name(&item.ty).map(|ty| Local {
                        name: item.name.clone(),
                        ty,
                    })
                })
                .collect(),
            locals: Vec::new(),
            instructions: Vec::new(),
            flags: flag_bits(
                &member.flags,
                user_flags,
                FlagScope::Function,
                member.span,
                errors,
            ),
            is_global: *global,
            is_native: true,
            is_event: *event,
            source: member.span,
        });
    }
}

/// Materialize an Auto property or the accessor declaration for a manual property.
fn lower_property(
    member: &folio_hir::MemberFact,
    member_name: &str,
    ty: String,
    form: (bool, bool),
    user_flags: &[UserFlag],
    script: &mut Script,
    errors: &mut Vec<Diagnostic>,
) {
    let (auto, read_only) = form;

    if read_only
        && member.flags.iter().any(|flag| {
            flag.eq_ignore_ascii_case("conditional")
                || user_flags.iter().any(|definition| {
                    definition.name.eq_ignore_ascii_case(flag)
                        && definition.applies_to(FlagScope::Variable)
                        && !definition.applies_to(FlagScope::Property)
                })
        })
    {
        errors.push(reject(
            "target.read-only-storage-flag",
            "AutoReadOnly has no variable storage for this flag",
            member.span,
        ));
        return;
    }
    if read_only && (!auto || member.initial_literal.is_none()) {
        errors.push(reject(
            "target.property-initializer",
            "AutoReadOnly requires an initialized generated property",
            member.span,
        ));
        return;
    }
    let auto_var = (auto && !read_only).then(|| format!("::{member_name}_var"));
    let getter = if auto {
        lower_auto_property(
            member,
            member_name,
            &ty,
            read_only,
            user_flags,
            script,
            errors,
        )
    } else {
        Ok(None)
    };
    let Ok(getter) = getter else {
        return;
    };
    script.properties.push(Property {
        name: member_name.into(),
        ty,
        auto_var,
        read_only,
        getter,
        setter: None,
        flags: flag_bits(
            &member.flags,
            user_flags,
            FlagScope::Property,
            member.span,
            errors,
        ),
        source: member.span,
    });
    if read_only {
        script.decisions.push(Decision {
            feature: "read-only-property",
            outcome: Outcome::Lowered {
                rule: "constant-getter",
            },
            source: member.span,
        });
    }
}

/// Emit a literal-returning getter or mutable backing slot for an Auto property.
fn lower_auto_property(
    member: &folio_hir::MemberFact,
    member_name: &str,
    ty: &str,
    read_only: bool,
    user_flags: &[UserFlag],
    script: &mut Script,
    errors: &mut Vec<Diagnostic>,
) -> Result<Option<Function>, ()> {
    let mut getter = None;
    {
        let initial = if let Some(text) = &member.initial_literal {
            if let Some(value) = literal(text, &member.ty) {
                value
            } else {
                errors.push(reject(
                    "target.property-initializer",
                    "property initial value must be a representable constant",
                    member.span,
                ));
                return Err(());
            }
        } else {
            default_value(&member.ty)
        };
        if read_only {
            // A source constant is represented by executable getter
            // code rather than a mutable slot in the saved object.
            getter = Some(Function {
                name: "Get".into(),
                state: String::new(),
                return_type: ty.to_owned(),
                parameters: Vec::new(),
                locals: Vec::new(),
                instructions: vec![Instruction {
                    op: Op::Return(initial),
                    source: member.span,
                }],
                flags: 0,
                is_global: false,
                is_native: false,
                is_event: false,
                source: member.span,
            });
        } else {
            script.variables.push(Variable {
                name: format!("::{member_name}_var"),
                ty: ty.to_owned(),
                initial,
                flags: flag_bits(
                    &member.flags,
                    user_flags,
                    FlagScope::Variable,
                    member.span,
                    errors,
                ),
                source: member.span,
            });
        }
    }
    Ok(getter)
}
