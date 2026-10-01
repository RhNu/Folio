//! Synthetic HIR inputs and an independent minimal MIR evaluator.
use folio_hir::{
    Binding, Body, ExpressionFact, ExpressionKind, MemberFact, MemberKind, NameRef, ParameterFact,
    Statement, Symbol, Type,
};
use folio_lowering::lower_script;
use folio_mir::{BinaryOp, Op, Script, UnaryOp, Value};
use folio_profiles::TargetProfile;
use folio_source::{FileId, SourceSpan, TextRange};
use std::collections::BTreeMap;

pub(super) fn at() -> SourceSpan {
    SourceSpan {
        file: FileId(4),
        range: TextRange { start: 12, end: 18 },
    }
}

pub(super) fn symbol(name: &str) -> Symbol {
    Symbol::Member {
        script: "Probe".into(),
        name: name.into(),
    }
}

pub(super) fn name(text: &str) -> NameRef {
    NameRef {
        text: text.into(),
        span: at(),
    }
}

pub(super) fn expression(ty: Type, kind: ExpressionKind) -> ExpressionFact {
    ExpressionFact {
        span: at(),
        ty,
        binding: None,
        conversion: None,
        kind,
    }
}

pub(super) fn integer(value: i32) -> ExpressionFact {
    expression(Type::Int, ExpressionKind::Literal(value.to_string()))
}

pub(super) fn boolean(value: bool) -> ExpressionFact {
    expression(Type::Bool, ExpressionKind::Literal(value.to_string()))
}

pub(super) fn reference(text: &str, ty: Type) -> ExpressionFact {
    let mut result = expression(ty, ExpressionKind::Reference(name(text)));
    result.binding = Some(Binding {
        name: name(text),
        symbol: symbol(text),
        definition: None,
    });
    result
}

pub(super) fn call(
    text: &str,
    ty: Type,
    arguments: Vec<ExpressionFact>,
    ordinals: Vec<usize>,
    defaults: Vec<Option<(Type, String)>>,
) -> ExpressionFact {
    expression(
        ty.clone(),
        ExpressionKind::Call {
            callee: Box::new(reference(text, ty)),
            arguments,
            argument_ordinals: ordinals,
            parameter_defaults: defaults,
            is_global: false,
        },
    )
}

pub(super) fn callable(text: &str, ty: Type, native: bool) -> MemberFact {
    let parameters = if text == "Combine" {
        vec![
            ParameterFact {
                name: "a".into(),
                ty: Type::Int,
                default_literal: None,
                span: at(),
            },
            ParameterFact {
                name: "b".into(),
                ty: Type::Int,
                default_literal: Some("2".into()),
                span: at(),
            },
        ]
    } else {
        vec![]
    };
    MemberFact {
        symbol: symbol(text),
        kind: MemberKind::Function {
            event: false,
            global: false,
            native,
        },
        ty,
        parameters,
        flags: vec![],
        initial_literal: None,
        span: at(),
    }
}

pub(super) fn program(statements: Vec<Statement>, return_type: Type) -> folio_hir::Script {
    folio_hir::Script {
        name: Some(name("Probe")),
        members: vec![callable("Use", return_type.clone(), false)],
        bodies: vec![Body {
            symbol: symbol("Use"),
            return_type,
            parameters: vec![],
            statements,
        }],
        ..Default::default()
    }
}

pub(super) fn returns(value: ExpressionFact) -> folio_hir::Script {
    let ty = value.ty.clone();
    program(
        vec![Statement::Return {
            span: at(),
            value: Some(value),
        }],
        ty,
    )
}

pub(super) fn lower(source: &folio_hir::Script) -> Script {
    lower_script(source, TargetProfile::skyrim_se(), &[]).unwrap()
}

pub(super) fn with_field(source: &mut folio_hir::Script) {
    source.members.push(MemberFact {
        symbol: symbol("x"),
        kind: MemberKind::Variable,
        ty: Type::Int,
        parameters: vec![],
        flags: vec![],
        initial_literal: Some("3".into()),
        span: at(),
    });
    source.members.push(callable("SetX", Type::Int, true));
}

// This deliberately tiny evaluator implements only the operations needed by
// these semantic examples. Calls have specified test effects; no Papyrus parser,
// PEX codec or production expression evaluator participates in the oracle.
pub(super) struct Machine {
    pub(super) slots: BTreeMap<String, Value>,
    pub(super) ticks: i32,
}

impl Machine {
    fn value(&self, value: &Value) -> Value {
        match value {
            Value::Identifier(name) => self
                .slots
                .get(name)
                .unwrap_or_else(|| panic!("unset {name}"))
                .clone(),
            other => other.clone(),
        }
    }

    fn put(&mut self, destination: &Value, value: Value) {
        let Value::Identifier(name) = destination else {
            panic!("non-slot destination");
        };
        self.slots.insert(name.clone(), value);
    }

    pub(super) fn run(script: &Script) -> (Value, Self) {
        let mut machine = Self {
            slots: script
                .variables
                .iter()
                .map(|variable| (variable.name.clone(), variable.initial.clone()))
                .collect(),
            ticks: 0,
        };
        let function = script
            .functions
            .iter()
            .find(|function| function.name == "Use")
            .unwrap();
        let labels = function
            .instructions
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                if let Op::Label(label) = item.op {
                    Some((label, index))
                } else {
                    None
                }
            })
            .collect::<BTreeMap<_, _>>();
        let mut pc = 0;
        for _ in 0..500 {
            let op = &function.instructions[pc].op;
            pc += 1;
            match op {
                Op::Assign(destination, value) => machine.put(destination, machine.value(value)),
                Op::Label(_) => {}
                Op::Jump(label) => pc = labels[label],
                Op::JumpIf {
                    when_true,
                    condition,
                    target,
                } => {
                    let Value::Bool(value) = machine.value(condition) else {
                        panic!("non-bool branch");
                    };
                    if value == *when_true {
                        pc = labels[target];
                    }
                }
                Op::Binary {
                    operator: BinaryOp::AddInt,
                    dest,
                    left,
                    right,
                } => {
                    let (Value::Int(left), Value::Int(right)) =
                        (machine.value(left), machine.value(right))
                    else {
                        panic!("non-int add");
                    };
                    machine.put(dest, Value::Int(left + right));
                }
                Op::Binary {
                    operator: BinaryOp::Eq,
                    dest,
                    left,
                    right,
                } => {
                    machine.put(
                        dest,
                        Value::Bool(machine.value(left) == machine.value(right)),
                    );
                }
                Op::Unary {
                    operator: UnaryOp::Not,
                    dest,
                    value,
                } => {
                    let Value::Bool(value) = machine.value(value) else {
                        panic!("non-bool not");
                    };
                    machine.put(dest, Value::Bool(!value));
                }
                Op::CallMethod {
                    name, dest, args, ..
                } => {
                    let value = match name.as_str() {
                        "Tick" => {
                            machine.ticks += 1;
                            Value::Int(machine.ticks)
                        }
                        "Mark" => {
                            machine.ticks += 1;
                            Value::Bool(true)
                        }
                        "SetX" => {
                            machine.slots.insert("x".into(), Value::Int(9));
                            Value::Int(5)
                        }
                        "Combine" => {
                            let (Value::Int(a), Value::Int(b)) =
                                (machine.value(&args[0]), machine.value(&args[1]))
                            else {
                                panic!("non-int arguments");
                            };
                            Value::Int(a * 10 + b)
                        }
                        _ => panic!("unexpected test call {name}"),
                    };
                    machine.put(dest, value);
                }
                Op::Return(value) => return (machine.value(value), machine),
                other => panic!("unsupported test operation {other:?}"),
            }
        }
        panic!("test flow did not terminate");
    }
}
