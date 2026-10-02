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
    if let Some(parse) = view.parse(file) {
        for issue in folio_papyrus::validate_declarations(&parse, Some(view.user_flags.as_ref())) {
            result.diagnostics.push(diagnostic(
                issue.code,
                issue.message,
                SourceSpan {
                    file,
                    range: issue.range,
                },
            ));
        }
    }
    let mut imports = Vec::new();
    if let Some(declarations) = view.located_declarations(file) {
        for item in declarations.iter() {
            if cancelled() {
                return Err(AnalysisCancelled);
            }
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
                _ => {}
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
            let actual = folio_papyrus::FunctionAst::cast(node.clone());
            let parameters: Vec<(String, Type, ParameterDefault)> = actual
                .map(|ast| {
                    ast.parameters()
                        .into_iter()
                        .map(|parameter| {
                            (
                                parameter.name,
                                Type::from_spelling(&parameter.ty),
                                parameter
                                    .default
                                    .clone()
                                    .map(ParameterDefault::Literal)
                                    .unwrap_or(ParameterDefault::Required),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(MemberInfo {
                name: name.clone(),
                ty,
                kind: MemberKind::Function,
                parameters,
                global: false,
                auto: false,
                read_only: false,
                readable: true,
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

pub(super) fn callable_name(node: &SyntaxNode) -> Option<String> {
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
