//! Target-legalized, source-mapped operations with explicit branch labels.

use folio_profiles::TargetProfile;
use folio_source::SourceSpan;

/// A concrete value or named storage slot; names are allocated during lowering.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    Int(i32),
    Float(f32),
    String(String),
    Identifier(String),
}

/// Operations have already passed target capability selection.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Assign(Value, Value),
    Cast(Value, Value),
    Unary {
        operator: UnaryOp,
        dest: Value,
        value: Value,
    },
    Binary {
        operator: BinaryOp,
        dest: Value,
        left: Value,
        right: Value,
    },
    CallMethod {
        name: String,
        receiver: Value,
        dest: Value,
        args: Vec<Value>,
    },
    CallParent {
        name: String,
        dest: Value,
        args: Vec<Value>,
    },
    CallStatic {
        script: String,
        name: String,
        dest: Value,
        args: Vec<Value>,
    },
    PropertyGet {
        name: String,
        receiver: Value,
        dest: Value,
    },
    PropertySet {
        name: String,
        receiver: Value,
        value: Value,
    },
    ArrayCreate {
        dest: Value,
        length: Value,
    },
    ArrayLength {
        dest: Value,
        array: Value,
    },
    ArrayGet {
        dest: Value,
        array: Value,
        index: Value,
    },
    ArraySet {
        array: Value,
        index: Value,
        value: Value,
    },
    ArrayFind {
        reverse: bool,
        dest: Value,
        array: Value,
        value: Value,
        start: Value,
    },
    Jump(u32),
    JumpIf {
        when_true: bool,
        condition: Value,
        target: u32,
    },
    Label(u32),
    Return(Value),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    NegInt,
    NegFloat,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    AddInt,
    AddFloat,
    AddString,
    SubInt,
    SubFloat,
    MulInt,
    MulFloat,
    DivInt,
    DivFloat,
    ModInt,
    Eq,
    Lt,
    Lte,
    Gt,
    Gte,
}

/// Every emitted operation retains the source use that required it.
#[derive(Clone, Debug, PartialEq)]
pub struct Instruction {
    pub op: Op,
    pub source: SourceSpan,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    pub name: String,
    pub ty: String,
    pub initial: Value,
    pub flags: u32,
    pub source: SourceSpan,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    pub ty: String,
    pub auto_var: Option<String>,
    pub read_only: bool,
    pub getter: Option<Function>,
    pub setter: Option<Function>,
    pub flags: u32,
    pub source: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    pub name: String,
    pub ty: String,
}

/// Parent-owned storage retained for provenance, never emitted locally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalSlot {
    pub owner: String,
    pub name: String,
    pub ty: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub state: String,
    pub return_type: String,
    pub parameters: Vec<Local>,
    pub locals: Vec<Local>,
    pub instructions: Vec<Instruction>,
    pub flags: u32,
    pub is_global: bool,
    pub is_native: bool,
    pub is_event: bool,
    pub source: SourceSpan,
}

/// A single selected source script after target legalization.
#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    pub target: TargetProfile,
    pub name: String,
    pub parent: String,
    pub flags: u32,
    pub auto_state: String,
    pub variables: Vec<Variable>,
    /// Parent-owned fields retain declaring identity without creating child storage.
    pub external_slots: Vec<ExternalSlot>,
    pub properties: Vec<Property>,
    pub functions: Vec<Function>,
    pub state_names: Vec<String>,
    pub source: SourceSpan,
    pub decisions: Vec<Decision>,
}

/// Per-use target selection for explainable build plans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub feature: &'static str,
    pub outcome: Outcome,
    pub source: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Native,
    Lowered { rule: &'static str },
}

mod validation;
pub use validation::{ValidationError, ValidationErrorKind, reaches_end, validate};
