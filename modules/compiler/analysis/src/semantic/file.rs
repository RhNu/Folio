//! Validation and typed body extraction for one source file.
use super::*;

pub(super) fn analyze_file(
    view: &AnalysisView,
    world: &World,
    file: FileId,
    script_key: &str,
    root: &SyntaxNode,
    result: &mut FileAnalysis,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), AnalysisCancelled> {
    let Some(script) = world.scripts.get(script_key) else {
        return Ok(());
    };
    let user_flags = view
        .user_flags
        .iter()
        .map(|flag| key(flag))
        .collect::<BTreeSet<_>>();
    let mut imports = Vec::new();
    if let Some(declarations) = view.located_declarations(file) {
        for item in declarations.iter() {
            if cancelled() {
                return Err(AnalysisCancelled);
            }
            let node = source_declaration_node(view, file, item.range);
            match &item.declaration {
                Declaration::Import { name } => {
                    if !world.scripts.contains_key(&key(name)) {
                        result.diagnostics.push(diagnostic(
                            "semantic.unknown-import",
                            format!("unknown import {name}"),
                            SourceSpan {
                                file,
                                range: item.range,
                            },
                        ));
                    } else {
                        imports.push(name.clone());
                    }
                }
                Declaration::Script { flags, .. } => validate_flags(
                    flags,
                    &["hidden", "conditional"],
                    &user_flags,
                    node.as_ref(),
                    file,
                    &mut result.diagnostics,
                ),
                Declaration::Variable { flags, .. } => validate_flags(
                    flags,
                    &["conditional"],
                    &user_flags,
                    node.as_ref(),
                    file,
                    &mut result.diagnostics,
                ),
                Declaration::Property { flags, .. } => validate_flags(
                    flags,
                    &["auto", "autoreadonly", "conditional", "hidden"],
                    &user_flags,
                    node.as_ref(),
                    file,
                    &mut result.diagnostics,
                ),
                Declaration::State { flags, .. } => validate_flags(
                    flags,
                    &["auto"],
                    &user_flags,
                    node.as_ref(),
                    file,
                    &mut result.diagnostics,
                ),
                Declaration::Function { modifiers, .. } | Declaration::Event { modifiers, .. } => {
                    validate_flags(
                        modifiers,
                        &["global", "native", "hidden"],
                        &user_flags,
                        node.as_ref(),
                        file,
                        &mut result.diagnostics,
                    )
                }
            }
        }
    }
    for node in root.descendants().filter(|node| {
        matches!(
            node.kind(),
            SyntaxKind::FunctionDecl | SyntaxKind::EventDecl
        )
    }) {
        if cancelled() {
            return Err(AnalysisCancelled);
        }
        let Some(name) = callable_name(&node) else {
            continue;
        };
        let state = enclosing_name(&node, SyntaxKind::StateDecl, "state");
        let property = enclosing_name(&node, SyntaxKind::PropertyDecl, "property");
        let member = if let Some(property) = &property {
            let property_type = script
                .members
                .get(&key(property))
                .map(|member| member.ty.clone())
                .unwrap_or(Type::Error);
            let ty = if name.eq_ignore_ascii_case("get") {
                property_type.clone()
            } else {
                Type::Void
            };
            if !name.eq_ignore_ascii_case("get") && !name.eq_ignore_ascii_case("set") {
                result.diagnostics.push(diagnostic(
                    "semantic.invalid-accessor",
                    "property accessor must be Get or Set",
                    span(file, &node),
                ));
            }
            let actual = folio_papyrus::FunctionAst::cast(node.clone());
            let actual_ty = actual
                .as_ref()
                .and_then(|ast| ast.return_type())
                .map(|text| Type::from_spelling(&text))
                .unwrap_or(Type::Void);
            if actual_ty != ty {
                result.diagnostics.push(diagnostic(
                    "semantic.accessor-type",
                    format!("accessor must return {ty:?}"),
                    span(file, &node),
                ));
            }
            let parameters: Vec<(String, Type, Option<String>)> = actual
                .map(|ast| {
                    ast.parameters()
                        .into_iter()
                        .map(|parameter| {
                            (
                                parameter.name,
                                Type::from_spelling(&parameter.ty),
                                parameter.default.clone(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            if (name.eq_ignore_ascii_case("get") && !parameters.is_empty())
                || (name.eq_ignore_ascii_case("set")
                    && (parameters.len() != 1 || parameters[0].1 != property_type))
            {
                result.diagnostics.push(diagnostic(
                    "semantic.accessor-parameters",
                    "property accessor parameters do not match property type",
                    span(file, &node),
                ));
            }
            Some(MemberInfo {
                name: name.clone(),
                ty,
                kind: MemberKind::Function,
                parameters,
                unknown_defaults: false,
                global: false,
                auto: false,
                read_only: false,
                writable: true,
                definition: Some(span(file, &node)),
            })
        } else if let Some(state) = &state {
            script
                .states
                .get(&key(state))
                .and_then(|members| members.get(&key(&name)))
                .cloned()
        } else {
            lookup_member(world, &script.name, &name).map(|(_, member)| member.clone())
        };
        let Some(member) = member else { continue };
        let symbol = if let Some(property) = property {
            Symbol::PropertyAccessor {
                script: script.name.clone(),
                property,
                name: name.clone(),
            }
        } else if let Some(state) = &state {
            Symbol::StateMember {
                script: script.name.clone(),
                state: state.clone(),
                name: name.clone(),
            }
        } else {
            Symbol::Member {
                script: script.name.clone(),
                name: name.clone(),
            }
        };
        let mut scope = Scope {
            world,
            file,
            script,
            member: &member,
            callable: symbol.clone(),
            state,
            imports: &imports,
            locals: BTreeMap::new(),
            parameters: Vec::new(),
            result,
            fill_missing_arguments: view.fill_missing_arguments,
            cancelled,
            interrupted: false,
        };
        scope.collect_parameters(&node);
        let statements = node
            .children()
            .find(|child| child.kind() == SyntaxKind::Block)
            .map(|block| scope.block(&block))
            .unwrap_or_default();
        if scope.interrupted || cancelled() {
            return Err(AnalysisCancelled);
        }
        scope.result.script.bodies.push(Body {
            symbol,
            return_type: member.ty.clone(),
            parameters: scope.parameters,
            statements,
        });
    }
    Ok(())
}

fn callable_name(node: &SyntaxNode) -> Option<String> {
    let keyword = if node.kind() == SyntaxKind::EventDecl {
        "event"
    } else {
        "function"
    };
    let mut saw_keyword = false;
    for token in node
        .children_with_tokens()
        .take_while(|item| item.kind() != SyntaxKind::ParameterList)
        .filter_map(|item| item.into_token())
    {
        if token.kind() != SyntaxKind::Ident {
            continue;
        }
        if saw_keyword {
            return Some(token.text().to_string());
        }
        saw_keyword = token.text().eq_ignore_ascii_case(keyword);
    }
    None
}

fn validate_flags(
    flags: &[String],
    builtins: &[&str],
    user_flags: &BTreeSet<String>,
    node: Option<&SyntaxNode>,
    file: FileId,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for flag in flags {
        if builtins
            .iter()
            .any(|builtin| builtin.eq_ignore_ascii_case(flag))
            || user_flags.contains(&key(flag))
        {
            continue;
        }
        let at = node
            .and_then(|node| {
                node.children_with_tokens()
                    .filter_map(|item| item.into_token())
                    .find(|token| {
                        token.kind() == SyntaxKind::Ident && token.text().eq_ignore_ascii_case(flag)
                    })
            })
            .map(|token| token_span(file, &token))
            .or_else(|| node.map(|node| span(file, node)));
        if let Some(at) = at {
            diagnostics.push(diagnostic(
                "semantic.unknown-flag",
                format!("unknown flag {flag}"),
                at,
            ));
        }
    }
}
