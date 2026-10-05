//! Prefix selection uses indexed checker names before candidate materialization.
use super::*;

pub(crate) fn completion_candidates(
    view: &AnalysisView,
    file: FileId,
    byte: usize,
    receiver: Option<&Type>,
    global: bool,
    prefix: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<crate::CompletionCandidate>, AnalysisCancelled> {
    view.try_warm_semantics(cancelled)?;
    checkpoint(cancelled)?;
    let mut cancellation = Cancellation::new(cancelled);
    let analysis = view.semantic();
    let Some(facts) = analysis.file(file) else {
        return Ok(Vec::new());
    };
    let Some(name) = &facts.script.name else {
        return Ok(Vec::new());
    };
    let world = &analysis.world;
    let Some(current) = world.scripts.get(&key(&name.text)) else {
        return Ok(Vec::new());
    };
    let parse = view.parse(file).unwrap();
    let root = parse.syntax();
    let callable = byte
        .try_into()
        .ok()
        .filter(|offset| *offset <= root.text_range().end())
        .and_then(|offset| {
            let tokens = root.token_at_offset(offset);
            // Check both sides to retain inclusive callable boundaries at the cursor.
            [tokens.clone().left_biased(), tokens.right_biased()]
                .into_iter()
                .flatten()
                .filter_map(|token| token.parent())
                .flat_map(|parent| parent.ancestors())
                .find(|node| {
                    matches!(
                        node.kind(),
                        SyntaxKind::FunctionDecl | SyntaxKind::EventDecl
                    )
                })
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
    let indexed_prefix = key(prefix);
    if let Some(receiver @ Type::Array(_)) = receiver {
        for candidate in crate::intrinsics::intrinsic_candidates(receiver, prefix) {
            names.insert(key(symbol_name(&candidate.symbol)), Some(candidate));
        }
        checkpoint(cancelled)?;
        return Ok(names.into_values().flatten().collect());
    }
    if receiver.is_none() {
        for declaration in &facts.script.declarations {
            cancellation.check()?;
            let name_key = key(symbol_name(&declaration.symbol));
            if !name_key.starts_with(&indexed_prefix) {
                continue;
            }
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
                let block = declaration
                    .span
                    .range
                    .start
                    .try_into()
                    .ok()
                    .and_then(|offset| {
                        root.token_at_offset(offset)
                            .right_biased()?
                            .parent()?
                            .ancestors()
                            .find(|node| {
                                let at = span(file, node);
                                node.kind() == SyntaxKind::Block
                                    && at.range.start <= declaration.span.range.start
                                    && declaration.span.range.end <= at.range.end
                            })
                    });
                if block.is_some_and(|node| {
                    let at = span(file, &node);
                    byte < at.range.start || byte > at.range.end
                }) {
                    continue;
                }
            }
            names.insert(
                name_key,
                matches_prefix(symbol_name(&declaration.symbol), prefix).then(|| {
                    crate::CompletionCandidate {
                        symbol: declaration.symbol.clone(),
                        ty: declaration.ty.clone(),
                        definition: Some(declaration.span),
                    }
                }),
            );
        }
    }
    let receiver_name = match receiver {
        Some(Type::Script(name)) => name.as_str(),
        None => &current.name,
        _ => return Ok(Vec::new()),
    };
    let mut lineage = Some(key(receiver_name));
    let mut visited = BTreeSet::new();
    let mut possible = BTreeSet::new();
    while let Some(script_key) = lineage {
        cancellation.check()?;
        if !visited.insert(script_key.clone()) {
            break;
        }
        let Some(script) = world.scripts.get(&script_key) else {
            break;
        };
        for members in [
            &script.members,
            &script.variables,
            &script.callable_overloads,
        ] {
            for (name, _) in prefix_entries(members, &indexed_prefix) {
                cancellation.check()?;
                possible.insert(name.as_str());
            }
        }
        if receiver.is_none() {
            for state in script.states.values() {
                cancellation.check()?;
                for (name, _) in prefix_entries(state, &indexed_prefix) {
                    cancellation.check()?;
                    possible.insert(name.as_str());
                }
            }
        }
        lineage = script.parent.as_ref().map(|parent| key(parent));
    }
    let state = callable
        .as_ref()
        .and_then(|node| enclosing_name(node, SyntaxKind::StateDecl, "state"));
    for member_name in possible {
        cancellation.check()?;
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
        names.entry(member_name.to_owned()).or_insert_with(|| {
            // Excluded spellings still shadow lower-priority names sharing the
            // same Unicode-normalized checker identity.
            matches_prefix(&member.name, prefix).then(|| {
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
                crate::CompletionCandidate {
                    symbol,
                    ty: member.ty.clone(),
                    definition: member.definition,
                }
            })
        });
    }
    if receiver.is_none() {
        for (script_key, script) in prefix_entries(&world.scripts, &indexed_prefix) {
            cancellation.check()?;
            names.entry(script_key.clone()).or_insert_with(|| {
                matches_prefix(&script.name, prefix).then(|| crate::CompletionCandidate {
                    symbol: Symbol::Script(script.name.clone()),
                    ty: Type::Script(script.name.clone()),
                    definition: script.definition,
                })
            });
        }
        let imports = view.declarations(file).unwrap();
        let mut imported: BTreeMap<String, Vec<(&ScriptInfo, &MemberInfo)>> = BTreeMap::new();
        for declaration in imports.iter() {
            cancellation.check()?;
            let Declaration::Import { name: import } = declaration else {
                continue;
            };
            let mut lineage = Some(key(import));
            let mut visited = BTreeSet::new();
            let mut member_names = BTreeSet::new();
            while let Some(script_key) = lineage {
                cancellation.check()?;
                if !visited.insert(script_key.clone()) {
                    break;
                }
                let Some(script) = world.scripts.get(&script_key) else {
                    break;
                };
                for (name, _) in prefix_entries(&script.members, &indexed_prefix) {
                    cancellation.check()?;
                    member_names.insert(name.as_str());
                }
                lineage = script.parent.as_ref().map(|parent| key(parent));
            }
            for member_name in member_names {
                cancellation.check()?;
                if let Some((owner, member)) = lookup_member(world, import, &member_name)
                    && member.global
                {
                    imported
                        .entry(member_name.to_owned())
                        .or_default()
                        .push((owner, member));
                }
            }
        }
        for (name_key, items) in imported {
            cancellation.check()?;
            if let [(owner, member)] = items.as_slice() {
                names.entry(name_key).or_insert_with(|| {
                    matches_prefix(&member.name, prefix).then(|| crate::CompletionCandidate {
                        symbol: Symbol::Member {
                            script: owner.name.clone(),
                            name: member.name.clone(),
                        },
                        ty: member.ty.clone(),
                        definition: member.definition,
                    })
                });
            }
        }
    }
    if !global && !context_global {
        for candidate in
            crate::intrinsics::intrinsic_candidates(&Type::Script(receiver_name.into()), prefix)
        {
            names
                .entry(key(symbol_name(&candidate.symbol)))
                .or_insert(Some(candidate));
        }
    }
    checkpoint(cancelled)?;
    Ok(names.into_values().flatten().collect())
}

/// Normalized keys place every prefix match in one contiguous ordered range.
fn prefix_entries<'a, T>(
    entries: &'a BTreeMap<String, T>,
    prefix: &'a str,
) -> impl Iterator<Item = (&'a String, &'a T)> {
    use std::ops::Bound::{Included, Unbounded};
    entries
        .range::<str, _>((Included(prefix), Unbounded))
        .take_while(move |(name, _)| name.starts_with(prefix))
}

fn matches_prefix(name: &str, prefix: &str) -> bool {
    name.get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

fn checkpoint(cancelled: &dyn Fn() -> bool) -> Result<(), AnalysisCancelled> {
    if cancelled() {
        Err(AnalysisCancelled)
    } else {
        Ok(())
    }
}

/// Check the adapter's cancellation predicate once per batch of inspected entries.
struct Cancellation<'a> {
    cancelled: &'a dyn Fn() -> bool,
    remaining: u8,
}

impl<'a> Cancellation<'a> {
    fn new(cancelled: &'a dyn Fn() -> bool) -> Self {
        Self {
            cancelled,
            remaining: 0,
        }
    }

    fn check(&mut self) -> Result<(), AnalysisCancelled> {
        if self.remaining == 0 {
            checkpoint(self.cancelled)?;
            self.remaining = 63;
        } else {
            self.remaining -= 1;
        }
        Ok(())
    }
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

#[cfg(test)]
mod tests;
