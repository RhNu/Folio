use crate::{Parameter, SyntaxKind, SyntaxNode};

/// Typed access to a possibly incomplete script header.
#[derive(Debug, Clone)]
pub struct ScriptAst(SyntaxNode);

impl ScriptAst {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::ScriptDecl).then_some(Self(node))
    }
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    pub fn name(&self) -> Option<String> {
        self.words().nth(1)
    }

    pub fn parent(&self) -> Option<String> {
        let mut words = self.words();
        while let Some(word) = words.next() {
            if word.eq_ignore_ascii_case("extends") {
                return words.next();
            }
        }
        None
    }

    pub fn flags(&self) -> Vec<String> {
        let words = self.words().collect::<Vec<_>>();
        let start = if words
            .get(2)
            .is_some_and(|word| word.eq_ignore_ascii_case("extends"))
        {
            4
        } else {
            2
        };
        words
            .into_iter()
            .skip(start)
            .map(|word| word.to_ascii_lowercase())
            .collect()
    }

    fn words(&self) -> impl Iterator<Item = String> + '_ {
        self.0
            .children_with_tokens()
            .filter_map(|element| element.into_token())
            .filter(|token| token.kind() == SyntaxKind::Ident)
            .map(|token| token.text().to_string())
    }
}

/// Typed access to the signature of a possibly incomplete function.
#[derive(Debug, Clone)]
pub struct FunctionAst(SyntaxNode);

impl FunctionAst {
    pub fn cast(node: SyntaxNode) -> Option<Self> {
        (node.kind() == SyntaxKind::FunctionDecl).then_some(Self(node))
    }
    pub fn syntax(&self) -> &SyntaxNode {
        &self.0
    }

    pub fn has_complete_signature(&self) -> bool {
        !self
            .0
            .children_with_tokens()
            .take_while(|element| element.kind() != SyntaxKind::Block)
            .any(|element| match element {
                rowan::NodeOrToken::Node(node) => node
                    .descendants_with_tokens()
                    .any(|item| item.kind() == SyntaxKind::Missing),
                rowan::NodeOrToken::Token(token) => token.kind() == SyntaxKind::Missing,
            })
    }

    pub fn name(&self) -> Option<String> {
        let mut saw_keyword = false;
        for token in self
            .0
            .children_with_tokens()
            .take_while(|element| element.kind() != SyntaxKind::ParameterList)
            .filter_map(|element| element.into_token())
        {
            if token.kind() != SyntaxKind::Ident {
                continue;
            }
            if saw_keyword {
                return Some(token.text().to_string());
            }
            saw_keyword = token.text().eq_ignore_ascii_case("function");
        }
        None
    }

    pub fn return_type(&self) -> Option<String> {
        self.0
            .children()
            .find(|child| child.kind() == SyntaxKind::TypeRef)
            .map(type_text)
    }

    pub fn parameters(&self) -> Vec<Parameter> {
        node_parameters(&self.0)
    }

    pub fn modifiers(&self) -> Vec<String> {
        node_modifiers(&self.0)
    }
}

pub(crate) fn node_parameters(node: &SyntaxNode) -> Vec<Parameter> {
    node.children()
        .find(|child| child.kind() == SyntaxKind::ParameterList)
        .into_iter()
        .flat_map(|list| list.children())
        .filter(|child| child.kind() == SyntaxKind::Parameter)
        .filter_map(|node| {
            let ty = node
                .children()
                .find(|child| child.kind() == SyntaxKind::TypeRef)
                .map(type_text)?;
            if ty.is_empty() {
                return None;
            }
            let name = node
                .children_with_tokens()
                .filter_map(|element| element.into_token())
                .find(|token| token.kind() == SyntaxKind::Ident)?
                .text()
                .to_string();
            let default = node
                .children_with_tokens()
                .skip_while(|element| element.kind() != SyntaxKind::Equals)
                .skip(1)
                .flat_map(|element| element.into_node())
                .flat_map(|expression| expression.descendants_with_tokens())
                .filter_map(|element| element.into_token())
                .filter(|token| !token.kind().is_trivia())
                .map(|token| token.text().to_string())
                .collect::<String>();
            Some(Parameter {
                name,
                ty,
                default: (!default.is_empty()).then_some(default),
            })
        })
        .collect()
}

pub(crate) fn node_modifiers(node: &SyntaxNode) -> Vec<String> {
    node.children_with_tokens()
        .skip_while(|element| element.kind() != SyntaxKind::ParameterList)
        .skip(1)
        .take_while(|element| element.kind() != SyntaxKind::Block)
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::Ident)
        .map(|token| token.text().to_ascii_lowercase())
        .collect()
}

pub(crate) fn type_text(node: SyntaxNode) -> String {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .map(|token| token.text().to_string())
        .collect()
}
