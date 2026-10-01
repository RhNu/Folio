//! Schema 1 binary wire types. Tuple order and numeric tags are protocol data.

mod guards;
pub(crate) use guards::guard_json;

use crate::{codec::binary, *};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Bundle(u32, String, String, Option<String>, Vec<WireScript>);
#[derive(Serialize, Deserialize)]
struct WireScript(
    String,
    Option<String>,
    bool,
    Vec<String>,
    Vec<String>,
    Vec<WireMember>,
    Vec<WireState>,
    Option<WireSource>,
);
#[derive(Serialize, Deserialize)]
struct WireState(String, bool, Vec<WireMember>);
#[derive(Serialize, Deserialize)]
struct WireSource(String, u32, u32);
#[derive(Serialize, Deserialize)]
struct WireMember(
    String,
    u8,
    Vec<String>,
    Option<String>,
    u8,
    u8,
    Option<String>,
    Vec<WireParameter>,
);
#[derive(Serialize, Deserialize)]
struct WireParameter(String, String, u8, Option<String>);

pub(crate) fn encode(bundle: &DeclarationBundle) -> Result<Vec<u8>, DecodeError> {
    let wire = Bundle(
        bundle.schema,
        bundle.profile.clone(),
        bundle.origin.source.clone(),
        bundle.origin.input_digest.clone(),
        bundle.scripts.iter().map(script_to_wire).collect(),
    );
    let payload = rmp_serde::to_vec(&wire).map_err(binary)?;
    // A successful encoder must never produce a carrier its reader rejects.
    guards::guard_messagepack(&payload)?;
    Ok(payload)
}

pub(crate) fn decode(input: &[u8]) -> Result<DeclarationBundle, DecodeError> {
    guards::guard_messagepack(input)?;
    let Bundle(schema, profile, source, input_digest, scripts) =
        rmp_serde::from_slice(input).map_err(binary)?;
    Ok(DeclarationBundle {
        format: FORMAT.into(),
        schema,
        profile,
        origin: Origin {
            source,
            input_digest,
        },
        scripts: scripts
            .into_iter()
            .map(script_from_wire)
            .collect::<Result<_, _>>()?,
    })
}

fn script_to_wire(script: &Script) -> WireScript {
    WireScript(
        script.name.clone(),
        script.parent.clone(),
        script.is_native,
        script.flags.clone(),
        script.imports.clone(),
        script.members.iter().map(member_to_wire).collect(),
        script
            .states
            .iter()
            .map(|state| {
                WireState(
                    state.name.clone(),
                    state.auto,
                    state.members.iter().map(member_to_wire).collect(),
                )
            })
            .collect(),
        script
            .source
            .as_ref()
            .map(|source| WireSource(source.path.clone(), source.line, source.column)),
    )
}

fn script_from_wire(wire: WireScript) -> Result<Script, DecodeError> {
    let WireScript(name, parent, is_native, flags, imports, members, states, source) = wire;
    Ok(Script {
        name,
        parent,
        is_native,
        flags,
        imports,
        members: members
            .into_iter()
            .map(member_from_wire)
            .collect::<Result<_, _>>()?,
        states: states
            .into_iter()
            .map(|WireState(name, auto, members)| {
                Ok(State {
                    name,
                    auto,
                    members: members
                        .into_iter()
                        .map(member_from_wire)
                        .collect::<Result<_, DecodeError>>()?,
                })
            })
            .collect::<Result<_, DecodeError>>()?,
        source: source.map(|WireSource(path, line, column)| SourceLocation { path, line, column }),
    })
}

fn member_to_wire(member: &Member) -> WireMember {
    // Member tags: function=0, event=1, unknown=2, property=3, variable=4.
    let tag = match member.kind() {
        MemberKind::Function => 0,
        MemberKind::Event => 1,
        MemberKind::UnknownCallable => 2,
        MemberKind::Property => 3,
        MemberKind::Variable => 4,
    };
    let attributes = u8::from(member.is_global()) | (u8::from(member.is_native()) << 1);
    let access = match &member.data {
        MemberData::Property {
            access: PropertyAccess::Auto,
            ..
        } => 4,
        MemberData::Property {
            access: PropertyAccess::AutoReadOnly,
            ..
        } => 5,
        MemberData::Property {
            access: PropertyAccess::Manual { readable, writable },
            ..
        } => u8::from(*readable) | (u8::from(*writable) << 1),
        _ => 0,
    };
    WireMember(
        member.name.clone(),
        tag,
        member.flags.clone(),
        member.ty().map(str::to_owned),
        attributes,
        access,
        member.initial_literal().map(str::to_owned),
        member
            .parameters()
            .iter()
            .map(|parameter| {
                let (tag, literal) = match &parameter.default {
                    ParameterDefault::Required => (0, None),
                    ParameterDefault::Literal(value) => (1, Some(value.clone())),
                    ParameterDefault::Unknown => (2, None),
                };
                WireParameter(parameter.name.clone(), parameter.ty.clone(), tag, literal)
            })
            .collect(),
    )
}

fn member_from_wire(wire: WireMember) -> Result<Member, DecodeError> {
    let WireMember(name, tag, flags, ty, attributes, access, initial_literal, parameters) = wire;
    if attributes & !3 != 0 {
        return Err(binary("unknown callable attribute bits"));
    }
    let global = attributes & 1 != 0;
    let native = attributes & 2 != 0;
    let parameters = parameters
        .into_iter()
        .map(|WireParameter(name, ty, tag, value)| {
            let default = match (tag, value) {
                (0, None) => ParameterDefault::Required,
                (1, Some(value)) => ParameterDefault::Literal(value),
                (2, None) => ParameterDefault::Unknown,
                _ => return Err(binary("invalid parameter default tag or value")),
            };
            Ok(Parameter { name, ty, default })
        })
        .collect::<Result<Vec<_>, DecodeError>>()?;
    let data = match tag {
        0..=2 => {
            if access != 0 || initial_literal.is_some() {
                return Err(binary("callable contains property or variable facts"));
            }
            match tag {
                0 => MemberData::Function {
                    return_type: ty,
                    global,
                    native,
                    parameters,
                },
                1 => {
                    if ty.is_some() || global {
                        return Err(binary("event contains return type or global attribute"));
                    }
                    MemberData::Event { native, parameters }
                }
                _ => MemberData::UnknownCallable {
                    return_type: ty,
                    global,
                    native,
                    parameters,
                },
            }
        }
        3 | 4 => {
            if attributes != 0 || !parameters.is_empty() {
                return Err(binary("storage member contains callable facts"));
            }
            let ty = ty.ok_or_else(|| binary("storage member requires a type"))?;
            if tag == 3 {
                let access = match access {
                    0..=3 => PropertyAccess::Manual {
                        readable: access & 1 != 0,
                        writable: access & 2 != 0,
                    },
                    4 => PropertyAccess::Auto,
                    5 => PropertyAccess::AutoReadOnly,
                    _ => return Err(binary("unknown property access tag")),
                };
                MemberData::Property {
                    ty,
                    access,
                    initial_literal,
                }
            } else {
                if access != 0 {
                    return Err(binary("variable contains property access facts"));
                }
                MemberData::Variable {
                    ty,
                    initial_literal,
                }
            }
        }
        _ => return Err(binary("unknown member kind tag")),
    };
    Ok(Member { name, flags, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(member: WireMember) -> Vec<u8> {
        rmp_serde::to_vec(&Bundle(
            1,
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
            );
            assert!(decode(&payload(member)).is_err());
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
        );
        assert!(decode(&payload(property)).is_err());
    }

    #[test]
    fn rejects_incorrect_fixed_array_lengths_and_tail_values() {
        // The schema contract requires all five bundle fields, including nil digest.
        let short = rmp_serde::to_vec(&(1u32, "papyrus-skyrim", "fixture")).unwrap();
        assert!(decode(&short).is_err());
        let long = rmp_serde::to_vec(&(
            1u32,
            "papyrus-skyrim",
            "fixture",
            None::<String>,
            Vec::<WireScript>::new(),
            0u8,
        ))
        .unwrap();
        assert!(decode(&long).is_err());
        let mut tail = rmp_serde::to_vec(&(
            1u32,
            "papyrus-skyrim",
            "fixture",
            None::<String>,
            Vec::<WireScript>::new(),
        ))
        .unwrap();
        tail.push(0xc0);
        assert!(decode(&tail).is_err());
    }
}
