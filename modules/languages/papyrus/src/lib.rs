//! Lossless, error-tolerant Papyrus syntax and declaration facts.

mod ast;
mod documentation;
mod lexer;
mod literals;
mod parser;
mod source_structure;
mod validation;

use folio_source::TextRange;
use rowan::{GreenNode, Language};

pub use ast::{FunctionAst, ScriptAst};
pub use documentation::{declaration_documentation, declaration_header};
pub use lexer::{LexToken, lex};
pub use literals::normalize_constant_literal_text;
pub use literals::{
    constant_literal_matches_type, constant_literal_text, decode_integer_literal,
    decode_string_literal,
};
pub use parser::parse;
pub use validation::{DeclarationIssue, is_constant_literal, validate_declarations};

/// The language policy is explicit even while the first parser shares Skyrim syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PapyrusDialect {
    Skyrim,
}

/// Kinds cover both source tokens and lossless syntax nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u16)]
pub enum SyntaxKind {
    Root,
    ScriptDecl,
    ImportDecl,
    FunctionDecl,
    EventDecl,
    PropertyDecl,
    StateDecl,
    ParameterList,
    Parameter,
    TypeRef,
    Block,
    Statement,
    VariableDecl,
    ReturnStmt,
    AssignmentStmt,
    NewArrayExpr,
    NamedArgument,
    IfStmt,
    ElseIfClause,
    ElseClause,
    WhileStmt,
    BinaryExpr,
    UnaryExpr,
    CallExpr,
    MemberExpr,
    IndexExpr,
    NameExpr,
    LiteralExpr,
    ParenExpr,
    Error,
    Missing,
    Ident,
    Number,
    String,
    UnclosedString,
    Whitespace,
    Newline,
    Comment,
    UnclosedComment,
    Continuation,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Equals,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    EqEq,
    NotEq,
    LessEq,
    GreaterEq,
    AndAnd,
    OrOr,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Less,
    Greater,
    Bang,
    Unknown,
}

impl SyntaxKind {
    pub(crate) fn is_trivia(self) -> bool {
        matches!(self, Self::Whitespace | Self::Comment | Self::Continuation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PapyrusLanguage {}

impl Language for PapyrusLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        const KINDS: &[SyntaxKind] = &[
            SyntaxKind::Root,
            SyntaxKind::ScriptDecl,
            SyntaxKind::ImportDecl,
            SyntaxKind::FunctionDecl,
            SyntaxKind::EventDecl,
            SyntaxKind::PropertyDecl,
            SyntaxKind::StateDecl,
            SyntaxKind::ParameterList,
            SyntaxKind::Parameter,
            SyntaxKind::TypeRef,
            SyntaxKind::Block,
            SyntaxKind::Statement,
            SyntaxKind::VariableDecl,
            SyntaxKind::ReturnStmt,
            SyntaxKind::AssignmentStmt,
            SyntaxKind::NewArrayExpr,
            SyntaxKind::NamedArgument,
            SyntaxKind::IfStmt,
            SyntaxKind::ElseIfClause,
            SyntaxKind::ElseClause,
            SyntaxKind::WhileStmt,
            SyntaxKind::BinaryExpr,
            SyntaxKind::UnaryExpr,
            SyntaxKind::CallExpr,
            SyntaxKind::MemberExpr,
            SyntaxKind::IndexExpr,
            SyntaxKind::NameExpr,
            SyntaxKind::LiteralExpr,
            SyntaxKind::ParenExpr,
            SyntaxKind::Error,
            SyntaxKind::Missing,
            SyntaxKind::Ident,
            SyntaxKind::Number,
            SyntaxKind::String,
            SyntaxKind::UnclosedString,
            SyntaxKind::Whitespace,
            SyntaxKind::Newline,
            SyntaxKind::Comment,
            SyntaxKind::UnclosedComment,
            SyntaxKind::Continuation,
            SyntaxKind::LParen,
            SyntaxKind::RParen,
            SyntaxKind::LBracket,
            SyntaxKind::RBracket,
            SyntaxKind::Comma,
            SyntaxKind::Dot,
            SyntaxKind::Equals,
            SyntaxKind::PlusEq,
            SyntaxKind::MinusEq,
            SyntaxKind::StarEq,
            SyntaxKind::SlashEq,
            SyntaxKind::PercentEq,
            SyntaxKind::EqEq,
            SyntaxKind::NotEq,
            SyntaxKind::LessEq,
            SyntaxKind::GreaterEq,
            SyntaxKind::AndAnd,
            SyntaxKind::OrOr,
            SyntaxKind::Plus,
            SyntaxKind::Minus,
            SyntaxKind::Star,
            SyntaxKind::Slash,
            SyntaxKind::Percent,
            SyntaxKind::Less,
            SyntaxKind::Greater,
            SyntaxKind::Bang,
            SyntaxKind::Unknown,
        ];
        KINDS
            .get(usize::from(raw.0))
            .copied()
            .expect("unknown Papyrus syntax kind")
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        rowan::SyntaxKind(kind as u16)
    }
}

pub type SyntaxNode = rowan::SyntaxNode<PapyrusLanguage>;
pub type SyntaxToken = rowan::SyntaxToken<PapyrusLanguage>;

/// A recoverable parser failure, located in UTF-8 byte offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub kind: SyntaxErrorKind,
    pub range: TextRange,
    pub message: String,
}

/// Stable syntax failure categories independent of presentation wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntaxErrorKind {
    UnclosedString,
    UnclosedComment,
    Missing(SyntaxKind),
    MissingFunctionKeyword,
    MissingEndFunction,
    MissingEndEvent,
    MissingEndProperty,
    MissingEndState,
    MissingEndIf,
    MissingEndWhile,
    ExpectedExpression,
    UnexpectedText,
    ExpectedDeclaration,
    UnsupportedStatement,
    ReturnOutsideFunction,
}

/// Immutable parsed text; green trees can be retained by incremental queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parse {
    pub green: GreenNode,
    pub errors: Vec<SyntaxError>,
}

impl Parse {
    pub fn syntax(&self) -> SyntaxNode {
        SyntaxNode::new_root(self.green.clone())
    }
}

/// A declaration's semantic identity excludes source offsets and function bodies.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Declaration {
    Script {
        name: String,
        parent: Option<String>,
        flags: Vec<String>,
    },
    Function {
        name: String,
        return_type: Option<String>,
        parameters: Vec<Parameter>,
        modifiers: Vec<String>,
    },
    Import {
        name: String,
    },
    Variable {
        name: String,
        ty: String,
        flags: Vec<String>,
    },
    Property {
        name: String,
        ty: String,
        flags: Vec<String>,
    },
    State {
        name: String,
        flags: Vec<String>,
    },
    Event {
        name: String,
        parameters: Vec<Parameter>,
        modifiers: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Parameter {
    pub name: String,
    pub ty: String,
    pub default: Option<String>,
}

/// A declaration's current location accompanies its stable semantic summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedDeclaration {
    pub declaration: Declaration,
    pub range: TextRange,
}

/// Extracts available signatures even when other syntax is damaged.
pub fn declarations(parse: &Parse) -> Vec<LocatedDeclaration> {
    let root = parse.syntax();
    let mut out = Vec::new();
    for node in root.descendants().skip(1).filter(is_declaration_scope) {
        let declaration = match node.kind() {
            SyntaxKind::ScriptDecl => ScriptAst::cast(node.clone()).and_then(|script| {
                script.name().map(|name| Declaration::Script {
                    name,
                    parent: script.parent(),
                    flags: script.flags(),
                })
            }),
            SyntaxKind::FunctionDecl => FunctionAst::cast(node.clone())
                .filter(FunctionAst::has_complete_signature)
                .and_then(|function| {
                    function.name().map(|name| Declaration::Function {
                        name,
                        return_type: function.return_type(),
                        parameters: function.parameters(),
                        modifiers: function.modifiers(),
                    })
                }),
            SyntaxKind::ImportDecl => direct_words(&node)
                .get(1)
                .cloned()
                .map(|name| Declaration::Import { name }),
            SyntaxKind::VariableDecl => typed_member(&node)
                .map(|(name, ty, flags)| Declaration::Variable { name, ty, flags }),
            SyntaxKind::PropertyDecl => typed_member(&node)
                .map(|(name, ty, flags)| Declaration::Property { name, ty, flags }),
            SyntaxKind::StateDecl => {
                let words = direct_words(&node);
                let keyword = words
                    .iter()
                    .position(|word| word.eq_ignore_ascii_case("state"));
                keyword.and_then(|index| {
                    words
                        .get(index + 1)
                        .cloned()
                        .map(|name| Declaration::State {
                            name,
                            flags: words
                                .into_iter()
                                .take(index)
                                .chain(direct_words(&node).into_iter().skip(index + 2))
                                .map(|word| word.to_ascii_lowercase())
                                .collect(),
                        })
                })
            }
            SyntaxKind::EventDecl => {
                direct_words(&node)
                    .get(1)
                    .cloned()
                    .map(|name| Declaration::Event {
                        name,
                        parameters: ast::node_parameters(&node),
                        modifiers: ast::node_modifiers(&node),
                    })
            }
            _ => None,
        };
        if let Some(declaration) = declaration {
            let range = node.text_range();
            out.push(LocatedDeclaration {
                declaration,
                range: TextRange {
                    start: usize::from(range.start()),
                    end: usize::from(range.end()),
                },
            });
        }
    }
    out
}

fn is_declaration_scope(node: &SyntaxNode) -> bool {
    match node.parent() {
        Some(parent) if parent.kind() == SyntaxKind::Root => true,
        Some(parent) if parent.kind() == SyntaxKind::Block => {
            parent.parent().is_some_and(|owner| {
                matches!(
                    owner.kind(),
                    SyntaxKind::StateDecl | SyntaxKind::PropertyDecl
                )
            })
        }
        _ => false,
    }
}

fn direct_words(node: &SyntaxNode) -> Vec<String> {
    node.children_with_tokens()
        .take_while(|element| element.kind() != SyntaxKind::Block)
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::Ident)
        .map(|token| token.text().to_string())
        .collect()
}

fn typed_member(node: &SyntaxNode) -> Option<(String, String, Vec<String>)> {
    let ty = node
        .children()
        .find(|child| child.kind() == SyntaxKind::TypeRef)
        .map(ast::type_text)?;
    let words = direct_words(node);
    let property = node.kind() == SyntaxKind::PropertyDecl;
    let name_index = usize::from(property);
    let name = words.get(name_index)?.clone();
    let flags = words
        .into_iter()
        .skip(name_index + 1)
        .map(|word| word.to_ascii_lowercase())
        .collect();
    Some((name, ty, flags))
}
