//! Source and portable declarations share one presentation and virtual source format.
use crate::{Hover, navigation::same, symbols};
use folio_build::ProjectAnalysisView;
use folio_format_declarations::{Member, MemberData, PropertyAccess};
use folio_hir::Symbol;
use folio_papyrus::SyntaxKind;
use folio_source::TextRange;

pub fn hover_symbol(view: &ProjectAnalysisView, symbol: &Symbol) -> Option<Hover> {
    if let Symbol::Intrinsic { name } = symbol {
        return intrinsic_hover(name, None);
    }
    hover_at_definition(view, symbol, None)
}

pub(crate) fn intrinsic_hover(name: &str, receiver: Option<&folio_hir::Type>) -> Option<Hover> {
    let signature = folio_analysis::intrinsic_signature(name, receiver)?;
    let parameters = signature
        .parameters
        .iter()
        .map(|(name, ty, default)| {
            format!(
                "{} {}{}",
                crate::display_type(ty),
                name,
                default
                    .as_ref()
                    .map_or(String::new(), |value| format!(" = {value}"))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let declaration = if signature.callable {
        format!(
            "{}Function {}({parameters})",
            if signature.result == folio_hir::Type::Void {
                String::new()
            } else {
                format!("{} ", crate::display_type(&signature.result))
            },
            signature.name
        )
    } else {
        format!(
            "{} Property {}",
            crate::display_type(&signature.result),
            signature.name
        )
    };
    Some(Hover {
        content: declaration.clone(),
        declaration,
        documentation: None,
        details: vec![if signature.callable {
            "Compiler intrinsic".into()
        } else {
            "Read-only array length".into()
        }],
        symbol: Some(Symbol::Intrinsic {
            name: signature.name,
        }),
        span: None,
        owner_script: None,
    })
}

pub(crate) fn intrinsic_call_member(
    callee: &folio_hir::ExpressionFact,
) -> Option<folio_hir::MemberFact> {
    let name = match &callee.binding.as_ref()?.symbol {
        Symbol::Intrinsic { name } => name,
        _ => return None,
    };
    let receiver = match &callee.kind {
        folio_hir::ExpressionKind::Member { owner, .. } => Some(&owner.ty),
        _ => None,
    };
    let signature = folio_analysis::intrinsic_signature(name, receiver)?;
    let span = callee.span;
    Some(folio_hir::MemberFact {
        symbol: Symbol::Intrinsic {
            name: signature.name,
        },
        kind: folio_hir::MemberKind::Function {
            event: false,
            global: false,
            native: false,
        },
        ty: signature.result,
        parameters: signature
            .parameters
            .into_iter()
            .map(|(name, ty, default)| folio_hir::ParameterFact {
                name,
                ty,
                default_literal: default,
                span,
            })
            .collect(),
        flags: Vec::new(),
        initial_literal: None,
        span,
    })
}

pub(crate) fn hover_at_definition(
    view: &ProjectAnalysisView,
    symbol: &Symbol,
    definition: Option<folio_source::SourceSpan>,
) -> Option<Hover> {
    if let Symbol::Intrinsic { name } = symbol {
        return intrinsic_hover(name, None);
    }
    let owner = symbols::owner_script(symbol);
    for file in view.analysis.file_ids() {
        let script = view.analysis.hir(file)?;
        let found = script.declarations.iter().find(|item| {
            same(&item.symbol, symbol) && definition.is_none_or(|span| span == item.span)
        });
        let at = if let Some(item) = found {
            Some(item.span)
        } else {
            script.name.as_ref().filter(|item|matches!(symbol,Symbol::Script(name) if name.eq_ignore_ascii_case(&item.text))).map(|item|item.span)
        };
        if let Some(at) = at {
            let parse = view.analysis.parse(file)?;
            let node = parse
                .syntax()
                .descendants()
                .filter(|node| {
                    matches!(
                        node.kind(),
                        SyntaxKind::ScriptDecl
                            | SyntaxKind::FunctionDecl
                            | SyntaxKind::EventDecl
                            | SyntaxKind::PropertyDecl
                            | SyntaxKind::VariableDecl
                            | SyntaxKind::Parameter
                    )
                })
                .filter(|node| {
                    usize::from(node.text_range().start()) <= at.range.start
                        && at.range.end <= usize::from(node.text_range().end())
                })
                .min_by_key(|node| u32::from(node.text_range().len()))?;
            let declaration = folio_papyrus::declaration_header(&node);
            let documentation = folio_papyrus::declaration_documentation(&node);
            let details = match symbol {
                Symbol::Local { .. } => vec!["Local variable".into()],
                Symbol::Parameter { .. } => vec!["Parameter".into()],
                _ => Vec::new(),
            };
            let content = if let Some(item) = found {
                symbols::describe_symbol(&script, symbol, &item.ty)
            } else {
                declaration.clone()
            };
            return Some(Hover {
                content,
                symbol: Some(symbol.clone()),
                declaration,
                documentation,
                details,
                span: Some(at),
                owner_script: owner,
            });
        }
    }
    let owner_name = owner.as_deref()?;
    let external = view
        .analysis
        .external_declarations()
        .iter()
        .flat_map(|bundle| &bundle.scripts)
        .find(|script| script.name.eq_ignore_ascii_case(owner_name))?;
    let (declaration, documentation) = match symbol {
        Symbol::Script(_) => (script_header(external), external.documentation.clone()),
        Symbol::Member { name, .. } => {
            let member = external
                .members
                .iter()
                .find(|member| member.name.eq_ignore_ascii_case(name))?;
            (member_declaration(member), member.documentation.clone())
        }
        Symbol::StateMember { state, name, .. } => {
            let state = external
                .states
                .iter()
                .find(|item| item.name.eq_ignore_ascii_case(state))?;
            let member = state
                .members
                .iter()
                .find(|member| member.name.eq_ignore_ascii_case(name))?;
            (member_declaration(member), member.documentation.clone())
        }
        _ => return None,
    };
    let mut details = Vec::new();
    if let Some(member) = external_member(view, symbol) {
        if matches!(member.data, MemberData::UnknownCallable { .. }) {
            details.push("Callable kind was not recoverable from PEX".into());
        }
        let unknown = member
            .parameters()
            .iter()
            .filter(|parameter| {
                matches!(
                    parameter.default,
                    folio_format_declarations::ParameterDefault::Unknown
                )
            })
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>();
        if !unknown.is_empty() {
            details.push(format!(
                "Defaults not recoverable from PEX: {}",
                unknown.join(", ")
            ));
        }
    }
    Some(Hover {
        content: declaration.clone(),
        symbol: Some(symbol.clone()),
        declaration,
        documentation,
        details,
        span: None,
        owner_script: owner,
    })
}

pub(crate) fn external_member<'a>(
    view: &'a ProjectAnalysisView,
    symbol: &Symbol,
) -> Option<&'a Member> {
    let owner = symbols::owner_script(symbol)?;
    let script = view
        .analysis
        .external_declarations()
        .iter()
        .flat_map(|bundle| &bundle.scripts)
        .find(|script| script.name.eq_ignore_ascii_case(&owner))?;
    match symbol {
        Symbol::Member { name, .. } => script
            .members
            .iter()
            .find(|member| member.name.eq_ignore_ascii_case(name)),
        Symbol::StateMember { state, name, .. } => script
            .states
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(state))?
            .members
            .iter()
            .find(|member| member.name.eq_ignore_ascii_case(name)),
        _ => None,
    }
}

fn script_header(script: &folio_format_declarations::Script) -> String {
    let mut header = format!("Scriptname {}", script.name);
    if let Some(parent) = &script.parent {
        header.push_str(&format!(" Extends {parent}"));
    }
    if script.is_native {
        header.push_str(" Native");
    }
    for flag in &script.flags {
        if flag.eq_ignore_ascii_case("native") {
            continue;
        }
        header.push(' ');
        header.push_str(flag);
    }
    header
}

fn member_header(member: &Member) -> String {
    let parameters = member
        .parameters()
        .iter()
        .map(|parameter| {
            let default = parameter
                .default
                .literal()
                .map_or(String::new(), |value| format!(" = {value}"));
            format!("{} {}{default}", parameter.ty, parameter.name)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut header = match &member.data {
        MemberData::Function { return_type, .. } => format!(
            "{}Function {}({parameters})",
            return_type
                .as_ref()
                .map_or(String::new(), |ty| format!("{ty} ")),
            member.name
        ),
        MemberData::UnknownCallable { return_type, .. } => format!(
            "{}Callable {}({parameters})",
            return_type
                .as_ref()
                .map_or(String::new(), |ty| format!("{ty} ")),
            member.name
        ),
        MemberData::Event { .. } => format!("Event {}({parameters})", member.name),
        MemberData::Variable {
            ty,
            initial_literal,
        } => format!(
            "{ty} {}{}",
            member.name,
            initial_literal
                .as_ref()
                .map_or(String::new(), |value| format!(" = {value}"))
        ),
        MemberData::Property {
            ty,
            access,
            initial_literal,
        } => {
            let auto = match access {
                PropertyAccess::Auto => " Auto",
                PropertyAccess::AutoReadOnly => " AutoReadOnly",
                PropertyAccess::Manual { .. } => "",
            };
            format!(
                "{ty} Property {}{}{auto}",
                member.name,
                initial_literal
                    .as_ref()
                    .map_or(String::new(), |value| format!(" = {value}"))
            )
        }
    };
    if member.is_global() {
        header.push_str(" Global");
    }
    if member.is_native() {
        header.push_str(" Native");
    }
    for flag in &member.flags {
        if ["global", "native", "auto", "autoreadonly"]
            .iter()
            .any(|word| flag.eq_ignore_ascii_case(word))
        {
            continue;
        }
        header.push(' ');
        header.push_str(flag);
    }
    header
}

fn member_declaration(member: &Member) -> String {
    let mut declaration = member_header(member);
    if let MemberData::Property {
        ty,
        access: PropertyAccess::Manual { readable, writable },
        ..
    } = &member.data
    {
        if *readable {
            declaration.push_str(&format!("\n    {ty} Function Get()\n    EndFunction"));
        }
        if *writable {
            declaration.push_str(&format!("\n    Function Set({ty} value)\n    EndFunction"));
        }
        declaration.push_str("\nEndProperty");
    }
    declaration
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclarationDocument {
    pub text: String,
    pub declarations: Vec<(Symbol, TextRange)>,
}

/// Render only recovered API facts; the virtual document contains no invented bodies.
pub fn declaration_document(
    view: &ProjectAnalysisView,
    script_name: &str,
) -> Option<DeclarationDocument> {
    let script = view
        .analysis
        .external_declarations()
        .iter()
        .flat_map(|bundle| &bundle.scripts)
        .find(|script| script.name.eq_ignore_ascii_case(script_name))?;
    fn doc(text: &mut String, documentation: &Option<String>) {
        if let Some(doc) = documentation {
            text.push_str("{\n");
            text.push_str(doc);
            text.push_str("\n}\n");
        }
    }
    fn member(
        text: &mut String,
        item: &Member,
        symbol: Symbol,
        declarations: &mut Vec<(Symbol, TextRange)>,
    ) {
        let header = member_header(item);
        let prefix = match &item.data {
            MemberData::Variable { ty, .. } => ty.len() + 1,
            MemberData::Property { ty, .. } => ty.len() + 10,
            _ => header.find(&format!(" {}(", item.name)).unwrap() + 1,
        };
        if matches!(item.data, MemberData::UnknownCallable { .. }) {
            text.push_str("; ");
        }
        let start = text.len() + prefix;
        declarations.push((
            symbol,
            TextRange {
                start,
                end: start + item.name.len(),
            },
        ));
        text.push_str(&header);
        text.push('\n');
        if matches!(item.data, MemberData::UnknownCallable { .. }) {
            text.push_str("; Callable kind is unknown in this PEX API.\n");
        }
        for parameter in item.parameters().iter().filter(|parameter| {
            matches!(
                parameter.default,
                folio_format_declarations::ParameterDefault::Unknown
            )
        }) {
            text.push_str(&format!(
                "; Default for {} is unknown in this PEX API.\n",
                parameter.name
            ));
        }
        doc(text, &item.documentation);
        match &item.data {
            MemberData::Function { native: false, .. } => text.push_str("EndFunction\n"),
            MemberData::Event { native: false, .. } => text.push_str("EndEvent\n"),
            MemberData::Property {
                ty,
                access: PropertyAccess::Manual { readable, writable },
                ..
            } => {
                if *readable {
                    text.push_str(&format!("    {ty} Function Get()\n    EndFunction\n"));
                }
                if *writable {
                    text.push_str(&format!("    Function Set({ty} value)\n    EndFunction\n"));
                }
                text.push_str("EndProperty\n");
            }
            _ => {}
        }
        text.push('\n');
    }
    let mut declarations = vec![(
        Symbol::Script(script.name.clone()),
        TextRange {
            start: 11,
            end: 11 + script.name.len(),
        },
    )];
    let mut text = script_header(script);
    text.push('\n');
    doc(&mut text, &script.documentation);
    text.push('\n');
    for import in &script.imports {
        text.push_str(&format!("Import {import}\n"));
    }
    for item in &script.members {
        member(
            &mut text,
            item,
            Symbol::Member {
                script: script.name.clone(),
                name: item.name.clone(),
            },
            &mut declarations,
        );
    }
    for state in &script.states {
        text.push_str(&format!(
            "{}State {}\n",
            if state.auto { "Auto " } else { "" },
            state.name
        ));
        doc(&mut text, &state.documentation);
        for item in &state.members {
            member(
                &mut text,
                item,
                Symbol::StateMember {
                    script: script.name.clone(),
                    state: state.name.clone(),
                    name: item.name.clone(),
                },
                &mut declarations,
            );
        }
        text.push_str("EndState\n\n");
    }
    Some(DeclarationDocument { text, declarations })
}

#[cfg(test)]
mod tests;
