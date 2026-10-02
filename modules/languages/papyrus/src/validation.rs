//! Pure declaration checks shared by source analysis and API extraction.

use std::collections::{BTreeMap, BTreeSet};

use folio_profiles::{FlagScope, UserFlag};
use folio_source::TextRange;

use crate::{Declaration, FunctionAst, Parse, SyntaxKind, SyntaxNode, declarations};

/// A source contract violation; adapters attach file identity and render the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclarationIssue {
    pub code: &'static str,
    pub range: TextRange,
    pub message: String,
}

fn issue(
    out: &mut Vec<DeclarationIssue>,
    node: &SyntaxNode,
    code: &'static str,
    message: impl Into<String>,
) {
    out.push(DeclarationIssue {
        code,
        range: TextRange {
            start: usize::from(node.text_range().start()),
            end: usize::from(node.text_range().end()),
        },
        message: message.into(),
    });
}

/// Validate source declaration facts without resolving types or compiling bodies.
/// `None` permits unknown dependency metadata flags; standard flag rules still apply.
pub fn validate_declarations(
    parse: &Parse,
    user_flags: Option<&[UserFlag]>,
) -> Vec<DeclarationIssue> {
    let root = parse.syntax();
    let summaries = declarations(parse)
        .into_iter()
        .map(|item| (item.range.start, item.declaration))
        .collect::<BTreeMap<_, _>>();
    let conditional = summaries.values().any(|decl| matches!(decl, Declaration::Script { flags, .. } if flags.iter().any(|flag| flag.eq_ignore_ascii_case("conditional"))));
    let mut out = Vec::new();
    for node in root.descendants() {
        if !matches!(
            node.kind(),
            SyntaxKind::ScriptDecl
                | SyntaxKind::VariableDecl
                | SyntaxKind::PropertyDecl
                | SyntaxKind::FunctionDecl
                | SyntaxKind::EventDecl
                | SyntaxKind::StateDecl
                | SyntaxKind::ImportDecl
        ) {
            continue;
        }
        let declaration = summaries.get(&usize::from(node.text_range().start()));
        let local = node.kind() == SyntaxKind::VariableDecl
            && node
                .parent()
                .is_some_and(|parent| parent.kind() == SyntaxKind::Statement);
        if local {
            if user_flags.is_none() {
                continue;
            }
            if let Some((_, _, flags)) = crate::typed_member(&node) {
                check_flags(
                    &node,
                    &flags,
                    None,
                    false,
                    conditional,
                    user_flags,
                    &mut out,
                );
            }
            continue;
        }
        let Some(declaration) = declaration else {
            continue;
        };
        let (flags, scope, auto) = match declaration {
            Declaration::Script { flags, .. } => (flags, FlagScope::Script, false),
            Declaration::Variable { flags, .. } => (flags, FlagScope::Variable, false),
            Declaration::Property { flags, .. } => (
                flags,
                FlagScope::Property,
                flags.iter().any(|flag| flag.eq_ignore_ascii_case("auto")),
            ),
            Declaration::Function { modifiers, .. } | Declaration::Event { modifiers, .. } => {
                (modifiers, FlagScope::Function, false)
            }
            Declaration::State { flags, .. } => {
                check_flags(&node, flags, None, false, conditional, user_flags, &mut out);
                continue;
            }
            Declaration::Import { .. } => continue,
        };
        check_flags(
            &node,
            flags,
            Some(scope),
            auto,
            conditional,
            user_flags,
            &mut out,
        );
        match declaration {
            Declaration::Property { ty, flags, .. } => check_property(&node, ty, flags, &mut out),
            Declaration::Variable { ty, .. } => {
                if let Some(value) = initializer(&node) {
                    if !is_constant_literal(&value) {
                        issue(
                            &mut out,
                            &value,
                            "semantic.variable-initializer",
                            "script variable initializer must be a literal",
                        );
                    } else {
                        check_constant_type(&value, ty, "semantic.initializer-type", &mut out);
                    }
                }
            }
            Declaration::Function { .. } | Declaration::Event { .. } => {
                check_parameters(&node, &mut out)
            }
            _ => {}
        }
    }
    tracing::debug!(issues = out.len(), "validated source declaration contracts");
    out
}

fn check_flags(
    node: &SyntaxNode,
    flags: &[String],
    scope: Option<FlagScope>,
    auto: bool,
    conditional_owner: bool,
    user_flags: Option<&[UserFlag]>,
    out: &mut Vec<DeclarationIssue>,
) {
    let mut seen = BTreeSet::new();
    for flag in flags {
        let flag = flag.to_ascii_lowercase();
        if !seen.insert(flag.clone()) {
            issue(
                out,
                node,
                "semantic.duplicate-flag",
                format!("duplicate modifier or flag {flag}"),
            );
            continue;
        }
        let standard = match flag.as_str() {
            "auto" => Some(
                node.kind() == SyntaxKind::PropertyDecl || node.kind() == SyntaxKind::StateDecl,
            ),
            "autoreadonly" => Some(node.kind() == SyntaxKind::PropertyDecl),
            "global" => Some(
                node.kind() == SyntaxKind::FunctionDecl
                    && !node.ancestors().any(|parent| {
                        matches!(
                            parent.kind(),
                            SyntaxKind::StateDecl | SyntaxKind::PropertyDecl
                        )
                    }),
            ),
            "native" => Some(matches!(
                node.kind(),
                SyntaxKind::FunctionDecl | SyntaxKind::EventDecl
            )),
            "hidden" => Some(matches!(
                scope,
                Some(FlagScope::Script | FlagScope::Property)
            )),
            "conditional" => Some(
                matches!(scope, Some(FlagScope::Script | FlagScope::Variable))
                    || (scope == Some(FlagScope::Property) && auto),
            ),
            _ => None,
        };
        let allowed = standard.unwrap_or_else(|| {
            if scope.is_none() {
                return false;
            }
            match user_flags {
                None => !folio_profiles::is_skyrim_keyword(&flag),
                Some(definitions) => definitions
                    .iter()
                    .find(|definition| definition.name.eq_ignore_ascii_case(&flag))
                    .is_some_and(|definition| {
                        definition.applies_to(scope.unwrap())
                            || (scope == Some(FlagScope::Property)
                                && auto
                                && definition.applies_to(FlagScope::Variable))
                    }),
            }
        });
        if !allowed {
            let unknown = standard.is_none()
                && scope.is_some()
                && user_flags.is_some_and(|definitions| {
                    !definitions
                        .iter()
                        .any(|definition| definition.name.eq_ignore_ascii_case(&flag))
                });
            issue(
                out,
                node,
                if unknown {
                    "semantic.unknown-flag"
                } else {
                    "semantic.flag-scope"
                },
                format!("flag {flag} is not allowed on this declaration"),
            );
        } else if flag == "conditional" && scope != Some(FlagScope::Script) && !conditional_owner {
            issue(
                out,
                node,
                "semantic.conditional-owner",
                "Conditional storage requires a Conditional script",
            );
        }
    }
}

fn check_parameters(node: &SyntaxNode, out: &mut Vec<DeclarationIssue>) {
    let Some(parameters) = node
        .children()
        .find(|child| child.kind() == SyntaxKind::ParameterList)
    else {
        return;
    };
    // Named calls can omit a defaulted slot before a required parameter.
    for parameter in parameters
        .children()
        .filter(|child| child.kind() == SyntaxKind::Parameter)
    {
        if let Some(value) = initializer(&parameter) {
            if !is_constant_literal(&value) {
                issue(
                    out,
                    &value,
                    "semantic.parameter-default",
                    "parameter default must be a literal constant",
                );
            } else if let Some(ty) = parameter
                .children()
                .find(|child| child.kind() == SyntaxKind::TypeRef)
                .map(crate::ast::type_text)
            {
                check_constant_type(&value, &ty, "semantic.parameter-default", out);
            }
        }
    }
}

fn check_property(node: &SyntaxNode, ty: &str, flags: &[String], out: &mut Vec<DeclarationIssue>) {
    let auto = flags.iter().any(|flag| flag.eq_ignore_ascii_case("auto"));
    let readonly = flags
        .iter()
        .any(|flag| flag.eq_ignore_ascii_case("autoreadonly"));
    if auto && readonly {
        issue(
            out,
            node,
            "semantic.property-form",
            "Auto and AutoReadOnly are mutually exclusive",
        );
    }
    let value = initializer(node);
    if readonly && value.is_none() {
        issue(
            out,
            node,
            "semantic.property-initializer",
            "AutoReadOnly requires a literal initializer",
        );
    }
    if let Some(value) = value {
        if !(auto || readonly) || !is_constant_literal(&value) {
            issue(
                out,
                &value,
                "semantic.property-initializer",
                "only generated properties accept literal initializers",
            );
        } else {
            check_constant_type(&value, ty, "semantic.initializer-type", out);
        }
    }
    if auto || readonly {
        return;
    }
    let mut accessors = BTreeSet::new();
    for child in node
        .children()
        .filter(|child| child.kind() == SyntaxKind::Block)
        .flat_map(|block| block.children())
    {
        let Some(accessor) = FunctionAst::cast(child.clone()) else {
            continue;
        };
        let Some(name) = accessor.name() else {
            continue;
        };
        let name = name.to_ascii_lowercase();
        if !accessors.insert(name.clone()) {
            issue(
                out,
                &child,
                "semantic.duplicate-accessor",
                format!("duplicate property accessor {name}"),
            );
        }
        let parameters = accessor.parameters();
        let valid = match name.as_str() {
            "get" => {
                parameters.is_empty()
                    && accessor
                        .return_type()
                        .is_some_and(|result| result.eq_ignore_ascii_case(ty))
            }
            "set" => {
                accessor.return_type().is_none()
                    && parameters.len() == 1
                    && parameters[0].ty.eq_ignore_ascii_case(ty)
                    && parameters[0].default.is_none()
            }
            _ => false,
        };
        if !valid {
            issue(
                out,
                &child,
                "semantic.accessor-signature",
                "property Get/Set signature does not match its property type",
            );
        }
    }
    if accessors.is_empty() {
        issue(
            out,
            node,
            "semantic.property-accessors",
            "full property requires Get and/or Set",
        );
    }
}

fn initializer(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.children_with_tokens()
        .skip_while(|item| item.kind() != SyntaxKind::Equals)
        .skip(1)
        .find_map(|item| item.into_node())
}

fn check_constant_type(
    node: &SyntaxNode,
    ty: &str,
    code: &'static str,
    out: &mut Vec<DeclarationIssue>,
) {
    let valid = crate::constant_literal_text(node)
        .is_some_and(|text| crate::constant_literal_matches_type(&text, ty));
    if !valid {
        issue(
            out,
            node,
            code,
            format!("constant must be a valid literal compatible with {ty}"),
        );
    }
}

/// Constant source declarations accept literals or a single numeric minus.
pub fn is_constant_literal(node: &SyntaxNode) -> bool {
    if node.kind() == SyntaxKind::LiteralExpr {
        return true;
    }
    if node.kind() != SyntaxKind::UnaryExpr {
        return false;
    }
    let tokens = node
        .descendants_with_tokens()
        .filter_map(|item| item.into_token())
        .filter(|token| !token.kind().is_trivia())
        .collect::<Vec<_>>();
    tokens.len() == 2
        && tokens[0].kind() == SyntaxKind::Minus
        && tokens[1].kind() == SyntaxKind::Number
}

#[cfg(test)]
mod tests;
