use std::collections::BTreeSet;

use crate::{
    DeclarationBundle, DecodeError, FORMAT, Member, MemberData, PROFILE, ParameterDefault,
    PropertyAccess, SCHEMA_VERSION,
};

pub(crate) const MAX_BYTES: usize = 128 * 1024 * 1024;
pub(crate) const MAX_STRING: usize = 1024 * 1024;
pub(crate) const MAX_CONTAINER: usize = 100_000;
pub(crate) const MAX_VALUES: usize = 2_000_000;
pub(crate) const MAX_DEPTH: usize = 64;

pub(crate) fn invalid(field: impl Into<String>, reason: &'static str) -> DecodeError {
    DecodeError::Invalid {
        field: field.into(),
        reason,
    }
}

/// Validate carrier invariants; type resolution belongs to the analysis layer.
///
/// # Errors
/// Returns an error for unsupported schemas or profiles, invalid fields, duplicate identities, or capacity violations.
pub fn validate(bundle: &DeclarationBundle) -> Result<(), DecodeError> {
    if bundle.format != FORMAT {
        return Err(invalid("format", "expected folio-declarations"));
    }
    if !matches!(bundle.schema, 1 | SCHEMA_VERSION) {
        return Err(DecodeError::UnsupportedSchema(bundle.schema));
    }
    if bundle.profile != PROFILE {
        return Err(invalid("profile", "unsupported language/ABI profile"));
    }
    validate_origin(&bundle.origin)?;
    container("scripts", bundle.scripts.len())?;
    let mut names = BTreeSet::new();
    let mut values = 0usize;
    for (index, script) in bundle.scripts.iter().enumerate() {
        let field = format!("scripts[{index}]");
        text(&format!("{field}.name"), &script.name)?;
        documentation(&field, script.documentation.as_deref(), bundle.schema)?;
        if !names.insert(script.name.to_ascii_lowercase()) {
            return Err(invalid(field, "duplicate script identity"));
        }
        if let Some(parent) = &script.parent {
            text(&format!("{field}.parent"), parent)?;
        }
        strings(&format!("{field}.flags"), &script.flags)?;
        strings(&format!("{field}.imports"), &script.imports)?;
        values += script.flags.len() + script.imports.len() + 1;
        if let Some(source) = &script.source {
            text(&format!("{field}.source.path"), &source.path)?;
            if source.path.contains('\\')
                || source.path.starts_with('/')
                || source.path.contains(':')
                || source
                    .path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err(invalid(
                    format!("{field}.source.path"),
                    "must be a portable relative path",
                ));
            }
            if source.line == 0 || source.column == 0 {
                return Err(invalid(
                    format!("{field}.source"),
                    "line and column are one-based",
                ));
            }
        }
        validate_members(
            &format!("{field}.members"),
            &script.members,
            false,
            bundle.schema,
            &mut values,
        )?;
        container(&format!("{field}.states"), script.states.len())?;
        let mut states = BTreeSet::new();
        let mut auto_state = false;
        for (index, state) in script.states.iter().enumerate() {
            let field = format!("{field}.states[{index}]");
            text(&format!("{field}.name"), &state.name)?;
            documentation(&field, state.documentation.as_deref(), bundle.schema)?;
            if !states.insert(state.name.to_ascii_lowercase()) {
                return Err(invalid(field, "duplicate state identity"));
            }
            if state.auto && auto_state {
                return Err(invalid(field, "multiple auto states"));
            }
            auto_state |= state.auto;
            validate_members(
                &format!("{field}.members"),
                &state.members,
                true,
                bundle.schema,
                &mut values,
            )?;
        }
        if values > MAX_VALUES {
            return Err(invalid("scripts", "too many declaration values"));
        }
    }
    Ok(())
}

/// Check portable generation provenance separately from declaration structure.
fn validate_origin(origin: &crate::Origin) -> Result<(), DecodeError> {
    text("origin.source", &origin.source)?;
    if origin.source.contains('\\')
        || origin.source.starts_with('/')
        || origin.source.contains(':')
        || origin
            .source
            .split('/')
            .any(|part| part == "." || part == ".." || part.is_empty())
    {
        return Err(invalid(
            "origin.source",
            "must be a portable provenance label",
        ));
    }
    if let Some(digest) = &origin.input_digest
        && (digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(invalid(
            "origin.input_digest",
            "expected a BLAKE3 hexadecimal digest",
        ));
    }
    Ok(())
}

fn validate_members(
    field: &str,
    members: &[Member],
    state: bool,
    schema: u32,
    values: &mut usize,
) -> Result<(), DecodeError> {
    container(field, members.len())?;
    let mut names = BTreeSet::new();
    for (index, member) in members.iter().enumerate() {
        let field = format!("{field}[{index}]");
        text(&format!("{field}.name"), &member.name)?;
        documentation(&field, member.documentation.as_deref(), schema)?;
        // All callables share a namespace; properties and variables remain separate.
        let namespace = match member.data {
            MemberData::Property { .. } => 1,
            MemberData::Variable { .. } => 2,
            _ => 0,
        };
        if !names.insert((namespace, member.name.to_ascii_lowercase())) {
            return Err(invalid(field, "duplicate member in the same namespace"));
        }
        if state && namespace != 0 {
            return Err(invalid(field, "states contain only callables"));
        }
        if state && member.is_global() {
            return Err(invalid(field, "state callables cannot be global"));
        }
        strings(&format!("{field}.flags"), &member.flags)?;
        if let Some(ty) = member.ty() {
            text(&format!("{field}.type"), ty)?;
        }
        if let Some(initial) = member.initial_literal() {
            text(&format!("{field}.initial_literal"), initial)?;
        }
        if matches!(
            member.data,
            MemberData::Property {
                access: PropertyAccess::Manual {
                    readable: false,
                    writable: false
                },
                ..
            }
        ) {
            return Err(invalid(
                field,
                "manual property requires at least one accessor",
            ));
        }
        let parameters = member.parameters();
        container(&format!("{field}.parameters"), parameters.len())?;
        let mut names = BTreeSet::new();
        // Defaults belong to individual parameter slots, including before required slots.
        for (index, parameter) in parameters.iter().enumerate() {
            let field = format!("{field}.parameters[{index}]");
            text(&format!("{field}.name"), &parameter.name)?;
            text(&format!("{field}.ty"), &parameter.ty)?;
            if !names.insert(parameter.name.to_ascii_lowercase()) {
                return Err(invalid(field, "duplicate parameter name"));
            }
            if let ParameterDefault::Literal(literal) = &parameter.default {
                text(&format!("{field}.default"), literal)?;
            }
        }
        *values += 1 + parameters.len() + member.flags.len();
        if *values > MAX_VALUES {
            return Err(invalid(field, "too many declaration values"));
        }
    }
    Ok(())
}

fn documentation(field: &str, value: Option<&str>, schema: u32) -> Result<(), DecodeError> {
    if let Some(value) = value {
        let field = format!("{field}.documentation");
        if schema == 1 {
            return Err(invalid(field, "documentation requires schema 2"));
        }
        text(&field, value)?;
    }
    Ok(())
}

fn text(field: &str, value: &str) -> Result<(), DecodeError> {
    if value.trim().is_empty() {
        return Err(invalid(field, "must not be empty"));
    }
    if value.len() > MAX_STRING {
        return Err(invalid(field, "string exceeds capacity limit"));
    }
    Ok(())
}

fn strings(field: &str, strings: &[String]) -> Result<(), DecodeError> {
    container(field, strings.len())?;
    for value in strings {
        text(field, value)?;
    }
    Ok(())
}

fn container(field: &str, len: usize) -> Result<(), DecodeError> {
    if len > MAX_CONTAINER {
        Err(invalid(field, "container exceeds capacity limit"))
    } else {
        Ok(())
    }
}
