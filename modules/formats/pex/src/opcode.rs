use super::{PexStringId, PexWriteError};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PexValue {
    None,
    Identifier(PexStringId),
    String(PexStringId),
    Integer(i32),
    Float(f32),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PexOpcode {
    Nop               = 0,
    IAdd              = 1,
    FAdd              = 2,
    ISub              = 3,
    FSub              = 4,
    IMul              = 5,
    FMul              = 6,
    IDiv              = 7,
    FDiv              = 8,
    IMod              = 9,
    Not               = 10,
    INeg              = 11,
    FNeg              = 12,
    Assign            = 13,
    Cast              = 14,
    CmpEq             = 15,
    CmpLt             = 16,
    CmpLte            = 17,
    CmpGt             = 18,
    CmpGte            = 19,
    Jmp               = 20,
    JmpT              = 21,
    JmpF              = 22,
    CallMethod        = 23,
    CallParent        = 24,
    CallStatic        = 25,
    Return            = 26,
    StrCat            = 27,
    PropGet           = 28,
    PropSet           = 29,
    ArrayCreate       = 30,
    ArrayLength       = 31,
    ArrayGetElement   = 32,
    ArraySetElement   = 33,
    ArrayFindElement  = 34,
    ArrayRFindElement = 35,
}

impl PexOpcode {
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Nop),
            1 => Some(Self::IAdd),
            2 => Some(Self::FAdd),
            3 => Some(Self::ISub),
            4 => Some(Self::FSub),
            5 => Some(Self::IMul),
            6 => Some(Self::FMul),
            7 => Some(Self::IDiv),
            8 => Some(Self::FDiv),
            9 => Some(Self::IMod),
            10 => Some(Self::Not),
            11 => Some(Self::INeg),
            12 => Some(Self::FNeg),
            13 => Some(Self::Assign),
            14 => Some(Self::Cast),
            15 => Some(Self::CmpEq),
            16 => Some(Self::CmpLt),
            17 => Some(Self::CmpLte),
            18 => Some(Self::CmpGt),
            19 => Some(Self::CmpGte),
            20 => Some(Self::Jmp),
            21 => Some(Self::JmpT),
            22 => Some(Self::JmpF),
            23 => Some(Self::CallMethod),
            24 => Some(Self::CallParent),
            25 => Some(Self::CallStatic),
            26 => Some(Self::Return),
            27 => Some(Self::StrCat),
            28 => Some(Self::PropGet),
            29 => Some(Self::PropSet),
            30 => Some(Self::ArrayCreate),
            31 => Some(Self::ArrayLength),
            32 => Some(Self::ArrayGetElement),
            33 => Some(Self::ArraySetElement),
            34 => Some(Self::ArrayFindElement),
            35 => Some(Self::ArrayRFindElement),
            _ => None,
        }
    }

    pub const fn fixed_arg_count(self) -> usize {
        match self {
            Self::Nop => 0,
            Self::Jmp | Self::Return => 1,
            Self::Not
            | Self::INeg
            | Self::FNeg
            | Self::Assign
            | Self::Cast
            | Self::JmpT
            | Self::JmpF
            | Self::CallParent
            | Self::ArrayCreate
            | Self::ArrayLength => 2,
            Self::IAdd
            | Self::FAdd
            | Self::ISub
            | Self::FSub
            | Self::IMul
            | Self::FMul
            | Self::IDiv
            | Self::FDiv
            | Self::IMod
            | Self::CmpEq
            | Self::CmpLt
            | Self::CmpLte
            | Self::CmpGt
            | Self::CmpGte
            | Self::CallMethod
            | Self::CallStatic
            | Self::StrCat
            | Self::PropGet
            | Self::PropSet
            | Self::ArrayGetElement
            | Self::ArraySetElement => 3,
            Self::ArrayFindElement | Self::ArrayRFindElement => 4,
        }
    }

    pub(crate) const fn has_variadic_arguments(self) -> bool {
        matches!(self, Self::CallMethod | Self::CallParent | Self::CallStatic)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Nop => "NOP",
            Self::IAdd => "IADD",
            Self::FAdd => "FADD",
            Self::ISub => "ISUB",
            Self::FSub => "FSUB",
            Self::IMul => "IMUL",
            Self::FMul => "FMUL",
            Self::IDiv => "IDIV",
            Self::FDiv => "FDIV",
            Self::IMod => "IMOD",
            Self::Not => "NOT",
            Self::INeg => "INEG",
            Self::FNeg => "FNEG",
            Self::Assign => "ASSIGN",
            Self::Cast => "CAST",
            Self::CmpEq => "CMPEQ",
            Self::CmpLt => "CMPLT",
            Self::CmpLte => "CMPLTE",
            Self::CmpGt => "CMPGT",
            Self::CmpGte => "CMPGTE",
            Self::Jmp => "JMP",
            Self::JmpT => "JMPT",
            Self::JmpF => "JMPF",
            Self::CallMethod => "CALLMETHOD",
            Self::CallParent => "CALLPARENT",
            Self::CallStatic => "CALLSTATIC",
            Self::Return => "RETURN",
            Self::StrCat => "STRCAT",
            Self::PropGet => "PROPGET",
            Self::PropSet => "PROPSET",
            Self::ArrayCreate => "ARRAYCREATE",
            Self::ArrayLength => "ARRAYLENGTH",
            Self::ArrayGetElement => "ARRAYGETELEMENT",
            Self::ArraySetElement => "ARRAYSETELEMENT",
            Self::ArrayFindElement => "ARRAYFINDELEMENT",
            Self::ArrayRFindElement => "ARRAYRFINDELEMENT",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PexInstruction {
    pub opcode: PexOpcode,
    pub arguments: Vec<PexValue>,
    pub variadic_arguments: Vec<PexValue>,
}

impl PexInstruction {
    /// # Errors
    /// Returns an error when the opcode argument count is invalid or requires variadic arguments.
    pub fn new(opcode: PexOpcode, arguments: Vec<PexValue>) -> Result<Self, PexWriteError> {
        let expected = opcode.fixed_arg_count();
        let actual = arguments.len();
        if actual != expected {
            return Err(PexWriteError::InvalidInstructionArity {
                opcode,
                expected,
                actual,
            });
        }

        Ok(Self {
            opcode,
            arguments,
            variadic_arguments: Vec::new(),
        })
    }

    /// # Errors
    /// Returns an error when the fixed argument count or variadic opcode usage is invalid.
    pub fn new_variadic(
        opcode: PexOpcode,
        arguments: Vec<PexValue>,
        variadic_arguments: Vec<PexValue>,
    ) -> Result<Self, PexWriteError> {
        if !opcode.has_variadic_arguments() && !variadic_arguments.is_empty() {
            return Err(PexWriteError::UnexpectedVariadicArguments { opcode });
        }

        let mut instruction = Self::new(opcode, arguments)?;
        instruction.variadic_arguments = variadic_arguments;
        Ok(instruction)
    }
}
