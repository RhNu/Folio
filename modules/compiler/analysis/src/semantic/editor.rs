//! Candidate enumeration uses the checker world; protocol filtering stays in IDE.
use super::*;

pub(crate) fn completion_candidates(
    view: &AnalysisView,
    file: FileId,
    byte: usize,
    receiver: Option<&Type>,
    global: bool,
) -> Vec<crate::CompletionCandidate> {
    let analysis = view.semantic();
    let Some(facts) = analysis.file(file) else {
        return Vec::new();
    };
    let Some(name) = &facts.script.name else {
        return Vec::new();
    };
    let world = &analysis.world;
    let Some(current) = world.scripts.get(&key(&name.text)) else {
        return Vec::new();
    };
    let parse = view.parse(file).unwrap();
    let root = parse.syntax();
    let callable = root
        .descendants()
        .filter(|node| {
            matches!(
                node.kind(),
                SyntaxKind::FunctionDecl | SyntaxKind::EventDecl
            )
        })
        .find(|node| {
            let at = span(file, node);
            at.range.start <= byte && byte <= at.range.end
        });
    let owner = callable.as_ref().and_then(|node| {
        facts.script.members.iter().find(|member| {
            let at = span(file, node);
            at.range.start <= member.span.range.start && member.span.range.end <= at.range.end
        })
    });
    let context_global = owner
        .is_some_and(|member| matches!(member.kind, HirMemberKind::Function { global: true, .. }));
    let mut names = BTreeMap::new();
    if let Some(Type::Array(_)) = receiver {
        return crate::intrinsics::intrinsic_candidates(receiver.unwrap());
    }
    if receiver.is_none() {
        for declaration in &facts.script.declarations {
            let local_owner = match &declaration.symbol {
                Symbol::Local { owner, .. } | Symbol::Parameter { owner, .. } => owner,
                _ => continue,
            };
            if !owner.is_some_and(|item| &item.symbol == local_owner.as_ref())
                || declaration.span.range.start > byte
            {
                continue;
            }
            if matches!(declaration.symbol, Symbol::Local { .. }) {
                let block = root
                    .descendants()
                    .filter(|node| node.kind() == SyntaxKind::Block)
                    .filter(|node| {
                        let at = span(file, node);
                        at.range.start <= declaration.span.range.start
                            && declaration.span.range.end <= at.range.end
                    })
                    .min_by_key(|node| node.text_range().len());
                if block.is_some_and(|node| {
                    let at = span(file, &node);
                    byte < at.range.start || byte > at.range.end
                }) {
                    continue;
                }
            }
            names.insert(
                key(symbol_name(&declaration.symbol)),
                crate::CompletionCandidate {
                    symbol: declaration.symbol.clone(),
                    ty: declaration.ty.clone(),
                    definition: Some(declaration.span),
                },
            );
        }
    }
    let receiver_name = match receiver {
        Some(Type::Script(name)) => name.as_str(),
        None => &current.name,
        _ => return Vec::new(),
    };
    let mut lineage = Some(key(receiver_name));
    let mut visited = BTreeSet::new();
    let mut possible = BTreeSet::new();
    while let Some(script_key) = lineage {
        if !visited.insert(script_key.clone()) {
            break;
        }
        let Some(script) = world.scripts.get(&script_key) else {
            break;
        };
        possible.extend(script.members.keys().cloned());
        possible.extend(script.variables.keys().cloned());
        possible.extend(script.callable_overloads.keys().cloned());
        if receiver.is_none() {
            possible.extend(
                script
                    .states
                    .values()
                    .flat_map(|state| state.keys().cloned()),
            );
        }
        lineage = script.parent.as_ref().map(|parent| key(parent));
    }
    let state = callable
        .as_ref()
        .and_then(|node| enclosing_name(node, SyntaxKind::StateDecl, "state"));
    for member_name in possible {
        let selected = state
            .as_ref()
            .filter(|_| receiver.is_none())
            .and_then(|state| lookup_state_member(world, receiver_name, state, &member_name));
        let is_state = selected.is_some();
        let Some((script, member)) =
            selected.or_else(|| lookup_member(world, receiver_name, &member_name))
        else {
            continue;
        };
        if (global || (receiver.is_none() && context_global)) && !member.global {
            continue;
        }
        if receiver.is_some() && !global && member.global {
            continue;
        }
        let symbol = if is_state {
            Symbol::StateMember {
                script: script.name.clone(),
                state: state.clone().unwrap(),
                name: member.name.clone(),
            }
        } else {
            Symbol::Member {
                script: script.name.clone(),
                name: member.name.clone(),
            }
        };
        names
            .entry(member_name)
            .or_insert(crate::CompletionCandidate {
                symbol,
                ty: member.ty.clone(),
                definition: member.definition,
            });
    }
    if receiver.is_none() {
        for script in world.scripts.values() {
            names
                .entry(key(&script.name))
                .or_insert(crate::CompletionCandidate {
                    symbol: Symbol::Script(script.name.clone()),
                    ty: Type::Script(script.name.clone()),
                    definition: script.definition,
                });
        }
        let imports = view.declarations(file).unwrap();
        let mut imported: BTreeMap<String, Vec<(&ScriptInfo, &MemberInfo)>> = BTreeMap::new();
        for declaration in imports.iter() {
            let Declaration::Import { name: import } = declaration else {
                continue;
            };
            let mut lineage = Some(key(import));
            let mut visited = BTreeSet::new();
            let mut member_names = BTreeSet::new();
            while let Some(script_key) = lineage {
                if !visited.insert(script_key.clone()) {
                    break;
                }
                let Some(script) = world.scripts.get(&script_key) else {
                    break;
                };
                member_names.extend(script.members.keys().cloned());
                lineage = script.parent.as_ref().map(|parent| key(parent));
            }
            for member_name in member_names {
                if let Some((owner, member)) = lookup_member(world, import, &member_name)
                    && member.global
                {
                    imported
                        .entry(member_name)
                        .or_default()
                        .push((owner, member));
                }
            }
        }
        for (name_key, items) in imported {
            if let [(owner, member)] = items.as_slice() {
                names.entry(name_key).or_insert(crate::CompletionCandidate {
                    symbol: Symbol::Member {
                        script: owner.name.clone(),
                        name: member.name.clone(),
                    },
                    ty: member.ty.clone(),
                    definition: member.definition,
                });
            }
        }
    }
    if !global && !context_global {
        for candidate in
            crate::intrinsics::intrinsic_candidates(&Type::Script(receiver_name.into()))
        {
            names
                .entry(key(symbol_name(&candidate.symbol)))
                .or_insert(candidate);
        }
    }
    names.into_values().collect()
}

fn symbol_name(symbol: &Symbol) -> &str {
    match symbol {
        Symbol::ParentReceiver { .. } => "Parent",
        Symbol::Script(name)
        | Symbol::Intrinsic { name }
        | Symbol::Member { name, .. }
        | Symbol::StateMember { name, .. }
        | Symbol::PropertyAccessor { name, .. }
        | Symbol::Parameter { name, .. }
        | Symbol::Local { name, .. } => name,
    }
}
