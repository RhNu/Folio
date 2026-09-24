//! Pure conversion of Skyrim PEX binaries into the API facts they actually retain.

use folio_format_declarations::{
    Compatibility, DeclarationBundle, Member, MemberKind, Naming, Package, Parameter, Script, State,
};
use folio_format_pex::{PexFile, PexFunction, PexTarget};

/// One binary and its directory-relative path; no filesystem access occurs here.
pub struct PexInput<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
}

/// Converted API plus exact binary file for each script in bundle order.
pub struct ExtractedPex {
    pub bundle: DeclarationBundle,
    pub paths: Vec<String>,
}

/// Decode every object as one script, preserving PEX's callable ambiguity.
pub fn extract(name: &str, inputs: &[PexInput<'_>]) -> Result<ExtractedPex, String> {
    let mut sorted = inputs.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.path.cmp(right.path));
    let mut digest = blake3::Hasher::new();
    let mut scripts = Vec::new();
    for input in sorted {
        digest.update(&(input.path.len() as u64).to_le_bytes());
        digest.update(input.path.as_bytes());
        digest.update(&(input.bytes.len() as u64).to_le_bytes());
        digest.update(input.bytes);
        let file = PexFile::read_from_slice(input.bytes)
            .map_err(|cause| format!("{}: {cause}", input.path))?;
        if file.target() != PexTarget::skyrim() {
            return Err(format!("{}: PEX target is not Skyrim", input.path));
        }
        for object in &file.objects {
            let name = string(&file, object.name).to_owned();
            let parent = string(&file, object.parent_class_name);
            let auto_state = string(&file, object.auto_state_name);
            let mut members = Vec::new();
            let mut states = Vec::new();
            for variable in &object.variables {
                let variable_name = string(&file, variable.name);
                // Generated auto-property storage has no independent source API.
                if variable_name.starts_with("::")
                    || object
                        .properties
                        .iter()
                        .any(|property| property.auto_var == Some(variable.name))
                {
                    continue;
                }
                members.push(Member {
                    name: variable_name.into(),
                    kind: MemberKind::Variable,
                    ty: Some(string(&file, variable.type_name).into()),
                    is_global: false,
                    is_native: false,
                    is_auto: false,
                    is_read_only: false,
                    is_readable: false,
                    is_writable: false,
                    flags: Vec::new(),
                    initial_literal: None,
                    parameters: Vec::new(),
                    unknown_defaults: false,
                });
            }
            for property in &object.properties {
                members.push(Member {
                    name: string(&file, property.name).into(),
                    kind: MemberKind::Property,
                    ty: Some(string(&file, property.type_name).into()),
                    is_global: false,
                    is_native: false,
                    is_auto: property.is_auto,
                    is_read_only: !property.is_writable,
                    is_readable: property.is_readable,
                    is_writable: property.is_writable,
                    flags: Vec::new(),
                    initial_literal: None,
                    parameters: Vec::new(),
                    unknown_defaults: false,
                });
            }
            for state in &object.states {
                let state_name = string(&file, state.name);
                let functions = state
                    .functions
                    .iter()
                    .map(|function| callable(&file, function))
                    .collect();
                if state_name.is_empty() {
                    members.extend(functions);
                } else {
                    states.push(State {
                        name: state_name.into(),
                        auto: state_name.eq_ignore_ascii_case(auto_state),
                        members: functions,
                    });
                }
            }
            scripts.push((
                Script {
                    name,
                    parent: (!parent.is_empty()).then(|| parent.into()),
                    is_native: false,
                    flags: Vec::new(),
                    imports: Vec::new(),
                    members,
                    states,
                    source: None,
                },
                input.path.to_owned(),
            ));
        }
    }
    if scripts.is_empty() {
        return Err("PEX dependency contains no script objects".into());
    }
    scripts.sort_by_key(|(script, _)| script.name.to_ascii_lowercase());
    let (scripts, paths): (Vec<_>, Vec<_>) = scripts.into_iter().unzip();
    let bundle = DeclarationBundle {
        schema: folio_format_declarations::SCHEMA_VERSION,
        package: Package {
            name: name.into(),
            version: "local".into(),
            source: "local-pex".into(),
            generator: format!("folio-pex-import/{}", env!("CARGO_PKG_VERSION")),
            source_digest: Some(digest.finalize().to_hex().to_string()),
        },
        compatibility: Compatibility {
            target: "skyrim-se".into(),
            abi: "papyrus-skyrim".into(),
        },
        naming: Naming {
            language: "papyrus".into(),
            case_sensitive: false,
        },
        scripts,
    };
    folio_format_declarations::validate(&bundle).map_err(|cause| cause.to_string())?;
    tracing::info!(
        package = name,
        scripts = bundle.scripts.len(),
        "extracted PEX declarations"
    );
    Ok(ExtractedPex { bundle, paths })
}

fn string(file: &PexFile, id: folio_format_pex::PexStringId) -> &str {
    file.resolve_string(id)
        .expect("PEX reader validated string IDs")
}

fn callable(file: &PexFile, function: &PexFunction) -> Member {
    let return_type = string(file, function.return_type_name);
    Member {
        name: string(file, function.name).into(),
        kind: MemberKind::UnknownCallable,
        ty: (!return_type.eq_ignore_ascii_case("none")).then(|| return_type.into()),
        is_global: function.is_global,
        is_native: function.is_native,
        is_auto: false,
        is_read_only: false,
        is_readable: false,
        is_writable: false,
        flags: Vec::new(),
        initial_literal: None,
        parameters: function
            .parameters
            .iter()
            .map(|parameter| Parameter {
                name: string(file, parameter.name).into(),
                ty: string(file, parameter.type_name).into(),
                default_literal: None,
            })
            .collect(),
        unknown_defaults: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let extracted = extract(
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
        assert_eq!(member.kind, MemberKind::UnknownCallable);
        assert_eq!(member.ty.as_deref(), Some("Int"));
        assert!(member.unknown_defaults);
        assert_eq!(member.parameters[0].ty, "Int");
        assert_eq!(
            extracted.bundle.scripts[0]
                .members
                .iter()
                .filter(|item| item.kind == MemberKind::Property)
                .count(),
            1
        );
        assert!(
            extracted.bundle.scripts[0]
                .members
                .iter()
                .all(|item| item.kind != MemberKind::Variable)
        );
    }

    #[test]
    fn malformed_binary_reports_its_dependency_path() {
        let error = extract(
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
}
