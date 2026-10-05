//! Exact schema 1 tuples: no optional trailing-field deserialization shortcuts.

use serde::Deserialize;

use super::{
    Bundle, DecodeError, WireMember, WireParameter, WireScript, WireSource, WireState, binary,
};

#[derive(Deserialize)]
struct LegacyBundle(u32, String, String, Option<String>, Vec<LegacyScript>);

#[derive(Deserialize)]
struct LegacyScript(
    String,
    Option<String>,
    bool,
    Vec<String>,
    Vec<String>,
    Vec<LegacyMember>,
    Vec<LegacyState>,
    Option<WireSource>,
);

#[derive(Deserialize)]
struct LegacyState(String, bool, Vec<LegacyMember>);

#[derive(Deserialize)]
struct LegacyMember(
    String,
    u8,
    Vec<String>,
    Option<String>,
    u8,
    u8,
    Option<String>,
    Vec<WireParameter>,
);

pub(super) fn decode(input: &[u8]) -> Result<Bundle, DecodeError> {
    let LegacyBundle(schema, profile, source, digest, scripts) =
        rmp_serde::from_slice(input).map_err(binary)?;
    Ok(Bundle(
        schema,
        profile,
        source,
        digest,
        scripts
            .into_iter()
            .map(
                |LegacyScript(name, parent, native, flags, imports, members, states, source)| {
                    WireScript(
                        name,
                        parent,
                        native,
                        flags,
                        imports,
                        members.into_iter().map(member).collect(),
                        states
                            .into_iter()
                            .map(|LegacyState(name, auto, members)| {
                                WireState(
                                    name,
                                    auto,
                                    members.into_iter().map(member).collect(),
                                    None,
                                )
                            })
                            .collect(),
                        source,
                        None,
                    )
                },
            )
            .collect(),
    ))
}

fn member(
    LegacyMember(name, tag, flags, ty, attributes, access, initial, parameters): LegacyMember,
) -> WireMember {
    WireMember(
        name, tag, flags, ty, attributes, access, initial, parameters, None,
    )
}
