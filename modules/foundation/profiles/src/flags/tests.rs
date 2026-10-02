use super::*;

#[test]
fn explicit_bits_survive_automatic_allocation_and_input_order() {
    let fixed = UserFlag {
        name: "Fixed".into(),
        bit: Some(7),
        scopes: vec![FlagScope::Function],
    };
    let first = resolve_user_flags(&["Zeta".into(), fixed.clone(), "Alpha".into()], 31).unwrap();
    let second = resolve_user_flags(&["Alpha".into(), "Zeta".into(), fixed], 31).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first
            .iter()
            .map(|flag| (flag.name.as_str(), flag.bit))
            .collect::<Vec<_>>(),
        vec![("Alpha", Some(2)), ("Fixed", Some(7)), ("Zeta", Some(3))]
    );
}

#[test]
fn rejects_reserved_names_duplicate_bits_and_invalid_scopes() {
    assert!(resolve_user_flags(&["Native".into()], 31).is_err());
    let flag = UserFlag {
        name: "Tag".into(),
        bit: Some(5),
        scopes: vec![FlagScope::Script],
    };
    let other = UserFlag {
        name: "Other".into(),
        ..flag.clone()
    };
    assert!(resolve_user_flags(&[flag.clone(), other], 31).is_err());
    assert!(
        resolve_user_flags(
            &[UserFlag {
                bit: Some(32),
                ..flag.clone()
            }],
            31
        )
        .is_err()
    );
    assert!(
        resolve_user_flags(
            &[UserFlag {
                scopes: vec![],
                ..flag
            }],
            31
        )
        .is_err()
    );
}

#[test]
fn limits_automatic_flags_to_available_bits() {
    assert!(resolve_user_flags(&["One".into()], 2).is_ok());
    assert!(resolve_user_flags(&["One".into(), "Two".into()], 2).is_err());
}
