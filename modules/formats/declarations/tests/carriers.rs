//! Public declaration carriers, validation, and semantic identity contracts.
#[path = "common/support.rs"]
mod common;
use common::sample;
use folio_format_declarations::*;

#[test]
fn both_carriers_preserve_unknown_and_known_facts() {
    let bundle = sample();
    for format in [DeclarationFormat::Json, DeclarationFormat::Binary] {
        let bytes = encode(&bundle, format).unwrap();
        let decoded = decode(&bytes).unwrap();
        let parameters = decoded.scripts[0].members[0].parameters();
        assert_eq!(parameters[0].default, ParameterDefault::Required);
        assert_eq!(parameters[1].default, ParameterDefault::Literal("7".into()));
        assert_eq!(parameters[2].default, ParameterDefault::Unknown);
        assert_eq!(
            decoded.scripts[0].members[0].kind(),
            MemberKind::UnknownCallable
        );
        assert!(decoded.scripts[0].members[1].is_read_only());
        assert_eq!(decoded, bundle);
    }
}

#[test]
fn rejects_unknown_fields_and_cross_callable_duplicates() {
    let mut value = serde_json::to_value(sample()).unwrap();
    value["scripts"][0]["members"][0]["is_auto"] = true.into();
    assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
    let mut bundle = sample();
    bundle.scripts[0].members.push(Member {
        name: "read".into(),
        documentation: None,
        flags: vec![],
        data: MemberData::Event {
            native: false,
            parameters: vec![],
        },
    });
    assert!(validate(&bundle).is_err());
    bundle.scripts[0].members.pop();
    bundle.scripts[0].members.push(Member {
        name: "read".into(),
        documentation: None,
        flags: vec![],
        data: MemberData::Property {
            ty: "Int".into(),
            access: PropertyAccess::Auto,
            initial_literal: None,
        },
    });
    assert!(validate(&bundle).is_ok());
}

#[test]
fn rejects_unsupported_schema() {
    let mut bundle = sample();
    bundle.schema = 3;
    assert!(matches!(
        validate(&bundle),
        Err(DecodeError::UnsupportedSchema(3))
    ));
}

#[test]
fn semantic_identity_ignores_origins_and_unordered_declarations() {
    let original = sample();
    let mut changed = original.clone();
    changed.origin.source = "ck/1.6.1170.0".into();
    changed.scripts[0].source = None;
    changed.scripts[0].members.reverse();
    assert_eq!(semantic_digest(&original), semantic_digest(&changed));
    let MemberData::UnknownCallable { parameters, .. } = &mut changed.scripts[0].members[1].data
    else {
        panic!()
    };
    parameters.swap(0, 1);
    assert_ne!(semantic_digest(&original), semantic_digest(&changed));
}

#[test]
fn rejects_nonportable_origins_and_sources() {
    let mut bundle = sample();
    bundle.origin.source = "C:/private/scripts".into();
    assert!(validate(&bundle).is_err());
    bundle.origin.source = "fixture".into();
    bundle.scripts[0].source.as_mut().unwrap().path = "../Base.psc".into();
    assert!(validate(&bundle).is_err());
}

#[test]
fn carriers_preserve_states_flags_and_each_member_variant() {
    let declaration = decode(br#"{"format":"folio-declarations","schema":2,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","flags":["hidden"],"imports":["Utility"],"members":[{"name":"Run","kind":"function","global":true,"native":true},{"name":"OnInit","kind":"event"},{"name":"count","kind":"variable","ty":"Int","initial_literal":"3"},{"name":"Value","kind":"property","ty":"Int","access":{"kind":"manual","readable":false,"writable":true}}],"states":[{"name":"Busy","auto":true,"members":[{"name":"Pulse","kind":"event","native":true}]}]}]}"#).unwrap();
    for format in [DeclarationFormat::Json, DeclarationFormat::Binary] {
        let actual = decode(&encode(&declaration, format).unwrap()).unwrap();
        assert!(actual.scripts[0].members[0].is_global());
        assert_eq!(actual.scripts[0].members[1].kind(), MemberKind::Event);
        assert_eq!(actual.scripts[0].members[2].initial_literal(), Some("3"));
        assert!(!actual.scripts[0].members[3].is_readable());
        assert!(actual.scripts[0].members[3].is_writable());
        assert!(actual.scripts[0].states[0].auto);
        assert!(actual.scripts[0].states[0].members[0].is_native());
        assert_eq!(actual, declaration);
    }
}
