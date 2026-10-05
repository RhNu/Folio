//! Public crate behavior over in-memory inputs.
use folio_declaration_tools::{PexInput, extract_pex};
use folio_format_declarations::{DeclarationFormat, MemberKind, ParameterDefault, decode, encode};
use folio_format_pex::{
    PexFile, PexFunction, PexHeader, PexObject, PexParameter, PexProperty, PexState, PexValue,
    PexVariable,
};

#[test]
fn extracts_callable_signature_without_inventing_event_or_defaults() {
    let file = actor_fixture();
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
    assert_eq!(
        extracted.bundle.scripts[0].documentation.as_deref(),
        Some("Actor docs")
    );
    assert_eq!(member.documentation.as_deref(), Some("GetValue docs"));
    assert_eq!(
        extracted.bundle.scripts[0]
            .members
            .iter()
            .find(|item| item.name == "Value")
            .unwrap()
            .documentation
            .as_deref(),
        Some("Value docs")
    );
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

fn actor_fixture() -> PexFile {
    let mut file = PexFile::new(PexHeader::skyrim(0, "Actor.psc", "", ""));
    let empty = file
        .intern("")
        .expect("synthetic fixture has a small string table");
    let actor = file
        .intern("Actor")
        .expect("synthetic fixture has a small string table");
    let get_value = file
        .intern("GetValue")
        .expect("synthetic fixture has a small string table");
    let int = file
        .intern("Int")
        .expect("synthetic fixture has a small string table");
    let count = file
        .intern("count")
        .expect("synthetic fixture has a small string table");
    let value = file
        .intern("Value")
        .expect("synthetic fixture has a small string table");
    let backing = file
        .intern("::Value_var")
        .expect("synthetic fixture has a small string table");
    let script_doc = file
        .intern("Actor docs")
        .expect("synthetic fixture has a small string table");
    let property_doc = file
        .intern("Value docs")
        .expect("synthetic fixture has a small string table");
    let callable_doc = file
        .intern("GetValue docs")
        .expect("synthetic fixture has a small string table");
    file.objects.push(PexObject {
        name: actor,
        parent_class_name: empty,
        documentation_string: script_doc,
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
            documentation_string: property_doc,
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
                documentation_string: callable_doc,
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
    file
}
