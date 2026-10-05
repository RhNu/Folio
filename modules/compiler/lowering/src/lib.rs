//! Pure target legalization from typed HIR to source-mapped MIR.

use std::collections::{BTreeSet, HashMap};

use folio_diagnostics::{Diagnostic, Severity};
use folio_hir::{ExpressionFact, ExpressionKind, MemberFact, MemberKind, Statement, Symbol, Type};
use folio_mir::{
    BinaryOp, Decision, ExternalSlot, Function, Instruction, Local, Op, Outcome, Property, Script,
    UnaryOp, Value, Variable,
};
use folio_profiles::{FlagScope, TargetProfile, UserFlag};
use folio_source::SourceSpan;

fn reject(code: &str, message: impl Into<String>, source: SourceSpan) -> Diagnostic {
    Diagnostic::new(code, Severity::Error, message).at(source)
}

fn type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Void | Type::None => Some("None".into()),
        Type::Int => Some("Int".into()),
        Type::Float => Some("Float".into()),
        Type::Bool => Some("Bool".into()),
        Type::String => Some("String".into()),
        Type::Script(name) => Some(name.clone()),
        Type::Array(element)
            if !matches!(
                **element,
                Type::Array(_) | Type::Void | Type::None | Type::Error
            ) =>
        {
            Some(format!("{}[]", type_name(element)?))
        },
        _ => None,
    }
}

/// Papyrus widens signed integers with the VM's single-precision rounding.
#[expect(
    clippy::cast_precision_loss,
    reason = "Papyrus Int-to-Float conversion uses VM single-precision rounding"
)]
fn papyrus_float(value: i32) -> f32 { value as f32 }

fn literal(text: &str, ty: &Type) -> Option<Value> {
    let normalized = folio_hir::normalize_constant_literal_text(text)?;
    let text = normalized.as_str();
    match ty {
        Type::None => Some(Value::None),
        Type::Bool if text.eq_ignore_ascii_case("true") => Some(Value::Bool(true)),
        Type::Bool if text.eq_ignore_ascii_case("false") => Some(Value::Bool(false)),
        Type::Int => folio_hir::decode_integer_literal(text).map(Value::Int),
        Type::Float => text
            .parse::<f32>()
            .ok()
            .or_else(|| folio_hir::decode_integer_literal(text).map(papyrus_float))
            .filter(|n| n.is_finite())
            .map(Value::Float),
        Type::String if text.starts_with('"') && text.ends_with('"') && text.len() >= 2 => {
            folio_hir::decode_string_literal(text).map(Value::String)
        },
        _ if text.eq_ignore_ascii_case("none") => Some(Value::None),
        _ => None,
    }
}

fn default_value(ty: &Type) -> Value {
    match ty {
        Type::Int => Value::Int(0),
        Type::Float => Value::Float(0.0),
        Type::Bool => Value::Bool(false),
        Type::String => Value::String(String::new()),
        _ => Value::None,
    }
}

fn member_name(symbol: &Symbol) -> Option<&str> {
    match symbol {
        Symbol::Member { name, .. }
        | Symbol::StateMember { name, .. }
        | Symbol::PropertyAccessor { name, .. } => Some(name),
        _ => None,
    }
}

fn find_member<'a>(source: &'a folio_hir::Script, symbol: &Symbol) -> Option<&'a MemberFact> {
    source
        .members
        .iter()
        .chain(&source.external_members)
        .chain(&source.referenced_members)
        .find(|member| &member.symbol == symbol)
}

/// Legalize one source script using the project's stable custom flag bit allocation.
///
/// # Errors
/// Returns diagnostics for invalid declarations, unsupported target features, or invalid MIR.
#[tracing::instrument(skip(source, user_flags), fields(script = source.name.as_ref().map_or("<missing>", |name| name.text.as_str()), target = target.id))]
pub fn lower_script(
    source: &folio_hir::Script,
    target: TargetProfile,
    user_flags: &[UserFlag],
) -> Result<Script, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    if target.id != TargetProfile::skyrim_se().id {
        errors.push(Diagnostic::new(
            "target.unimplemented",
            Severity::Error,
            format!("no backend for {}", target.id),
        ));
        return Err(errors);
    }
    let Some(name) = &source.name else {
        errors.push(Diagnostic::new(
            "lowering.missing-script",
            Severity::Error,
            "script declaration is missing",
        ));
        return Err(errors);
    };
    validate_flag_allocations(user_flags, name.span, &mut errors);
    let mut script = Script {
        target,
        name: name.text.clone(),
        parent: source
            .parent
            .as_ref()
            .map(|name| name.text.clone())
            .unwrap_or_default(),
        flags: flag_bits(
            &source.flags,
            user_flags,
            FlagScope::Script,
            name.span,
            &mut errors,
        ),
        auto_state: source
            .states
            .iter()
            .find(|state| state.auto)
            .map(|state| state.name.clone())
            .unwrap_or_default(),
        variables: Vec::new(),
        external_slots: Vec::new(),
        properties: Vec::new(),
        functions: Vec::new(),
        state_names: source
            .states
            .iter()
            .map(|state| state.name.clone())
            .collect(),
        source: name.span,
        decisions: Vec::new(),
    };
    script.external_slots = external_slots(source);
    if source.states.len() + 1 > target.max_states as usize {
        errors.push(reject(
            "target.state-capacity",
            format!(
                "{} permits at most {} local states including the empty state",
                target.id, target.max_states
            ),
            name.span,
        ));
    }
    if source.states.iter().filter(|state| state.auto).count() > 1 {
        errors.push(reject(
            "target.multiple-auto-states",
            "only one auto state is permitted",
            name.span,
        ));
    }
    validate_function_shapes(source, &mut errors);
    lower_members(source, user_flags, &mut script, &mut errors);
    lower_bodies(source, target, user_flags, &mut script, &mut errors);
    lower_native_members(source, user_flags, &mut script, &mut errors);
    if let Err(spans) = folio_mir::validate(&script) {
        errors.extend(spans.into_iter().map(|error| {
            reject(
                "lowering.invalid-mir",
                format!("invalid MIR: {:?}", error.kind),
                error.source,
            )
        }));
    }
    if errors.is_empty() {
        tracing::debug!(
            functions = script.functions.len(),
            variables = script.variables.len(),
            decisions = script.decisions.len(),
            "target legalization complete"
        );
        Ok(script)
    } else {
        tracing::warn!(errors = errors.len(), "target legalization rejected script");
        Err(errors)
    }
}

/// Project inherited variable identities without emitting parent storage.
fn external_slots(source: &folio_hir::Script) -> Vec<ExternalSlot> {
    source
        .external_members
        .iter()
        .filter(|member| matches!(member.kind, MemberKind::Variable))
        .map(|member| ExternalSlot {
            owner: match &member.symbol {
                Symbol::Member { script, .. } => script.clone(),
                _ => String::new(),
            },
            name: member_name(&member.symbol).unwrap_or_default().into(),
            ty: type_name(&member.ty).unwrap_or_else(|| "None".into()),
        })
        .collect()
}

mod script;
use script::{lower_bodies, lower_members, lower_native_members, validate_flag_allocations};

fn validate_function_shapes(source: &folio_hir::Script, errors: &mut Vec<Diagnostic>) {
    let functions = source
        .members
        .iter()
        .chain(&source.external_members)
        .filter(|member| matches!(member.kind, MemberKind::Function { .. }))
        .collect::<Vec<_>>();
    for member in &source.members {
        let MemberKind::Function { event, .. } = member.kind else {
            continue;
        };
        let Some(name) = member_name(&member.symbol) else {
            continue;
        };
        if matches!(member.symbol, Symbol::StateMember { .. })
            && (name.eq_ignore_ascii_case("OnBeginState")
                || name.eq_ignore_ascii_case("OnEndState"))
            && (!event || member.ty != Type::Void || !member.parameters.is_empty())
        {
            errors.push(reject(
                "target.state-event-signature",
                "state lifecycle event must be a void event with no parameters",
                member.span,
            ));
        }
        for other in &functions {
            if member.symbol == other.symbol
                || !member_name(&other.symbol)
                    .is_some_and(|other_name| other_name.eq_ignore_ascii_case(name))
            {
                continue;
            }
            let same_script = match (&member.symbol, &other.symbol) {
                (
                    Symbol::Member { script: left, .. } | Symbol::StateMember { script: left, .. },
                    Symbol::Member { script: right, .. }
                    | Symbol::StateMember { script: right, .. },
                ) => left.eq_ignore_ascii_case(right),
                _ => false,
            };
            if same_script
                && matches!(
                    (&member.symbol, &other.symbol),
                    (Symbol::Member { .. }, Symbol::Member { .. })
                )
            {
                continue;
            }
            if member.ty != other.ty
                || member.parameters.len() != other.parameters.len()
                || member
                    .parameters
                    .iter()
                    .zip(&other.parameters)
                    .any(|(left, right)| left.ty != right.ty)
            {
                errors.push(reject(
                    "target.override-signature",
                    format!("{name} does not match another state or inherited signature"),
                    member.span,
                ));
                break;
            }
        }
    }
}

fn flag_bits(
    flags: &[String],
    user_flags: &[UserFlag],
    scope: FlagScope,
    span: SourceSpan,
    errors: &mut Vec<Diagnostic>,
) -> u32 {
    let mut bits = 0u32;
    for flag in flags {
        let bit = if flag.eq_ignore_ascii_case("hidden") {
            matches!(scope, FlagScope::Script | FlagScope::Property).then_some(0)
        } else if flag.eq_ignore_ascii_case("conditional") {
            matches!(scope, FlagScope::Script | FlagScope::Variable).then_some(1)
        } else if matches!(
            flag.to_ascii_lowercase().as_str(),
            "auto" | "autoreadonly" | "global" | "native"
        ) {
            None
        } else {
            user_flags
                .iter()
                .find(|definition| definition.name.eq_ignore_ascii_case(flag))
                .filter(|definition| definition.scopes.contains(&scope))
                .and_then(|definition| definition.bit)
        };
        if let Some(bit) = bit {
            if bit < 32 {
                bits |= 1u32 << bit;
            } else {
                errors.push(reject(
                    "target.flag-bit",
                    "user flag bit exceeds PEX limit",
                    span,
                ));
            }
        } else if !flag.eq_ignore_ascii_case("hidden")
            && !flag.eq_ignore_ascii_case("conditional")
            && !user_flags
                .iter()
                .any(|definition| definition.name.eq_ignore_ascii_case(flag))
            && !matches!(
                flag.to_ascii_lowercase().as_str(),
                "auto" | "autoreadonly" | "global" | "native"
            )
        {
            errors.push(reject(
                "target.unknown-flag",
                format!("unknown user flag {flag}"),
                span,
            ));
        }
    }
    bits
}

mod function;
use function::FunctionLowerer;

#[cfg(test)]
mod tests;
