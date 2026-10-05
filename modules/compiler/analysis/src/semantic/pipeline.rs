//! Ordered declaration collection and export phases for coherent semantic views.
use super::{
    AnalysisCancelled, AnalysisView, Arc, BTreeMap, Declaration, DeclarationFact, Diagnostic,
    FileAnalysis, FileId, HashSet, HirMemberKind, MemberFact, MemberInfo, MemberKind, NameRef,
    ParameterFact, RelatedLocation, Script, ScriptInfo, Severity, SourceSpan, StateFact, Symbol,
    SyntaxKind, SyntaxNode, TextRange, Type, WorkingAnalysis, World, diagnostic, direct_expression,
    enclosing_name, key, member_from_source, name_token, source_declaration_node, token_span,
};

/// Collect source scripts before the next analysis phase.
pub(super) fn collect_source_scripts(
    view: &AnalysisView,
    cancelled: &dyn Fn() -> bool,
    analysis: &mut WorkingAnalysis,
    world: &mut World,
    file_scripts: &mut BTreeMap<FileId, String>,
) -> Result<(), AnalysisCancelled> {
    for file in view.file_ids() {
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        let mut file_analysis = FileAnalysis {
            script: Script::default(),
            diagnostics: Vec::new(),
        };
        if let Some(parse) = view.parse(file) {
            for error in &parse.errors {
                file_analysis.diagnostics.push(diagnostic(
                    "papyrus.syntax",
                    error.message.clone(),
                    SourceSpan {
                        file,
                        range: error.range,
                    },
                ));
            }
        }
        if let Some(located) = view.located_declarations(file) {
            for item in located.iter() {
                if let Declaration::Script {
                    name,
                    parent,
                    flags,
                } = &item.declaration
                {
                    file_analysis.script.flags.clone_from(flags);
                    let node = source_declaration_node(view, file, item.range);
                    let name_span = node
                        .as_ref()
                        .and_then(|node| name_token(node, &["scriptname"]))
                        .map_or(
                            SourceSpan {
                                file,
                                range: item.range,
                            },
                            |token| token_span(file, &token),
                        );
                    let name_ref = NameRef {
                        text: name.clone(),
                        span: name_span,
                    };
                    if let Some(parent) = parent {
                        let parent_span = node
                            .as_ref()
                            .and_then(|node| {
                                node.children_with_tokens()
                                    .filter_map(rowan::NodeOrToken::into_token)
                                    .filter(|token| token.kind() == SyntaxKind::Ident)
                                    .find(|token| token.text().eq_ignore_ascii_case(parent))
                            })
                            .map_or(name_span, |token| token_span(file, &token));
                        file_analysis.script.parent = Some(NameRef {
                            text: parent.clone(),
                            span: parent_span,
                        });
                    }
                    file_analysis.script.name = Some(name_ref);
                    if let Some(previous) = world.scripts.get(&key(name)) {
                        let mut error = diagnostic(
                            "semantic.duplicate-script",
                            format!("duplicate script {name}"),
                            name_span,
                        );
                        if let Some(location) = previous.definition {
                            error.related.push(RelatedLocation {
                                span: location,
                                message: "previous script".into(),
                            });
                        }
                        file_analysis.diagnostics.push(error);
                    } else {
                        file_scripts.insert(file, key(name));
                        world.scripts.insert(
                            key(name),
                            Arc::new(ScriptInfo {
                                name: name.clone(),
                                parent: parent.clone(),
                                definition: Some(name_span),
                                members: BTreeMap::new(),
                                variables: BTreeMap::new(),
                                callable_overloads: BTreeMap::new(),
                                states: BTreeMap::new(),
                            }),
                        );
                    }
                    break;
                }
            }
        }
        analysis.files.insert(file, file_analysis);
    }
    Ok(())
}

/// Collect external scripts before the next analysis phase.
pub(super) fn collect_external_scripts(
    view: &AnalysisView,
    cancelled: &dyn Fn() -> bool,
    analysis: &mut WorkingAnalysis,
    world: &mut World,
) -> Result<(), AnalysisCancelled> {
    for external in &view.external_declarations.entries {
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        let k = key(&external.info.name);
        if world.scripts.contains_key(&k) {
            analysis.project_diagnostics.push(Diagnostic::new(
                "semantic.duplicate-script",
                Severity::Error,
                format!(
                    "external script {} conflicts with selected source",
                    external.info.name
                ),
            ));
            continue;
        }
        analysis
            .project_diagnostics
            .extend(external.diagnostics.iter().cloned());
        world.scripts.insert(k, Arc::clone(&external.info));
    }
    Ok(())
}

/// Collect source members before the next analysis phase.
pub(super) fn collect_source_members(
    view: &AnalysisView,
    cancelled: &dyn Fn() -> bool,
    analysis: &mut WorkingAnalysis,
    world: &mut World,
    file_scripts: &BTreeMap<FileId, String>,
) -> Result<(), AnalysisCancelled> {
    // Source member signatures are collected before bodies to support mutual calls.
    for (&file, script_key) in file_scripts {
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        let Some(located) = view.located_declarations(file) else {
            continue;
        };
        let file_analysis = analysis
            .files
            .get_mut(&file)
            .expect("file_scripts only contains initialized source files");
        for item in located.iter() {
            if cancelled() {
                return Err(AnalysisCancelled);
            }
            if matches!(
                item.declaration,
                Declaration::Script { .. } | Declaration::Import { .. } | Declaration::State { .. }
            ) {
                if let Declaration::State { name, flags } = &item.declaration {
                    file_analysis.script.states.push(StateFact {
                        name: name.clone(),
                        auto: flags.iter().any(|flag| flag.eq_ignore_ascii_case("auto")),
                        span: SourceSpan {
                            file,
                            range: item.range,
                        },
                    });
                }
                continue;
            }
            let Some(node) = source_declaration_node(view, file, item.range) else {
                continue;
            };
            if enclosing_name(&node, SyntaxKind::PropertyDecl, "property").is_some() {
                continue;
            }
            if let Some(member) = member_from_source(file, &node, &item.declaration) {
                let script = world
                    .scripts
                    .get_mut(script_key)
                    .expect("source script exists");
                let script = Arc::make_mut(script);
                let member_key = key(&member.name);
                let state = enclosing_name(&node, SyntaxKind::StateDecl, "state");
                let table = if let Some(state) = &state {
                    script.states.entry(key(state)).or_default()
                } else {
                    &mut script.members
                };
                if let Some(previous) = table.get(&member_key) {
                    let mut error = diagnostic(
                        "semantic.duplicate-member",
                        format!("duplicate member {}", member.name),
                        member
                            .definition
                            .expect("source members retain declaration spans"),
                    );
                    if let Some(location) = previous.definition {
                        error.related.push(RelatedLocation {
                            span: location,
                            message: "previous member".into(),
                        });
                    }
                    file_analysis.diagnostics.push(error);
                    continue;
                }
                table.insert(member_key, member.clone());
                file_analysis.script.declarations.push(DeclarationFact {
                    symbol: if let Some(state) = state {
                        Symbol::StateMember {
                            script: script.name.clone(),
                            state,
                            name: member.name.clone(),
                        }
                    } else {
                        Symbol::Member {
                            script: script.name.clone(),
                            name: member.name.clone(),
                        }
                    },
                    ty: member.ty.clone(),
                    span: member
                        .definition
                        .expect("source members retain declaration spans"),
                });
                file_analysis.script.members.push(source_member_fact(
                    &node,
                    &item.declaration,
                    &script.name,
                    &member,
                ));
            }
        }
    }
    Ok(())
}

/// Preserve the source member shape and origin independently of signature indexing.
fn source_member_fact(
    node: &SyntaxNode,
    declaration: &Declaration,
    script_name: &str,
    member: &MemberInfo,
) -> MemberFact {
    let source_symbol = if let Some(state) = enclosing_name(node, SyntaxKind::StateDecl, "state") {
        Symbol::StateMember {
            script: script_name.to_owned(),
            state,
            name: member.name.clone(),
        }
    } else {
        Symbol::Member {
            script: script_name.to_owned(),
            name: member.name.clone(),
        }
    };
    let (kind, flags, parameters) = source_member_shape(
        declaration,
        member
            .definition
            .expect("source members retain declaration spans"),
    );
    let initial_literal =
        direct_expression(node).and_then(|expr| folio_papyrus::constant_literal_text(&expr));
    MemberFact {
        symbol: source_symbol,
        kind,
        ty: member.ty.clone(),
        parameters,
        flags,
        initial_literal,
        span: member
            .definition
            .expect("source members retain declaration spans"),
    }
}

/// Export inherited members before the next analysis phase.
pub(super) fn export_inherited_members(
    analysis: &mut WorkingAnalysis,
    world: &World,
    file_scripts: &BTreeMap<FileId, String>,
) {
    // Export the resolved ancestor layout once, so lowering never reopens source or external declarations.
    for (&file, script_key) in file_scripts {
        let mut inherited = Vec::new();
        let mut seen = HashSet::new();
        let mut uncertain_overrides = HashSet::new();
        let mut override_diagnostics = Vec::new();
        let mut current = world
            .scripts
            .get(script_key)
            .and_then(|script| script.parent.clone());
        while let Some(parent_name) = current.as_ref() {
            let parent_key = key(parent_name);
            if !seen.insert(parent_key.clone()) {
                break;
            }
            let Some(parent) = world.scripts.get(&parent_key) else {
                break;
            };
            for member in &analysis.files[&file].script.members {
                if !matches!(member.kind, HirMemberKind::Function { .. }) {
                    continue;
                }
                let (identity, inherited_member) = match &member.symbol {
                    Symbol::Member { name, .. } => (
                        key(name),
                        parent
                            .members
                            .get(&key(name))
                            .or_else(|| parent.callable_overloads.get(&key(name))),
                    ),
                    Symbol::StateMember { state, name, .. } => (
                        format!("{}:{}", key(state), key(name)),
                        parent
                            .states
                            .get(&key(state))
                            .and_then(|members| members.get(&key(name))),
                    ),
                    _ => continue,
                };
                if inherited_member.is_some_and(|item| item.kind == MemberKind::UnknownCallable)
                    && uncertain_overrides.insert(identity)
                {
                    override_diagnostics.push(
                        Diagnostic::new(
                            "semantic.override-ambiguous",
                            Severity::Error,
                            "cannot override a declaration whose event/function kind is unknown",
                        )
                        .at(member.span),
                    );
                }
            }
            if let Some((&parent_file, _)) =
                file_scripts.iter().find(|(_, key)| **key == parent_key)
            {
                inherited.extend(analysis.files[&parent_file].script.members.iter().cloned());
            } else {
                let at = analysis.files[&file].script.parent.as_ref().map_or(
                    SourceSpan {
                        file,
                        range: TextRange { start: 0, end: 0 },
                    },
                    |name| name.span,
                );
                inherited.extend(
                    parent
                        .members
                        .values()
                        .chain(parent.variables.values())
                        .chain(parent.callable_overloads.values())
                        .map(|member| {
                            external_member_fact(
                                member,
                                Symbol::Member {
                                    script: parent.name.clone(),
                                    name: member.name.clone(),
                                },
                                at,
                            )
                        }),
                );
            }
            current.clone_from(&parent.parent);
        }
        analysis
            .files
            .get_mut(&file)
            .expect("file_scripts only contains initialized source files")
            .script
            .external_members = inherited;
        analysis
            .files
            .get_mut(&file)
            .expect("file_scripts only contains initialized source files")
            .diagnostics
            .extend(override_diagnostics);
    }
}

/// Export referenced members before the next analysis phase.
pub(super) fn export_referenced_members(
    analysis: &mut WorkingAnalysis,
    world: &World,
    file_scripts: &BTreeMap<FileId, String>,
) {
    // Preserve the declaration kind of cross-script uses for lowering. A typed
    // member reference alone cannot distinguish a field from a property/function.
    for &file in file_scripts.keys() {
        let refs = analysis.files[&file]
            .script
            .expressions
            .iter()
            .filter_map(|expression| {
                expression
                    .binding
                    .as_ref()
                    .map(|binding| (binding.symbol.clone(), expression.span))
            })
            .collect::<Vec<_>>();
        let mut seen = HashSet::new();
        let mut referenced = Vec::new();
        for (symbol, at) in refs {
            if !seen.insert(symbol.clone())
                || analysis.files[&file]
                    .script
                    .members
                    .iter()
                    .chain(&analysis.files[&file].script.external_members)
                    .any(|member| member.symbol == symbol)
            {
                continue;
            }
            let (Symbol::Member {
                script: script_name,
                name: member_name,
            }
            | Symbol::StateMember {
                script: script_name,
                name: member_name,
                ..
            }) = &symbol
            else {
                continue;
            };
            let fact = file_scripts
                .iter()
                .find(|(_, candidate)| *candidate == &key(script_name))
                .and_then(|(owner_file, _)| {
                    analysis.files[owner_file]
                        .script
                        .members
                        .iter()
                        .find(|member| member.symbol == symbol)
                        .cloned()
                })
                .or_else(|| {
                    world
                        .scripts
                        .get(&key(script_name))
                        .and_then(|script| match &symbol {
                            Symbol::StateMember { state, .. } => script
                                .states
                                .get(&key(state))
                                .and_then(|members| members.get(&key(member_name))),
                            _ => script
                                .members
                                .get(&key(member_name))
                                .or_else(|| script.variables.get(&key(member_name)))
                                .or_else(|| script.callable_overloads.get(&key(member_name))),
                        })
                        .map(|member| external_member_fact(member, symbol.clone(), at))
                });
            if let Some(fact) = fact {
                referenced.push(fact);
            }
        }
        analysis
            .files
            .get_mut(&file)
            .expect("file_scripts only contains initialized source files")
            .script
            .referenced_members = referenced;
    }
}

/// Convert source declaration metadata into the shared HIR member shape.
fn source_member_shape(
    declaration: &Declaration,
    at: SourceSpan,
) -> (HirMemberKind, Vec<String>, Vec<ParameterFact>) {
    match declaration {
        Declaration::Variable { flags, .. } => (HirMemberKind::Variable, flags.clone(), Vec::new()),
        Declaration::Property { flags, .. } => (
            HirMemberKind::Property {
                auto: flags.iter().any(|flag| {
                    flag.eq_ignore_ascii_case("auto") || flag.eq_ignore_ascii_case("autoreadonly")
                }),
                read_only: flags
                    .iter()
                    .any(|flag| flag.eq_ignore_ascii_case("autoreadonly")),
            },
            flags.clone(),
            Vec::new(),
        ),
        Declaration::Function {
            parameters,
            modifiers,
            ..
        } => (
            HirMemberKind::Function {
                event: false,
                global: modifiers
                    .iter()
                    .any(|modifier| modifier.eq_ignore_ascii_case("global")),
                native: modifiers
                    .iter()
                    .any(|modifier| modifier.eq_ignore_ascii_case("native")),
            },
            modifiers.clone(),
            parameters
                .iter()
                .map(|parameter| ParameterFact {
                    name: parameter.name.clone(),
                    ty: Type::from_spelling(&parameter.ty),
                    default_literal: parameter.default.clone(),
                    span: at,
                })
                .collect(),
        ),
        Declaration::Event {
            parameters,
            modifiers,
            ..
        } => (
            HirMemberKind::Function {
                event: true,
                global: false,
                native: modifiers
                    .iter()
                    .any(|modifier| modifier.eq_ignore_ascii_case("native")),
            },
            modifiers.clone(),
            parameters
                .iter()
                .map(|parameter| ParameterFact {
                    name: parameter.name.clone(),
                    ty: Type::from_spelling(&parameter.ty),
                    default_literal: parameter.default.clone(),
                    span: at,
                })
                .collect(),
        ),
        _ => unreachable!(),
    }
}

/// Project external declaration facts without reopening API source data.
fn external_member_fact(member: &super::MemberInfo, symbol: Symbol, at: SourceSpan) -> MemberFact {
    MemberFact {
        symbol,
        kind: match member.kind {
            MemberKind::Variable => HirMemberKind::Variable,
            MemberKind::Property => HirMemberKind::Property {
                auto: member.property.auto,
                read_only: member.property.read_only,
            },
            MemberKind::Function | MemberKind::Event | MemberKind::UnknownCallable => {
                HirMemberKind::Function {
                    event: member.kind == MemberKind::Event,
                    global: member.global,
                    native: true,
                }
            },
        },
        ty: member.ty.clone(),
        parameters: member
            .parameters
            .iter()
            .map(|(name, ty, default)| ParameterFact {
                name: name.clone(),
                ty: ty.clone(),
                default_literal: default.literal().map(str::to_owned),
                span: at,
            })
            .collect(),
        flags: Vec::new(),
        initial_literal: None,
        span: at,
    }
}
