//! Minimal source-mapped MIR builders shared by validation tests.
use crate::{Function, Instruction, Local, Op};
use folio_source::{FileId, SourceSpan, TextRange};

pub(crate) fn source() -> SourceSpan {
    SourceSpan {
        file: FileId(7),
        range: TextRange { start: 10, end: 20 },
    }
}

pub(crate) fn function(ops: Vec<Op>) -> Function {
    Function {
        name: "Evaluate".into(),
        state: String::new(),
        return_type: "None".into(),
        parameters: vec![],
        locals: vec![Local {
            name: "result".into(),
            ty: "Int".into(),
        }],
        instructions: ops
            .into_iter()
            .map(|op| Instruction {
                op,
                source: source(),
            })
            .collect(),
        is_global: false,
        is_native: false,
        is_event: false,
        flags: 0,
        source: source(),
    }
}
