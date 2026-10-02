use super::*;
use folio_hir::{Binding, Body, NameRef, StateFact};
use folio_source::{FileId, TextRange};

mod integers;

#[test]
fn external_literal_trivia_is_normalized_before_target_values() {
    assert_eq!(
        literal("- ;/ note /; 0x10", &Type::Int),
        Some(Value::Int(-16))
    );
    assert_eq!(literal("- 1.5", &Type::Float), Some(Value::Float(-1.5)));
    assert_eq!(literal(" True ", &Type::Bool), Some(Value::Bool(true)));
    assert_eq!(
        literal(" \"a b\" ", &Type::String),
        Some(Value::String("a b".into()))
    );
}

fn span() -> SourceSpan {
    SourceSpan {
        file: FileId(1),
        range: TextRange { start: 0, end: 1 },
    }
}

fn name(text: &str) -> NameRef {
    NameRef {
        text: text.into(),
        span: span(),
    }
}

fn symbol(name: &str) -> Symbol {
    Symbol::Member {
        script: "Example".into(),
        name: name.into(),
    }
}

fn property(read_only: bool) -> MemberFact {
    MemberFact {
        symbol: symbol("Amount"),
        kind: MemberKind::Property {
            auto: true,
            read_only,
        },
        ty: Type::Int,
        parameters: vec![],
        flags: vec![if read_only { "autoreadonly" } else { "auto" }.into()],
        initial_literal: Some("0x10".into()),
        span: span(),
    }
}

fn script(members: Vec<MemberFact>) -> folio_hir::Script {
    folio_hir::Script {
        name: Some(name("Example")),
        members,
        ..Default::default()
    }
}

fn expression(ty: Type, kind: ExpressionKind) -> ExpressionFact {
    ExpressionFact {
        span: span(),
        ty,
        kind,
        binding: None,
        conversion: None,
    }
}

#[test]
fn conditional_and_custom_flags_route_to_declared_storage_and_property_scopes() {
    let mut member = property(false);
    member.flags.extend([
        "conditional".into(),
        "hidden".into(),
        "Stored".into(),
        "Listed".into(),
    ]);
    let flags = [
        UserFlag {
            name: "Stored".into(),
            bit: Some(7),
            scopes: vec![FlagScope::Variable],
        },
        UserFlag {
            name: "Listed".into(),
            bit: Some(12),
            scopes: vec![FlagScope::Property],
        },
    ];
    let source = script(vec![member]);
    let output = lower_script(&source, TargetProfile::skyrim_se(), &flags).unwrap();
    assert_eq!(output.variables[0].flags, (1 << 1) | (1 << 7));
    assert_eq!(output.properties[0].flags, 1 | (1 << 12));
    assert_eq!(output.variables[0].initial, Value::Int(16));
}

#[test]
fn read_only_property_has_a_literal_getter_without_persistent_storage() {
    let mut source = script(vec![property(true)]);
    let read = ExpressionFact {
        binding: Some(Binding {
            name: name("Amount"),
            symbol: symbol("Amount"),
            definition: None,
        }),
        ..expression(Type::Int, ExpressionKind::Reference(name("Amount")))
    };
    source.members.push(MemberFact {
        symbol: symbol("Read"),
        kind: MemberKind::Function {
            event: false,
            global: false,
            native: false,
        },
        ty: Type::Int,
        parameters: vec![],
        flags: vec![],
        initial_literal: None,
        span: span(),
    });
    source.bodies.push(Body {
        symbol: symbol("Read"),
        return_type: Type::Int,
        parameters: vec![],
        statements: vec![Statement::Return {
            span: span(),
            value: Some(read),
        }],
    });
    let output = lower_script(&source, TargetProfile::skyrim_se(), &[]).unwrap();
    assert!(output.variables.is_empty());
    assert!(output.properties[0].auto_var.is_none());
    assert_eq!(
        output.properties[0].getter.as_ref().unwrap().instructions[0].op,
        Op::Return(Value::Int(16))
    );
    assert_eq!(
        output.functions[0].instructions[0].op,
        Op::Return(Value::Int(16))
    );
}

#[test]
fn read_only_property_rejects_flags_that_require_absent_storage() {
    for flag in ["conditional", "Stored"] {
        let mut member = property(true);
        member.flags.push(flag.into());
        let flags = [UserFlag {
            name: "Stored".into(),
            bit: Some(7),
            scopes: vec![FlagScope::Variable],
        }];
        assert!(
            lower_script(&script(vec![member]), TargetProfile::skyrim_se(), &flags)
                .unwrap_err()
                .iter()
                .any(|error| error.code == "target.read-only-storage-flag")
        );
    }
    let mut member = property(true);
    member.flags.push("Listed".into());
    let flags = [UserFlag {
        name: "Listed".into(),
        bit: Some(12),
        scopes: vec![FlagScope::Property, FlagScope::Variable],
    }];
    let output = lower_script(&script(vec![member]), TargetProfile::skyrim_se(), &flags).unwrap();
    assert!(output.variables.is_empty());
    assert_eq!(output.properties[0].flags, 1 << 12);
}

#[test]
fn private_fields_preserve_owners_without_conflicting_with_child_storage() {
    let mut own = property(false);
    own.symbol = symbol("x");
    own.kind = MemberKind::Variable;
    own.flags.clear();
    let mut source = script(vec![own.clone()]);
    for owner in ["Parent", "Grandparent"] {
        let mut inherited = own.clone();
        inherited.symbol = Symbol::Member {
            script: owner.into(),
            name: "x".into(),
        };
        source.external_members.push(inherited);
    }
    let output = lower_script(&source, TargetProfile::skyrim_se(), &[]).unwrap();
    assert_eq!(output.variables.len(), 1);
    assert_eq!(output.external_slots.len(), 2);
    assert_eq!(output.external_slots[0].owner, "Parent");
    assert_eq!(output.external_slots[1].owner, "Grandparent");
}

#[test]
fn state_capacity_counts_the_empty_state() {
    let mut source = script(vec![]);
    source.states = (0..127)
        .map(|index| StateFact {
            name: format!("S{index}"),
            auto: false,
            span: span(),
        })
        .collect();
    assert!(lower_script(&source, TargetProfile::skyrim_se(), &[]).is_ok());
    source.states.push(StateFact {
        name: "TooMany".into(),
        auto: false,
        span: span(),
    });
    assert!(
        lower_script(&source, TargetProfile::skyrim_se(), &[])
            .unwrap_err()
            .iter()
            .any(|error| error.code == "target.state-capacity")
    );
}

#[test]
fn integer_decoder_is_used_for_literals_and_signed_decimal_minimum() {
    assert_eq!(literal("0x10", &Type::Int), Some(Value::Int(16)));
    assert_eq!(
        literal("-2147483648", &Type::Int),
        Some(Value::Int(i32::MIN))
    );
    assert_eq!(literal("0xffffffff", &Type::Int), Some(Value::Int(-1)));
    assert_eq!(literal("0x10", &Type::Float), Some(Value::Float(16.0)));
}

fn function_program(statements: Vec<Statement>) -> folio_hir::Script {
    let mut input = script(vec![MemberFact {
        symbol: symbol("Run"),
        kind: MemberKind::Function {
            event: false,
            global: false,
            native: false,
        },
        ty: Type::Void,
        parameters: vec![],
        flags: vec![],
        initial_literal: None,
        span: span(),
    }]);
    input.bodies.push(Body {
        symbol: symbol("Run"),
        return_type: Type::Void,
        parameters: vec![],
        statements,
    });
    input
}

#[test]
fn sibling_local_identities_have_independent_slots_and_uninitialized_loop_locals_are_not_reset() {
    let declaration = |identity| folio_hir::DeclarationFact {
        symbol: Symbol::Local {
            owner: Box::new(symbol("Run")),
            name: "x".into(),
            identity,
        },
        ty: Type::Int,
        span: span(),
    };
    let condition = || expression(Type::Bool, ExpressionKind::Literal("false".into()));
    let input = function_program(vec![
        Statement::If {
            span: span(),
            condition: condition(),
            then_branch: vec![Statement::Variable {
                declaration: declaration(1),
                value: Some(expression(Type::Int, ExpressionKind::Literal("1".into()))),
            }],
            else_if: vec![],
            else_branch: vec![Statement::Variable {
                declaration: declaration(2),
                value: Some(expression(Type::Int, ExpressionKind::Literal("2".into()))),
            }],
        },
        Statement::While {
            span: span(),
            condition: condition(),
            body: vec![Statement::Variable {
                declaration: declaration(3),
                value: None,
            }],
        },
    ]);
    let output = lower_script(&input, TargetProfile::skyrim_se(), &[]).unwrap();
    let function = &output.functions[0];
    assert_eq!(function.locals.len(), 3);
    let writes = function
        .instructions
        .iter()
        .filter_map(|instruction| match &instruction.op {
            Op::Assign(Value::Identifier(destination), value) => Some((destination, value)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 2);
    assert_ne!(writes[0].0, writes[1].0);
    assert_eq!(writes[0].1, &Value::Int(1));
    assert_eq!(writes[1].1, &Value::Int(2));
    assert!(
        writes
            .iter()
            .all(|(destination, _)| **destination != function.locals[2].name)
    );
}

#[test]
fn legal_parent_call_remains_callparent_and_parent_values_are_rejected() {
    let mut owner = expression(
        Type::Script("Base".into()),
        ExpressionKind::Reference(name("Parent")),
    );
    owner.binding = Some(Binding {
        name: name("Parent"),
        symbol: Symbol::ParentReceiver {
            script: "Base".into(),
        },
        definition: None,
    });
    let mut callee = expression(
        Type::Void,
        ExpressionKind::Member {
            owner: Box::new(owner.clone()),
            name: name("F"),
        },
    );
    callee.binding = Some(Binding {
        name: name("F"),
        symbol: Symbol::Member {
            script: "Base".into(),
            name: "F".into(),
        },
        definition: None,
    });
    let call = expression(
        Type::Void,
        ExpressionKind::Call {
            callee: Box::new(callee),
            arguments: vec![],
            argument_ordinals: vec![],
            parameter_defaults: vec![],
            is_global: false,
        },
    );
    let input = function_program(vec![Statement::Expression(call)]);
    let output = lower_script(&input, TargetProfile::skyrim_se(), &[]).unwrap();
    assert!(
        matches!(&output.functions[0].instructions[0].op, Op::CallParent { name, .. } if name == "F")
    );
    let invalid = function_program(vec![Statement::Expression(owner)]);
    assert!(
        lower_script(&invalid, TargetProfile::skyrim_se(), &[])
            .unwrap_err()
            .iter()
            .any(|error| error.code == "lowering.parent-value")
    );
}

#[test]
fn hexadecimal_defaults_arrays_and_signed_minimum_share_the_literal_decoder() {
    let constructor = expression(
        Type::Array(Box::new(Type::Int)),
        ExpressionKind::NewArray {
            element_type: Type::Int,
            length: Box::new(expression(
                Type::Int,
                ExpressionKind::Literal("0x10".into()),
            )),
        },
    );
    let minimum = expression(
        Type::Int,
        ExpressionKind::Unary {
            operator: "-".into(),
            operand: Box::new(expression(
                Type::Int,
                ExpressionKind::Literal("2147483648".into()),
            )),
        },
    );
    let mut callee = expression(Type::Void, ExpressionKind::Reference(name("WithDefault")));
    callee.binding = Some(Binding {
        name: name("WithDefault"),
        symbol: symbol("WithDefault"),
        definition: None,
    });
    let call = expression(
        Type::Void,
        ExpressionKind::Call {
            callee: Box::new(callee),
            arguments: vec![],
            argument_ordinals: vec![],
            parameter_defaults: vec![Some((Type::Int, "0x10".into()))],
            is_global: false,
        },
    );
    let mut input = function_program(vec![
        Statement::Expression(constructor),
        Statement::Expression(minimum),
        Statement::Expression(call),
    ]);
    input.members.push(MemberFact {
        symbol: symbol("WithDefault"),
        kind: MemberKind::Function {
            event: false,
            global: false,
            native: true,
        },
        ty: Type::Void,
        parameters: vec![folio_hir::ParameterFact {
            name: "value".into(),
            ty: Type::Int,
            default_literal: Some("0x10".into()),
            span: span(),
        }],
        flags: vec![],
        initial_literal: None,
        span: span(),
    });
    let output = lower_script(&input, TargetProfile::skyrim_se(), &[]).unwrap();
    let instructions = &output.functions[0].instructions;
    assert!(instructions.iter().any(|instruction| matches!(
        instruction.op,
        Op::ArrayCreate {
            length: Value::Int(16),
            ..
        }
    )));
    assert!(instructions.iter().any(|instruction| matches!(&instruction.op, Op::CallMethod { args, .. } if args == &[Value::Int(16)])));
    // The signed minimum is a literal value, with no overflowing positive intermediate.
    assert!(!instructions.iter().any(|instruction| matches!(
        instruction.op,
        Op::Unary {
            operator: UnaryOp::NegInt,
            ..
        }
    )));
}
