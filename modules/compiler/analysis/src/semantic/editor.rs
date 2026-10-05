//! Prefix selection uses indexed checker names before candidate materialization.
use super::{
    AnalysisCancelled, AnalysisView, AnalyzedFile, BTreeMap, BTreeSet, Declaration, FileId,
    HirMemberKind, MemberFact, MemberInfo, ScriptInfo, Symbol, SyntaxKind, SyntaxNode, Type, World,
    enclosing_name, key, lookup_member, lookup_state_member, span,
};

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
    let Some(parse) = view.parse(file) else {
        return Ok(Vec::new());
    };
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
    let mut collector = CandidateCollector::new(world, prefix, cancelled);
    if let Some(receiver @ Type::Array(_)) = receiver {
        for candidate in crate::intrinsics::intrinsic_candidates(receiver, prefix) {
            collector
                .names
                .insert(key(symbol_name(&candidate.symbol)), Some(candidate));
        }
        checkpoint(cancelled)?;
        return Ok(collector.names.into_values().flatten().collect());
    }
    if receiver.is_none() {
        collector.collect_locals(file, facts, &root, owner, byte)?;
    }
    let receiver_name = match receiver {
        Some(Type::Script(name)) => name.as_str(),
        None => &current.name,
        _ => return Ok(Vec::new()),
    };
    let state = callable
        .as_ref()
        .and_then(|node| enclosing_name(node, SyntaxKind::StateDecl, "state"));
    collector.collect_members(
        receiver_name,
        state.as_deref(),
        receiver,
        global,
        context_global,
    )?;
    if receiver.is_none() {
        let imports = view
            .declarations(file)
            .expect("semantic source files retain declaration inputs");
        collector.collect_globals(&imports)?;
    }
    if !global && !context_global {
        for candidate in
            crate::intrinsics::intrinsic_candidates(&Type::Script(receiver_name.into()), prefix)
        {
            collector
                .names
                .entry(key(symbol_name(&candidate.symbol)))
                .or_insert(Some(candidate));
        }
    }
    checkpoint(cancelled)?;
    Ok(collector.names.into_values().flatten().collect())
}

/// Candidate identity and cancellation are shared across completion lookup priorities.
struct CandidateCollector<'a> {
    world: &'a World,
    prefix: &'a str,
    indexed_prefix: String,
    names: BTreeMap<String, Option<crate::CompletionCandidate>>,
    cancellation: Cancellation<'a>,
}

impl<'a> CandidateCollector<'a> {
    fn new(world: &'a World, prefix: &'a str, cancelled: &'a dyn Fn() -> bool) -> Self {
        Self {
            world,
            prefix,
            indexed_prefix: key(prefix),
            names: BTreeMap::new(),
            cancellation: Cancellation::new(cancelled),
        }
    }

    /// Collect locals using the shared shadowing map and cancellation batches.
    fn collect_locals(
        &mut self,
        file: FileId,
        facts: &AnalyzedFile,
        root: &SyntaxNode,
        owner: Option<&MemberFact>,
        byte: usize,
    ) -> Result<(), AnalysisCancelled> {
        for declaration in &facts.script.declarations {
            self.cancellation.check()?;
            let name_key = key(symbol_name(&declaration.symbol));
            if !name_key.starts_with(&self.indexed_prefix) {
                continue;
            }
            let (Symbol::Local {
                owner: local_owner, ..
            }
            | Symbol::Parameter {
                owner: local_owner, ..
            }) = &declaration.symbol
            else {
                continue;
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
            self.names.insert(
                name_key,
                matches_prefix(symbol_name(&declaration.symbol), self.prefix).then(|| {
                    crate::CompletionCandidate {
                        symbol: declaration.symbol.clone(),
                        ty: declaration.ty.clone(),
                        definition: Some(declaration.span),
                    }
                }),
            );
        }
        Ok(())
    }

    /// Collect members using the shared shadowing map and cancellation batches.
    fn collect_members(
        &mut self,
        receiver_name: &str,
        state: Option<&str>,
        receiver: Option<&Type>,
        global: bool,
        context_global: bool,
    ) -> Result<(), AnalysisCancelled> {
        let mut lineage = Some(key(receiver_name));
        let mut visited = BTreeSet::new();
        let mut possible = BTreeSet::new();
        while let Some(script_key) = lineage {
            self.cancellation.check()?;
            if !visited.insert(script_key.clone()) {
                break;
            }
            let Some(script) = self.world.scripts.get(&script_key) else {
                break;
            };
            for members in [
                &script.members,
                &script.variables,
                &script.callable_overloads,
            ] {
                for (name, _) in prefix_entries(members, &self.indexed_prefix) {
                    self.cancellation.check()?;
                    possible.insert(name.as_str());
                }
            }
            if receiver.is_none() {
                for state in script.states.values() {
                    self.cancellation.check()?;
                    for (name, _) in prefix_entries(state, &self.indexed_prefix) {
                        self.cancellation.check()?;
                        possible.insert(name.as_str());
                    }
                }
            }
            lineage = script.parent.as_ref().map(|parent| key(parent));
        }
        for member_name in possible {
            self.cancellation.check()?;
            let selected = state
                .as_ref()
                .filter(|_| receiver.is_none())
                .and_then(|state| {
                    lookup_state_member(self.world, receiver_name, state, member_name)
                });
            let is_state = selected.is_some();
            let Some((script, member)) =
                selected.or_else(|| lookup_member(self.world, receiver_name, member_name))
            else {
                continue;
            };
            if (global || (receiver.is_none() && context_global)) && !member.global {
                continue;
            }
            if receiver.is_some() && !global && member.global {
                continue;
            }
            self.names.entry(member_name.to_owned()).or_insert_with(|| {
                // Excluded spellings still shadow lower-priority names sharing the
                // same Unicode-normalized checker identity.
                matches_prefix(&member.name, self.prefix).then(|| {
                    let symbol = if is_state {
                        Symbol::StateMember {
                            script: script.name.clone(),
                            state: state
                                .expect("state member selection requires an enclosing state")
                                .to_owned(),
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
        Ok(())
    }

    /// Collect globals using the shared shadowing map and cancellation batches.
    fn collect_globals(&mut self, imports: &[Declaration]) -> Result<(), AnalysisCancelled> {
        for (script_key, script) in prefix_entries(&self.world.scripts, &self.indexed_prefix) {
            self.cancellation.check()?;
            self.names.entry(script_key.clone()).or_insert_with(|| {
                matches_prefix(&script.name, self.prefix).then(|| crate::CompletionCandidate {
                    symbol: Symbol::Script(script.name.clone()),
                    ty: Type::Script(script.name.clone()),
                    definition: script.definition,
                })
            });
        }
        let mut imported: BTreeMap<String, Vec<(&ScriptInfo, &MemberInfo)>> = BTreeMap::new();
        for declaration in imports {
            self.cancellation.check()?;
            let Declaration::Import { name: import } = declaration else {
                continue;
            };
            let mut lineage = Some(key(import));
            let mut visited = BTreeSet::new();
            let mut member_names = BTreeSet::new();
            while let Some(script_key) = lineage {
                self.cancellation.check()?;
                if !visited.insert(script_key.clone()) {
                    break;
                }
                let Some(script) = self.world.scripts.get(&script_key) else {
                    break;
                };
                for (name, _) in prefix_entries(&script.members, &self.indexed_prefix) {
                    self.cancellation.check()?;
                    member_names.insert(name.as_str());
                }
                lineage = script.parent.as_ref().map(|parent| key(parent));
            }
            for member_name in member_names {
                self.cancellation.check()?;
                if let Some((owner, member)) = lookup_member(self.world, import, member_name)
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
            self.cancellation.check()?;
            if let [(owner, member)] = items.as_slice() {
                self.names.entry(name_key).or_insert_with(|| {
                    matches_prefix(&member.name, self.prefix).then(|| crate::CompletionCandidate {
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
        Ok(())
    }
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
