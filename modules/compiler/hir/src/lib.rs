//! Owned, source mapped semantic input and typed facts shared by analysis clients.

use folio_source::SourceSpan;

/// A name and its original source range, before name binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameRef {
    pub text: String,
    pub span: SourceSpan,
}

/// An unresolved type spelling. Resolution never changes the spelling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeRef {
    pub text: String,
    pub span: SourceSpan,
}

/// Semantic type facts; `Error` prevents one failed lookup from cascading.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Void,
    Int,
    Float,
    Bool,
    String,
    Script(String),
    Array(Box<Type>),
    None,
    Error,
}

impl Type {
    /// Parses builtin and nominal Papyrus types without consulting a symbol table.
    pub fn from_spelling(text: &str) -> Self {
        let text = text.trim();
        if let Some(element) = text.strip_suffix("[]") {
            return Self::Array(Box::new(Self::from_spelling(element)));
        }
        match text.to_ascii_lowercase().as_str() {
            "none" => Self::None,
            "int" => Self::Int,
            "float" => Self::Float,
            "bool" => Self::Bool,
            "string" => Self::String,
            _ if text.is_empty() => Self::Error,
            _ => Self::Script(text.to_owned()),
        }
    }
}

/// Stable semantic identity within one analysis view.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Symbol {
    Script(String),
    /// Compiler-provided operations independent of CK or SDK declarations.
    Intrinsic {
        name: String,
    },
    Member {
        script: String,
        name: String,
    },
    StateMember {
        script: String,
        state: String,
        name: String,
    },
    PropertyAccessor {
        script: String,
        property: String,
        name: String,
    },
    Parameter {
        owner: Box<Symbol>,
        name: String,
    },
    Local {
        owner: Box<Symbol>,
        name: String,
    },
}

/// A resolved use, including the original spelling and its defining location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub name: NameRef,
    pub symbol: Symbol,
    pub definition: Option<SourceSpan>,
}

/// A typed expression fact. Conversion records an inserted implicit conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpressionFact {
    pub span: SourceSpan,
    pub ty: Type,
    pub binding: Option<Binding>,
    pub conversion: Option<Type>,
    pub kind: ExpressionKind,
}

/// Typed expression shape retained for later target lowering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpressionKind {
    Missing,
    Literal(String),
    Reference(NameRef),
    Unary {
        operator: String,
        operand: Box<ExpressionFact>,
    },
    Binary {
        operator: String,
        left: Box<ExpressionFact>,
        right: Box<ExpressionFact>,
    },
    Cast {
        value: Box<ExpressionFact>,
        target: Type,
    },
    Member {
        owner: Box<ExpressionFact>,
        name: NameRef,
    },
    Index {
        owner: Box<ExpressionFact>,
        index: Box<ExpressionFact>,
    },
    NewArray {
        element_type: Type,
        length: Box<ExpressionFact>,
    },
    Call {
        callee: Box<ExpressionFact>,
        arguments: Vec<ExpressionFact>,
        /// Source-order arguments mapped to declaration-order parameters.
        argument_ordinals: Vec<usize>,
        /// Each declaration-order omitted parameter needs its literal default.
        parameter_defaults: Vec<Option<(Type, String)>>,
        is_global: bool,
    },
    Parenthesized(Box<ExpressionFact>),
}

/// A source statement with typed child expressions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Statement {
    Return {
        span: SourceSpan,
        value: Option<ExpressionFact>,
    },
    Variable {
        declaration: DeclarationFact,
        value: Option<ExpressionFact>,
    },
    Assignment {
        span: SourceSpan,
        target: Box<ExpressionFact>,
        value: Box<ExpressionFact>,
        operator: String,
    },
    Expression(ExpressionFact),
    If {
        span: SourceSpan,
        condition: ExpressionFact,
        then_branch: Vec<Statement>,
        else_if: Vec<(ExpressionFact, Vec<Statement>)>,
        else_branch: Vec<Statement>,
    },
    While {
        span: SourceSpan,
        condition: ExpressionFact,
        body: Vec<Statement>,
    },
    Error(SourceSpan),
}

/// Typed body of a callable member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    pub symbol: Symbol,
    pub return_type: Type,
    pub parameters: Vec<ParameterFact>,
    pub statements: Vec<Statement>,
}

/// Source declaration information required for target layout and ABI lowering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemberKind {
    Variable,
    Property {
        auto: bool,
        read_only: bool,
    },
    Function {
        event: bool,
        global: bool,
        native: bool,
    },
}

/// A named parameter in declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterFact {
    pub name: String,
    pub ty: Type,
    pub default_literal: Option<String>,
    pub span: SourceSpan,
}

/// One source member, distinct from body and cross-reference facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberFact {
    pub symbol: Symbol,
    pub kind: MemberKind,
    pub ty: Type,
    pub parameters: Vec<ParameterFact>,
    pub flags: Vec<String>,
    pub initial_literal: Option<String>,
    pub span: SourceSpan,
}

/// State declaration, including the runtime startup state marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateFact {
    pub name: String,
    pub auto: bool,
    pub span: SourceSpan,
}

/// Arguments are bound to parameter ordinals for consumers of typed calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallFact {
    pub span: SourceSpan,
    pub target: Option<Symbol>,
    pub arguments: Vec<usize>,
    pub result: Type,
}

/// Source declaration with a resolved type and original location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclarationFact {
    pub symbol: Symbol,
    pub ty: Type,
    pub span: SourceSpan,
}

/// Owned partial semantic model. Error facts coexist with valid sibling facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Script {
    pub name: Option<NameRef>,
    pub parent: Option<NameRef>,
    pub flags: Vec<String>,
    pub members: Vec<MemberFact>,
    /// Resolved ancestor and SDK member shapes needed by code generation.
    pub external_members: Vec<MemberFact>,
    /// Typed shapes for members reached through other script instances or imports.
    pub referenced_members: Vec<MemberFact>,
    pub states: Vec<StateFact>,
    pub declarations: Vec<DeclarationFact>,
    pub bodies: Vec<Body>,
    pub expressions: Vec<ExpressionFact>,
    pub calls: Vec<CallFact>,
}

impl Script {
    /// Finds the innermost typed expression covering a source byte offset.
    pub fn expression_at(&self, offset: usize) -> Option<&ExpressionFact> {
        self.expressions
            .iter()
            .filter(|fact| fact.span.range.start <= offset && offset < fact.span.range.end)
            .min_by_key(|fact| fact.span.range.end - fact.span.range.start)
    }
}
