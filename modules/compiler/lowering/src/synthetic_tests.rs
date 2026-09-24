use super::*;
use folio_hir::{Binding, Body, NameRef, ParameterFact};
use folio_source::{FileId, TextRange};
use std::collections::BTreeMap;

fn at() -> SourceSpan {
    SourceSpan {
        file: FileId(4),
        range: TextRange { start: 12, end: 18 },
    }
}

fn symbol(name: &str) -> Symbol {
    Symbol::Member {
        script: "Probe".into(),
        name: name.into(),
    }
}

fn name(text: &str) -> NameRef {
    NameRef {
        text: text.into(),
        span: at(),
    }
}

fn expression(ty: Type, kind: ExpressionKind) -> ExpressionFact {
    ExpressionFact {
        span: at(),
        ty,
        binding: None,
        conversion: None,
        kind,
    }
}

fn integer(value: i32) -> ExpressionFact {
    expression(Type::Int, ExpressionKind::Literal(value.to_string()))
}

fn boolean(value: bool) -> ExpressionFact {
    expression(Type::Bool, ExpressionKind::Literal(value.to_string()))
}

fn reference(text: &str, ty: Type) -> ExpressionFact {
    let mut result = expression(ty, ExpressionKind::Reference(name(text)));
    result.binding = Some(Binding {
        name: name(text),
        symbol: symbol(text),
        definition: None,
    });
    result
}

fn call(
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

fn callable(text: &str, ty: Type, native: bool) -> MemberFact {
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

fn program(statements: Vec<Statement>, return_type: Type) -> folio_hir::Script {
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

fn returns(value: ExpressionFact) -> folio_hir::Script {
    let ty = value.ty.clone();
    program(
        vec![Statement::Return {
            span: at(),
            value: Some(value),
        }],
        ty,
    )
}

// This deliberately tiny evaluator implements only the operations needed by
// these semantic examples. Calls have specified test effects; no Papyrus parser,
// PEX codec or production expression evaluator participates in the oracle.
struct Machine {
    slots: BTreeMap<String, Value>,
    ticks: i32,
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

    fn run(script: &Script) -> (Value, Self) {
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

fn lower(source: &folio_hir::Script) -> Script {
    lower_script(source, TargetProfile::skyrim_se(), &[]).unwrap()
}

#[test]
fn evaluates_named_arguments_in_source_order_then_places_them_in_parameter_order() {
    let tick = || call("Tick", Type::Int, vec![], vec![], vec![]);
    let mut source = returns(call(
        "Combine",
        Type::Int,
        vec![tick(), tick()],
        vec![1, 0],
        vec![None, None],
    ));
    source.members.extend([
        callable("Tick", Type::Int, true),
        callable("Combine", Type::Int, true),
    ]);
    let (value, machine) = Machine::run(&lower(&source));
    assert_eq!(value, Value::Int(21));
    assert_eq!(machine.ticks, 2);
}

#[test]
fn passes_stable_call_values_without_copy_instructions() {
    let mut source = returns(call(
        "Combine",
        Type::Int,
        vec![integer(3), integer(2)],
        vec![0, 1],
        vec![None, None],
    ));
    source.members.push(callable("Combine", Type::Int, true));
    let lowered = lower(&source);
    let function = &lowered.functions[0];
    assert!(
        !function
            .instructions
            .iter()
            .any(|item| matches!(item.op, Op::Assign(_, _)))
    );
    assert_eq!(Machine::run(&lowered).0, Value::Int(32));
}

#[test]
fn consumes_generated_binary_operands_without_extra_copies() {
    let mut source = returns(expression(
        Type::Int,
        ExpressionKind::Binary {
            operator: "+".into(),
            left: Box::new(call("Tick", Type::Int, vec![], vec![], vec![])),
            right: Box::new(call("Tick", Type::Int, vec![], vec![], vec![])),
        },
    ));
    source.members.push(callable("Tick", Type::Int, true));
    let lowered = lower(&source);
    assert_eq!(Machine::run(&lowered).0, Value::Int(3));
    assert!(
        !lowered.functions[0]
            .instructions
            .iter()
            .any(|item| matches!(item.op, Op::Assign(_, _)))
    );
}

#[test]
fn captures_a_field_before_a_later_argument_mutates_it() {
    let mut source = returns(call(
        "Combine",
        Type::Int,
        vec![
            reference("x", Type::Int),
            call("SetX", Type::Int, vec![], vec![], vec![]),
        ],
        vec![0, 1],
        vec![None, None],
    ));
    with_field(&mut source);
    source.members.push(callable("Combine", Type::Int, true));
    let lowered = lower(&source);
    assert_eq!(Machine::run(&lowered).0, Value::Int(35));
    assert!(
        lowered.functions[0]
            .instructions
            .iter()
            .any(|item| matches!(&item.op, Op::Assign(_, Value::Identifier(name)) if name == "x"))
    );
}

#[test]
fn omits_unreachable_return_after_both_branches_return() {
    let source = program(
        vec![Statement::If {
            span: at(),
            condition: boolean(true),
            then_branch: vec![Statement::Return {
                span: at(),
                value: Some(integer(1)),
            }],
            else_if: vec![],
            else_branch: vec![Statement::Return {
                span: at(),
                value: Some(integer(2)),
            }],
        }],
        Type::Int,
    );
    let lowered = lower(&source);
    assert_eq!(
        lowered.functions[0]
            .instructions
            .iter()
            .filter(|item| matches!(item.op, Op::Return(_)))
            .count(),
        2
    );
    assert_eq!(Machine::run(&lowered).0, Value::Int(1));
}

#[test]
fn supplies_declared_default_without_evaluating_another_expression() {
    let mut source = returns(call(
        "Combine",
        Type::Int,
        vec![integer(3)],
        vec![0],
        vec![None, Some((Type::Int, "2".into()))],
    ));
    source.members.push(callable("Combine", Type::Int, true));
    let (value, machine) = Machine::run(&lower(&source));
    assert_eq!(value, Value::Int(32));
    assert_eq!(machine.ticks, 0);
}

#[test]
fn supplies_compatibility_default_at_the_omitted_call_slot() {
    let mut source = returns(call(
        "Combine",
        Type::Int,
        vec![integer(2)],
        vec![1],
        vec![Some((Type::Int, "0".into())), None],
    ));
    source.members.push(callable("Combine", Type::Int, true));
    let (value, machine) = Machine::run(&lower(&source));
    assert_eq!(value, Value::Int(2));
    assert_eq!(machine.ticks, 0);
}

#[test]
fn string_addition_casts_numeric_operand_before_concat() {
    for (left, right) in [
        (
            expression(Type::String, ExpressionKind::Literal("\"n=\"".into())),
            integer(7),
        ),
        (
            expression(Type::Float, ExpressionKind::Literal("1.5".into())),
            expression(Type::String, ExpressionKind::Literal("\"x\"".into())),
        ),
    ] {
        let mut left = left;
        let mut right = right;
        for operand in [&mut left, &mut right] {
            if matches!(operand.ty, Type::Int | Type::Float) {
                operand.conversion = Some(Type::String);
            }
        }
        let source = returns(expression(
            Type::String,
            ExpressionKind::Binary {
                operator: "+".into(),
                left: Box::new(left),
                right: Box::new(right),
            },
        ));
        let lowered = lower(&source);
        let instructions = &lowered
            .functions
            .iter()
            .find(|item| item.name == "Use")
            .unwrap()
            .instructions;
        let cast = instructions
            .iter()
            .position(|item| matches!(item.op, Op::Cast(_, _)))
            .unwrap();
        let concat = instructions
            .iter()
            .position(|item| {
                matches!(
                    item.op,
                    Op::Binary {
                        operator: BinaryOp::AddString,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(cast < concat);
    }
}

#[test]
fn short_circuit_preserves_result_and_skips_only_the_unselected_rhs() {
    for (operator, left, expected, calls) in [
        ("&&", false, false, 0),
        ("||", true, true, 0),
        ("||", false, true, 1),
    ] {
        let mut source = returns(expression(
            Type::Bool,
            ExpressionKind::Binary {
                operator: operator.into(),
                left: Box::new(boolean(left)),
                right: Box::new(call("Mark", Type::Bool, vec![], vec![], vec![])),
            },
        ));
        source.members.push(callable("Mark", Type::Bool, true));
        let (value, machine) = Machine::run(&lower(&source));
        assert_eq!(value, Value::Bool(expected));
        assert_eq!(machine.ticks, calls);
    }
}

#[test]
fn truthy_condition_casts_before_branching() {
    let mut condition = integer(1);
    condition.conversion = Some(Type::Bool);
    let source = program(
        vec![
            Statement::If {
                span: at(),
                condition,
                then_branch: vec![Statement::Return {
                    span: at(),
                    value: Some(boolean(true)),
                }],
                else_if: vec![],
                else_branch: vec![],
            },
            Statement::Return {
                span: at(),
                value: Some(boolean(false)),
            },
        ],
        Type::Bool,
    );
    let lowered = lower(&source);
    let instructions = &lowered
        .functions
        .iter()
        .find(|item| item.name == "Use")
        .unwrap()
        .instructions;
    assert!(
        instructions
            .windows(2)
            .any(|pair| matches!(pair[0].op, Op::Cast(_, Value::Int(1)))
                && matches!(pair[1].op, Op::JumpIf { .. }))
    );
}

fn with_field(source: &mut folio_hir::Script) {
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

#[test]
fn preserves_left_value_before_a_rhs_call_changes_its_storage() {
    let mut source = returns(expression(
        Type::Int,
        ExpressionKind::Binary {
            operator: "+".into(),
            left: Box::new(reference("x", Type::Int)),
            right: Box::new(call("SetX", Type::Int, vec![], vec![], vec![])),
        },
    ));
    with_field(&mut source);
    let (value, machine) = Machine::run(&lower(&source));
    assert_eq!(value, Value::Int(8));
    assert_eq!(machine.slots["x"], Value::Int(9));
}

#[test]
fn compound_assignment_uses_old_value_before_rhs_side_effect() {
    let mut source = program(
        vec![
            Statement::Assignment {
                span: at(),
                target: Box::new(reference("x", Type::Int)),
                value: Box::new(call("SetX", Type::Int, vec![], vec![], vec![])),
                operator: "+=".into(),
            },
            Statement::Return {
                span: at(),
                value: Some(reference("x", Type::Int)),
            },
        ],
        Type::Int,
    );
    with_field(&mut source);
    let (value, _) = Machine::run(&lower(&source));
    assert_eq!(value, Value::Int(8));
}

#[test]
fn string_compound_assignment_casts_numeric_rhs() {
    let mut numeric = integer(7);
    numeric.conversion = Some(Type::String);
    let mut source = program(
        vec![
            Statement::Assignment {
                span: at(),
                target: Box::new(reference("label", Type::String)),
                value: Box::new(numeric),
                operator: "+=".into(),
            },
            Statement::Return {
                span: at(),
                value: Some(reference("label", Type::String)),
            },
        ],
        Type::String,
    );
    source.members.push(MemberFact {
        symbol: symbol("label"),
        kind: MemberKind::Variable,
        ty: Type::String,
        parameters: vec![],
        flags: vec![],
        initial_literal: Some("\"n=\"".into()),
        span: at(),
    });
    let lowered = lower(&source);
    let instructions = &lowered
        .functions
        .iter()
        .find(|item| item.name == "Use")
        .unwrap()
        .instructions;
    let cast = instructions
        .iter()
        .position(|item| matches!(item.op, Op::Cast(_, _)))
        .unwrap();
    let concat = instructions
        .iter()
        .position(|item| {
            matches!(
                item.op,
                Op::Binary {
                    operator: BinaryOp::AddString,
                    ..
                }
            )
        })
        .unwrap();
    assert!(cast < concat);
}

#[test]
fn inequality_inverts_comparison() {
    for (right, expected) in [(4, false), (5, true)] {
        let source = returns(expression(
            Type::Bool,
            ExpressionKind::Binary {
                operator: "!=".into(),
                left: Box::new(integer(4)),
                right: Box::new(integer(right)),
            },
        ));
        assert_eq!(Machine::run(&lower(&source)).0, Value::Bool(expected));
    }
}

#[test]
fn rejects_array_allocation_outside_skyrim_contract_at_original_use() {
    for size in [0, 129] {
        let source = returns(expression(
            Type::Array(Box::new(Type::Int)),
            ExpressionKind::NewArray {
                element_type: Type::Int,
                length: Box::new(integer(size)),
            },
        ));
        assert!(
            lower_script(&source, TargetProfile::skyrim_se(), &[])
                .unwrap_err()
                .iter()
                .any(|issue| issue.code == "target.array-size" && issue.primary == Some(at()))
        );
    }
    let mut length = reference("size", Type::Int);
    length.binding.as_mut().unwrap().symbol = Symbol::Parameter {
        owner: Box::new(symbol("Use")),
        name: "size".into(),
    };
    let mut source = returns(expression(
        Type::Array(Box::new(Type::Int)),
        ExpressionKind::NewArray {
            element_type: Type::Int,
            length: Box::new(length),
        },
    ));
    source.members[0].parameters.push(ParameterFact {
        name: "size".into(),
        ty: Type::Int,
        default_literal: None,
        span: at(),
    });
    source.bodies[0].parameters = source.members[0].parameters.clone();
    assert!(
        lower_script(&source, TargetProfile::skyrim_se(), &[])
            .unwrap_err()
            .iter()
            .any(|issue| issue.code == "target.array-size" && issue.primary == Some(at()))
    );
}

#[test]
fn decodes_escaped_backslashes_once() {
    assert_eq!(decode_string("\\\\n"), Some("\\n".into()));
    assert_eq!(decode_string("a\\n\\\"b"), Some("a\n\"b".into()));
    assert_eq!(decode_string("a\\q"), None);
}

#[test]
fn rejects_nonvoid_fallthrough_and_incompatible_state_abi() {
    let source = program(vec![], Type::Int);
    assert!(
        lower_script(&source, TargetProfile::skyrim_se(), &[])
            .unwrap_err()
            .iter()
            .any(|issue| issue.code == "target.missing-return")
    );

    let mut source = returns(integer(1));
    source.states.push(folio_hir::StateFact {
        name: "Ready".into(),
        auto: false,
        span: at(),
    });
    let mut event = callable("OnBeginState", Type::Void, false);
    event.symbol = Symbol::StateMember {
        script: "Probe".into(),
        state: "Ready".into(),
        name: "OnBeginState".into(),
    };
    event.kind = MemberKind::Function {
        event: true,
        global: false,
        native: false,
    };
    event.parameters.push(ParameterFact {
        name: "oldState".into(),
        ty: Type::String,
        default_literal: None,
        span: at(),
    });
    source.bodies.push(Body {
        symbol: event.symbol.clone(),
        return_type: Type::Void,
        parameters: vec![],
        statements: vec![],
    });
    source.members.push(event);
    assert!(
        lower_script(&source, TargetProfile::skyrim_se(), &[])
            .unwrap_err()
            .iter()
            .any(|issue| issue.code == "target.state-event-signature")
    );

    let mut source = returns(integer(1));
    source.states.push(folio_hir::StateFact {
        name: "Ready".into(),
        auto: false,
        span: at(),
    });
    let mut variant = callable("Use", Type::Bool, false);
    variant.symbol = Symbol::StateMember {
        script: "Probe".into(),
        state: "Ready".into(),
        name: "Use".into(),
    };
    source.bodies.push(Body {
        symbol: variant.symbol.clone(),
        return_type: Type::Bool,
        parameters: vec![],
        statements: vec![Statement::Return {
            span: at(),
            value: Some(boolean(true)),
        }],
    });
    source.members.push(variant);
    assert!(
        lower_script(&source, TargetProfile::skyrim_se(), &[])
            .unwrap_err()
            .iter()
            .any(|issue| issue.code == "target.override-signature")
    );
}

#[test]
fn accessor_parameters_belong_to_their_property_only() {
    let mut source = program(vec![], Type::Void);
    for (property, ty, parameter_name) in [
        ("First", Type::Int, "firstValue"),
        ("Second", Type::String, "secondValue"),
    ] {
        source.members.push(MemberFact {
            symbol: symbol(property),
            kind: MemberKind::Property {
                auto: false,
                read_only: false,
            },
            ty: ty.clone(),
            parameters: vec![],
            flags: vec![],
            initial_literal: None,
            span: at(),
        });
        let accessor = Symbol::PropertyAccessor {
            script: "Probe".into(),
            property: property.into(),
            name: "Set".into(),
        };
        let parameter = ParameterFact {
            name: parameter_name.into(),
            ty: ty.clone(),
            default_literal: None,
            span: at(),
        };
        source.declarations.push(folio_hir::DeclarationFact {
            symbol: Symbol::Parameter {
                owner: Box::new(accessor.clone()),
                name: parameter_name.into(),
            },
            ty: ty.clone(),
            span: at(),
        });
        source.bodies.push(Body {
            symbol: accessor,
            return_type: Type::Void,
            parameters: vec![parameter],
            statements: vec![],
        });
    }
    let mir = lower_script(&source, TargetProfile::skyrim_se(), &[]).unwrap();
    let first = mir
        .properties
        .iter()
        .find(|property| property.name == "First")
        .unwrap()
        .setter
        .as_ref()
        .unwrap();
    let second = mir
        .properties
        .iter()
        .find(|property| property.name == "Second")
        .unwrap()
        .setter
        .as_ref()
        .unwrap();
    assert_eq!(
        first.parameters,
        vec![Local {
            name: "firstValue".into(),
            ty: "Int".into()
        }]
    );
    assert_eq!(
        second.parameters,
        vec![Local {
            name: "secondValue".into(),
            ty: "String".into()
        }]
    );
}
