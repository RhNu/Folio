//! External and source script model, lookup, and type relations.
use super::*;

pub(super) fn script_from_external(external: &ExternalScript) -> ScriptInfo {
    let collect = |items: &[folio_format_declarations::Member]| {
        items
            .iter()
            .map(|member| (key(&member.name), member_from_external(member)))
            .collect::<BTreeMap<_, _>>()
    };
    let mut members = BTreeMap::new();
    let mut callable_overloads = BTreeMap::new();
    for item in &external.members {
        if item.kind() == MemberKind::Variable {
            continue;
        }
        let name = key(&item.name);
        let info = member_from_external(item);
        match (
            members.get(&name).map(|prior: &MemberInfo| prior.kind),
            item.kind(),
        ) {
            (
                Some(MemberKind::Property),
                MemberKind::Function | MemberKind::Event | MemberKind::UnknownCallable,
            ) => {
                callable_overloads.insert(name, info);
            }
            (
                Some(MemberKind::Function | MemberKind::Event | MemberKind::UnknownCallable),
                MemberKind::Property,
            ) => {
                callable_overloads.insert(name.clone(), members.insert(name, info).unwrap());
            }
            _ => {
                members.insert(name, info);
            }
        }
    }
    let variables = external
        .members
        .iter()
        .filter(|item| item.kind() == MemberKind::Variable)
        .map(|item| (key(&item.name), member_from_external(item)))
        .collect();
    let states = external
        .states
        .iter()
        .map(|state| (key(&state.name), collect(&state.members)))
        .collect();
    ScriptInfo {
        name: external.name.clone(),
        parent: external.parent.clone(),
        definition: None,
        members,
        variables,
        callable_overloads,
        states,
    }
}

/// Check retained external constants before member lookup discards storage details.
/// Missing values remain unknown, including PEX-derived AutoReadOnly properties.
pub(super) fn validate_external_initializers(
    external: &ExternalScript,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for member in &external.members {
        let (Some(literal), Some(ty)) = (member.initial_literal(), member.ty()) else {
            continue;
        };
        if !folio_papyrus::constant_literal_matches_type(literal, ty) {
            tracing::debug!(script = %external.name, member = %member.name, declared_type = %ty, "rejected external member initializer");
            diagnostics.push(Diagnostic::new(
                "semantic.initializer-type",
                Severity::Error,
                format!(
                    "external {}.{} initializer is not a valid literal compatible with {ty}",
                    external.name, member.name
                ),
            ));
        }
    }
}

fn member_from_external(member: &folio_format_declarations::Member) -> MemberInfo {
    MemberInfo {
        name: member.name.clone(),
        ty: member.ty().map(Type::from_spelling).unwrap_or(Type::Void),
        kind: member.kind(),
        parameters: member
            .parameters()
            .iter()
            .map(|parameter| {
                (
                    parameter.name.clone(),
                    Type::from_spelling(&parameter.ty),
                    parameter.default.clone(),
                )
            })
            .collect(),
        global: member.is_global(),
        auto: member.is_auto(),
        read_only: member.is_read_only(),
        readable: member.kind() != MemberKind::Property || member.is_readable(),
        writable: member.kind() != MemberKind::Property || member.is_writable(),
        definition: None,
    }
}

pub(super) fn member_from_source(
    file: FileId,
    node: &SyntaxNode,
    declaration: &Declaration,
) -> Option<MemberInfo> {
    let (name, ty, kind, parameters, global) = match declaration {
        Declaration::Function {
            name,
            return_type,
            parameters,
            modifiers,
        } => (
            name,
            return_type
                .as_deref()
                .map(Type::from_spelling)
                .unwrap_or(Type::Void),
            MemberKind::Function,
            parameters
                .iter()
                .map(|parameter| {
                    (
                        parameter.name.clone(),
                        Type::from_spelling(&parameter.ty),
                        parameter
                            .default
                            .clone()
                            .map(ParameterDefault::Literal)
                            .unwrap_or(ParameterDefault::Required),
                    )
                })
                .collect(),
            modifiers
                .iter()
                .any(|modifier| modifier.eq_ignore_ascii_case("global")),
        ),
        Declaration::Variable { name, ty, .. } => (
            name,
            Type::from_spelling(ty),
            MemberKind::Variable,
            Vec::new(),
            false,
        ),
        Declaration::Property { name, ty, .. } => (
            name,
            Type::from_spelling(ty),
            MemberKind::Property,
            Vec::new(),
            false,
        ),
        Declaration::Event {
            name, parameters, ..
        } => (
            name,
            Type::Void,
            MemberKind::Event,
            parameters
                .iter()
                .map(|parameter| {
                    (
                        parameter.name.clone(),
                        Type::from_spelling(&parameter.ty),
                        parameter
                            .default
                            .clone()
                            .map(ParameterDefault::Literal)
                            .unwrap_or(ParameterDefault::Required),
                    )
                })
                .collect(),
            false,
        ),
        _ => return None,
    };
    let definition = node
        .children_with_tokens()
        .filter_map(|item| item.into_token())
        .find(|token| token.kind() == SyntaxKind::Ident && token.text().eq_ignore_ascii_case(name))
        .map(|token| token_span(file, &token))
        .or_else(|| Some(span(file, node)));
    Some(MemberInfo {
        name: name.clone(),
        ty,
        kind,
        parameters,
        global,
        auto: matches!(declaration, Declaration::Property { flags, .. } if flags.iter().any(|flag| flag == "auto" || flag == "autoreadonly")),
        read_only: matches!(declaration, Declaration::Property { flags, .. } if flags.iter().any(|flag| flag == "autoreadonly")),
        readable: !matches!(declaration, Declaration::Property { flags, .. } if !flags.iter().any(|flag| flag == "auto" || flag == "autoreadonly") && !node.descendants().any(|child| super::file::callable_name(&child).is_some_and(|name| name.eq_ignore_ascii_case("get")))),
        writable: !matches!(declaration, Declaration::Property { flags, .. } if flags.iter().any(|flag| flag == "autoreadonly") || (!flags.iter().any(|flag| flag == "auto") && !node.descendants().any(|child| super::file::callable_name(&child).is_some_and(|name| name.eq_ignore_ascii_case("set"))))),
        definition,
    })
}

pub(super) fn validate_world(
    world: &World,
    analysis: &mut Analysis,
    file_scripts: &BTreeMap<FileId, String>,
) {
    for script in world
        .scripts
        .values()
        .filter(|script| script.definition.is_none())
    {
        if let Some(parent) = &script.parent
            && !world.scripts.contains_key(&key(parent))
        {
            analysis.project_diagnostics.push(Diagnostic::new(
                "semantic.unknown-parent",
                Severity::Error,
                format!(
                    "external script {} has unknown parent {parent}",
                    script.name
                ),
            ));
        }
        let mut visited = HashSet::new();
        let mut current = Some(key(&script.name));
        while let Some(name) = current {
            if !visited.insert(name.clone()) {
                analysis.project_diagnostics.push(Diagnostic::new(
                    "semantic.inheritance-cycle",
                    Severity::Error,
                    format!(
                        "external script {} participates in an inheritance cycle",
                        script.name
                    ),
                ));
                break;
            }
            current = world
                .scripts
                .get(&name)
                .and_then(|item| item.parent.as_ref())
                .map(|parent| key(parent));
        }
        for member in script
            .members
            .values()
            .chain(script.states.values().flat_map(|members| members.values()))
        {
            for (name, ty, default) in &member.parameters {
                if let ParameterDefault::Literal(text) = default {
                    let actual = literal_type(text);
                    if !constant_matches_type(text, ty) {
                        analysis.project_diagnostics.push(Diagnostic::new(
                            "semantic.parameter-default-type", Severity::Error,
                            format!("external {}.{} default for {name} is not a valid {ty:?} literal (found {actual:?})", script.name, member.name),
                        ));
                    }
                }
            }
            for ty in
                std::iter::once(&member.ty).chain(member.parameters.iter().map(|(_, ty, _)| ty))
            {
                if !known_type(world, ty) {
                    analysis.project_diagnostics.push(Diagnostic::new(
                        "semantic.unknown-type",
                        Severity::Error,
                        format!(
                            "external member {}.{} has unknown type {ty:?}",
                            script.name, member.name
                        ),
                    ));
                }
            }
        }
    }
    for (&file, script_key) in file_scripts {
        let script = &world.scripts[script_key];
        let file_analysis = analysis.files.get_mut(&file).unwrap();
        validate_source_contracts(world, script, file_analysis);
        if let Some(parent) = &script.parent {
            if !world.scripts.contains_key(&key(parent)) {
                if let Some(parent_ref) = &file_analysis.script.parent {
                    file_analysis.diagnostics.push(diagnostic(
                        "semantic.unknown-parent",
                        format!("unknown parent {parent}"),
                        parent_ref.span,
                    ));
                }
            } else {
                let mut visited = HashSet::new();
                let mut current = Some(script_key.clone());
                while let Some(name) = current {
                    if !visited.insert(name.clone()) {
                        if let Some(parent_ref) = &file_analysis.script.parent {
                            file_analysis.diagnostics.push(diagnostic(
                                "semantic.inheritance-cycle",
                                "inheritance cycle",
                                parent_ref.span,
                            ));
                        }
                        break;
                    }
                    current = world
                        .scripts
                        .get(&name)
                        .and_then(|item| item.parent.as_ref())
                        .map(|parent| key(parent));
                }
            }
        }
        for member in script
            .members
            .values()
            .chain(script.states.values().flat_map(|members| members.values()))
        {
            for ty in
                std::iter::once(&member.ty).chain(member.parameters.iter().map(|(_, ty, _)| ty))
            {
                if !known_type(world, ty)
                    && let Some(definition) = member.definition
                {
                    file_analysis.diagnostics.push(diagnostic(
                        "semantic.unknown-type",
                        format!("unknown type {ty:?}"),
                        definition,
                    ));
                }
            }
        }
    }
    let source_keys = file_scripts.values().collect::<BTreeSet<_>>();
    for (script_key, script) in &world.scripts {
        if source_keys.contains(script_key) {
            continue;
        }
        if let Some(parent) = &script.parent {
            if !world.scripts.contains_key(&key(parent)) {
                analysis.project_diagnostics.push(Diagnostic::new(
                    "semantic.unknown-parent",
                    Severity::Error,
                    format!(
                        "external script {} has unknown parent {parent}",
                        script.name
                    ),
                ));
            } else {
                let mut visited = HashSet::new();
                let mut current = Some(script_key.clone());
                while let Some(name) = current {
                    if !visited.insert(name.clone()) {
                        analysis.project_diagnostics.push(Diagnostic::new(
                            "semantic.inheritance-cycle",
                            Severity::Error,
                            format!(
                                "external script {} participates in an inheritance cycle",
                                script.name
                            ),
                        ));
                        break;
                    }
                    current = world
                        .scripts
                        .get(&name)
                        .and_then(|item| item.parent.as_ref())
                        .map(|parent| key(parent));
                }
            }
        }
        for member in script
            .members
            .values()
            .chain(script.variables.values())
            .chain(script.callable_overloads.values())
        {
            for ty in
                std::iter::once(&member.ty).chain(member.parameters.iter().map(|(_, ty, _)| ty))
            {
                if !known_type(world, ty) {
                    analysis.project_diagnostics.push(Diagnostic::new(
                        "semantic.unknown-type",
                        Severity::Error,
                        format!(
                            "external script {} member {} refers to unknown type {ty:?}",
                            script.name, member.name
                        ),
                    ));
                }
            }
        }
    }
}

/// Inherited contracts are checked even when no body or caller exists.
fn validate_source_contracts(world: &World, script: &ScriptInfo, result: &mut FileAnalysis) {
    for member in script
        .members
        .values()
        .filter(|member| member.kind == MemberKind::Property)
    {
        if script
            .parent
            .as_ref()
            .and_then(|parent| lookup_member(world, parent, &member.name))
            .is_some_and(|(_, inherited)| inherited.kind == MemberKind::Property)
        {
            result.diagnostics.push(diagnostic(
                "semantic.property-override",
                "inherited properties cannot be redefined",
                member.definition.unwrap(),
            ));
        }
    }
    for (state, members) in &script.states {
        for member in members.values() {
            let Some(at) = member.definition else {
                continue;
            };
            let contract =
                lookup_callable_member(world, &script.name, &member.name).map(|(_, member)| member);
            // Skyrim lifecycle handlers have an implicit parameterless event contract.
            let lifecycle = ["OnBeginState", "OnEndState"]
                .iter()
                .any(|name| member.name.eq_ignore_ascii_case(name));
            let valid = if let Some(contract) = contract {
                member.kind == contract.kind
                    && same_type(&member.ty, &contract.ty)
                    && member.parameters.len() == contract.parameters.len()
                    && member
                        .parameters
                        .iter()
                        .zip(&contract.parameters)
                        .all(|(actual, expected)| same_type(&actual.1, &expected.1))
            } else {
                lifecycle
                    && member.kind == MemberKind::Event
                    && member.ty == Type::Void
                    && member.parameters.is_empty()
            };
            if !valid {
                result.diagnostics.push(diagnostic(
                    "semantic.state-contract",
                    format!(
                        "state {state} callable {} requires a matching empty-state declaration",
                        member.name
                    ),
                    at,
                ));
            }
        }
    }
}

fn literal_type(text: &str) -> Type {
    let text = text.trim();
    match text.to_ascii_lowercase().as_str() {
        "true" | "false" => Type::Bool,
        "none" => Type::None,
        _ if text.starts_with('"') => Type::String,
        _ if text.contains('.') => Type::Float,
        _ => Type::Int,
    }
}

fn constant_matches_type(text: &str, ty: &Type) -> bool {
    fn spelling(ty: &Type) -> String {
        match ty {
            Type::Void => "void".into(),
            Type::Int => "int".into(),
            Type::Float => "float".into(),
            Type::Bool => "bool".into(),
            Type::String => "string".into(),
            Type::Script(name) => name.clone(),
            Type::Array(element) => format!("{}[]", spelling(element)),
            Type::None => "none".into(),
            Type::Error => String::new(),
        }
    }
    matches!(ty, Type::Error) || folio_papyrus::constant_literal_matches_type(text, &spelling(ty))
}

pub(super) fn known_type(world: &World, ty: &Type) -> bool {
    match ty {
        Type::Script(name) => world.scripts.contains_key(&key(name)),
        Type::Array(element) => known_type(world, element),
        _ => true,
    }
}

pub(super) fn lookup_member<'a>(
    world: &'a World,
    script: &str,
    name: &str,
) -> Option<(&'a ScriptInfo, &'a MemberInfo)> {
    let mut current = Some(key(script));
    let mut visited = BTreeSet::new();
    while let Some(script_key) = current {
        if !visited.insert(script_key.clone()) {
            break;
        }
        let owner = world.scripts.get(&script_key)?;
        if let Some(member) = owner
            .variables
            .get(&key(name))
            .or_else(|| owner.members.get(&key(name)))
        {
            return Some((owner, member));
        }
        current = owner.parent.as_ref().map(|parent| key(parent));
    }
    None
}

/// Calls prefer a callable with the same name as a property in external declaration sources.
pub(super) fn lookup_callable_member<'a>(
    world: &'a World,
    script: &str,
    name: &str,
) -> Option<(&'a ScriptInfo, &'a MemberInfo)> {
    let mut current = Some(key(script));
    let mut visited = BTreeSet::new();
    while let Some(script_key) = current {
        if !visited.insert(script_key.clone()) {
            break;
        }
        let owner = world.scripts.get(&script_key)?;
        if let Some(member) = owner
            .callable_overloads
            .get(&key(name))
            .or_else(|| owner.members.get(&key(name)))
            && matches!(
                member.kind,
                MemberKind::Function | MemberKind::Event | MemberKind::UnknownCallable
            )
        {
            return Some((owner, member));
        }
        current = owner.parent.as_ref().map(|parent| key(parent));
    }
    None
}

pub(super) fn lookup_state_member<'a>(
    world: &'a World,
    script: &str,
    state: &str,
    name: &str,
) -> Option<(&'a ScriptInfo, &'a MemberInfo)> {
    let owner = world.scripts.get(&key(script))?;
    let member = owner.states.get(&key(state))?.get(&key(name))?;
    Some((owner, member))
}

fn inherits(world: &World, subtype: &str, supertype: &str) -> bool {
    let mut current = Some(key(subtype));
    let mut visited = BTreeSet::new();
    while let Some(script_key) = current {
        if script_key == key(supertype) {
            return true;
        }
        if !visited.insert(script_key.clone()) {
            break;
        }
        current = world
            .scripts
            .get(&script_key)
            .and_then(|script| script.parent.as_ref())
            .map(|parent| key(parent));
    }
    false
}

pub(super) fn assignable(world: &World, actual: &Type, expected: &Type) -> bool {
    if same_type(actual, expected)
        || matches!(actual, Type::Error)
        || matches!(expected, Type::Error)
    {
        return true;
    }
    match (actual, expected) {
        (Type::Int, Type::Float) => true,
        (Type::None, Type::Script(_) | Type::Array(_)) => true,
        (Type::Script(child), Type::Script(parent)) => inherits(world, child, parent),
        _ => false,
    }
}

fn same_type(actual: &Type, expected: &Type) -> bool {
    match (actual, expected) {
        (Type::Script(left), Type::Script(right)) => left.eq_ignore_ascii_case(right),
        (Type::Array(left), Type::Array(right)) => same_type(left, right),
        _ => actual == expected,
    }
}

/// Bool coercion is valid in value contexts, but must not make `x == None`
/// legal for primitive values through the equality compatibility check.
pub(super) fn implicitly_convertible(world: &World, actual: &Type, expected: &Type) -> bool {
    assignable(world, actual, expected)
        || matches!(
            (actual, expected),
            (
                Type::Bool | Type::Int | Type::Float | Type::Script(_) | Type::Array(_),
                Type::String
            )
        )
        || matches!(
            (actual, expected),
            (
                Type::Int
                    | Type::Float
                    | Type::String
                    | Type::Script(_)
                    | Type::Array(_)
                    | Type::None,
                Type::Bool
            )
        )
}

pub(super) fn castable(world: &World, from: &Type, to: &Type) -> bool {
    if implicitly_convertible(world, from, to) || matches!(from, Type::Error) {
        return true;
    }
    matches!(
        (from, to),
        (Type::Float, Type::Int) | (Type::Bool | Type::String, Type::Int | Type::Float)
    ) || matches!((from, to), (Type::Script(child), Type::Script(parent)) if inherits(world, child, parent) || inherits(world, parent, child))
}
