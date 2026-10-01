//! Public crate behavior over in-memory inputs.
use folio_declaration_tools::{PexInput, extract_pex};
use folio_format_declarations::ParameterDefault;
use folio_format_pex::{PexFile, PexFunction};

use folio_format_declarations::{DeclarationFormat, MemberKind, decode, encode};
use folio_format_pex::{
    PexHeader, PexObject, PexParameter, PexProperty, PexState, PexValue, PexVariable,
};

#[test]
fn extracts_callable_signature_without_inventing_event_or_defaults() {
    let mut file = PexFile::new(PexHeader::skyrim(0, "Actor.psc", "", ""));
    let empty = file.intern("").unwrap();
    let actor = file.intern("Actor").unwrap();
    let get_value = file.intern("GetValue").unwrap();
    let int = file.intern("Int").unwrap();
    let count = file.intern("count").unwrap();
    let value = file.intern("Value").unwrap();
    let backing = file.intern("::Value_var").unwrap();
    file.objects.push(PexObject {
        name: actor,
        parent_class_name: empty,
        documentation_string: empty,
        user_flags: 0,
        auto_state_name: empty,
        variables: vec![PexVariable {
            name: backing,
            type_name: int,
            user_flags: 0,
            default_value: PexValue::None,
        }],
        properties: vec![PexProperty {
            name: value,
            type_name: int,
            documentation_string: empty,
            user_flags: 0,
            is_readable: true,
            is_writable: true,
            is_auto: true,
            auto_var: Some(backing),
            read_function: None,
            write_function: None,
        }],
        states: vec![PexState {
            name: empty,
            functions: vec![PexFunction {
                name: get_value,
                return_type_name: int,
                documentation_string: empty,
                user_flags: 0,
                is_global: false,
                is_native: true,
                parameters: vec![PexParameter {
                    name: count,
                    type_name: int,
                }],
                locals: vec![],
                instructions: vec![],
            }],
        }],
    });
    let bytes = file.write_to_vec().unwrap();
    let extracted = extract_pex(
        "binary-mod",
        &[PexInput {
            path: "Actor.pex",
            bytes: &bytes,
        }],
    )
    .unwrap();
    let member = extracted.bundle.scripts[0]
        .members
        .iter()
        .find(|item| item.name == "GetValue")
        .unwrap();
    assert_eq!(extracted.bundle.scripts[0].name, "Actor");
    assert_eq!(extracted.paths, ["Actor.pex"]);
    assert_eq!(member.kind(), MemberKind::UnknownCallable);
    assert_eq!(member.ty(), Some("Int"));
    assert_eq!(member.parameters()[0].default, ParameterDefault::Unknown);
    assert_eq!(member.parameters()[0].ty, "Int");
    for format in [DeclarationFormat::Json, DeclarationFormat::Binary] {
        let persisted = decode(&encode(&extracted.bundle, format).unwrap()).unwrap();
        assert_eq!(
            persisted.scripts[0]
                .members
                .iter()
                .find(|item| item.name == "GetValue")
                .unwrap()
                .parameters()[0]
                .default,
            ParameterDefault::Unknown
        );
    }
    assert_eq!(
        extracted.bundle.scripts[0]
            .members
            .iter()
            .filter(|item| item.kind() == MemberKind::Property)
            .count(),
        1
    );
    assert!(
        extracted.bundle.scripts[0]
            .members
            .iter()
            .all(|item| item.kind() != MemberKind::Variable)
    );
}

#[test]
fn malformed_binary_reports_its_dependency_path() {
    let error = extract_pex(
        "binary-mod",
        &[PexInput {
            path: "Broken.pex",
            bytes: &[0, 1, 2],
        }],
    )
    .err()
    .unwrap();
    assert!(error.contains("Broken.pex"));
}
