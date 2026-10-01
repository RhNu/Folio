use super::*;

fn payload(member: WireMember) -> Vec<u8> {
    rmp_serde::to_vec(&Bundle(
        SCHEMA_VERSION,
        "papyrus-skyrim".into(),
        "fixture".into(),
        None,
        vec![WireScript(
            "Base".into(),
            None,
            false,
            vec![],
            vec![],
            vec![member],
            vec![],
            None,
            None,
        )],
    ))
    .unwrap()
}

#[test]
fn rejects_unknown_schema_tags_and_irrelevant_facts() {
    for (kind, attrs, access, parameter_tag, literal) in [
        (99, 0, 0, 0, None),
        (0, 4, 0, 0, None),
        (0, 0, 4, 0, None),
        (0, 0, 0, 99, None),
        (0, 0, 0, 2, Some("1")),
        (0, 0, 0, 1, None),
    ] {
        let member = WireMember(
            "Read".into(),
            kind,
            vec![],
            None,
            attrs,
            access,
            None,
            vec![WireParameter(
                "n".into(),
                "Int".into(),
                parameter_tag,
                literal.map(str::to_owned),
            )],
            None,
        );
        assert!(decode(&payload(member), SCHEMA_VERSION).is_err());
    }
    let property = WireMember(
        "Value".into(),
        3,
        vec![],
        Some("Int".into()),
        0,
        99,
        None,
        vec![],
        None,
    );
    assert!(decode(&payload(property), SCHEMA_VERSION).is_err());
}

#[test]
fn rejects_incorrect_fixed_array_lengths_and_tail_values() {
    // The schema contract requires all five bundle fields, including nil digest.
    let short = rmp_serde::to_vec(&(1u32, "papyrus-skyrim", "fixture")).unwrap();
    assert!(decode(&short, 1).is_err());
    let long = rmp_serde::to_vec(&(
        1u32,
        "papyrus-skyrim",
        "fixture",
        None::<String>,
        Vec::<WireScript>::new(),
        0u8,
    ))
    .unwrap();
    assert!(decode(&long, 1).is_err());
    let mut tail = rmp_serde::to_vec(&(
        1u32,
        "papyrus-skyrim",
        "fixture",
        None::<String>,
        Vec::<WireScript>::new(),
    ))
    .unwrap();
    tail.push(0xc0);
    assert!(decode(&tail, 1).is_err());
}
