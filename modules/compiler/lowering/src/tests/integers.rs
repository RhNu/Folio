use super::*;

#[test]
fn full_width_hex_uses_signed_values_in_constants_and_float_widening() {
    for (text, value, widened) in [
        ("0x80000000", i32::MIN, f32::from_bits(0xCF00_0000)),
        ("0xFFFFFFFF", -1, -1.0),
    ] {
        assert_eq!(literal(text, &Type::Int), Some(Value::Int(value)));
        assert_eq!(literal(text, &Type::Float), Some(Value::Float(widened)));
        let mut member = property(true);
        member.initial_literal = Some(text.into());
        let output = lower_script(&script(vec![member]), TargetProfile::skyrim_se(), &[]).unwrap();
        assert_eq!(
            output.properties[0].getter.as_ref().unwrap().instructions[0].op,
            Op::Return(Value::Int(value))
        );
    }
}

#[test]
fn omitted_mask_default_becomes_signed_call_operand() {
    let mut callee = expression(Type::Void, ExpressionKind::Reference(name("Take")));
    callee.binding = Some(Binding {
        name: name("Take"),
        symbol: symbol("Take"),
        definition: None,
    });
    let call = expression(
        Type::Void,
        ExpressionKind::Call {
            callee: Box::new(callee),
            arguments: vec![],
            argument_ordinals: vec![],
            parameter_defaults: vec![Some((Type::Int, "0xFFFFFFFF".into()))],
            is_global: false,
        },
    );
    let output = lower_script(
        &function_program(vec![Statement::Expression(call)]),
        TargetProfile::skyrim_se(),
        &[],
    )
    .unwrap();
    assert!(output.functions[0].instructions.iter().any(|instruction|
        matches!(&instruction.op, Op::CallMethod { args, .. } if args == &[Value::Int(-1)])
    ));
}

#[test]
fn signed_literal_overflow_does_not_fall_back_to_runtime_negation() {
    for text in ["0xFFFFFFFF", "0x80000001", "2147483649", "0x100000000"] {
        let value = expression(
            Type::Int,
            ExpressionKind::Unary {
                operator: "-".into(),
                operand: Box::new(expression(Type::Int, ExpressionKind::Literal(text.into()))),
            },
        );
        let errors = lower_script(
            &function_program(vec![Statement::Expression(value)]),
            TargetProfile::skyrim_se(),
            &[],
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.code == "target.invalid-literal"),
            "{text}: {errors:?}"
        );
    }
    let value = expression(
        Type::Int,
        ExpressionKind::Unary {
            operator: "-".into(),
            operand: Box::new(expression(
                Type::Int,
                ExpressionKind::Literal("0x80000000".into()),
            )),
        },
    );
    let mut input = function_program(vec![Statement::Return {
        value: Some(value),
        span: span(),
    }]);
    input.members[0].ty = Type::Int;
    input.bodies[0].return_type = Type::Int;
    let output = lower_script(&input, TargetProfile::skyrim_se(), &[]).unwrap();
    assert_eq!(
        output.functions[0].instructions[0].op,
        Op::Return(Value::Int(i32::MIN))
    );
}

#[test]
fn high_bit_masks_remain_invalid_array_lengths() {
    for text in ["0x80000000", "0xFFFFFFFF"] {
        let value = expression(
            Type::Array(Box::new(Type::Int)),
            ExpressionKind::NewArray {
                element_type: Type::Int,
                length: Box::new(expression(Type::Int, ExpressionKind::Literal(text.into()))),
            },
        );
        assert!(
            lower_script(
                &function_program(vec![Statement::Expression(value)]),
                TargetProfile::skyrim_se(),
                &[]
            )
            .is_err(),
            "{text}"
        );
    }
}
