use super::*;
use crate::codec;
#[path = "../../tests/common/support.rs"]
mod common;
use common::sample;

#[test]
fn nontrailing_defaults_round_trip_without_changing_the_signature() {
    let bundle = decode(br#"{"format":"folio-declarations","schema":2,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Api","members":[{"name":"Travel","kind":"function","parameters":[{"name":"destination","ty":"Int","default":{"kind":"literal","value":"-1"}},{"name":"driver","ty":"Api"}]}]}]}"#).unwrap();
    for format in [DeclarationFormat::Json, DeclarationFormat::Binary] {
        let decoded = decode(&encode(&bundle, format).unwrap()).unwrap();
        let parameters = decoded.scripts[0].members[0].parameters();
        assert_eq!(parameters.len(), 2);
        assert_eq!(parameters[0].name, "destination");
        assert_eq!(parameters[0].ty, "Int");
        assert_eq!(
            parameters[0].default,
            crate::ParameterDefault::Literal("-1".into())
        );
        assert_eq!(parameters[1].name, "driver");
        assert_eq!(parameters[1].ty, "Api");
        assert_eq!(parameters[1].default, crate::ParameterDefault::Required);
    }
}

#[test]
fn binary_checks_header_hash_truncation_and_trailing_data() {
    let good = encode(&sample(), DeclarationFormat::Binary).unwrap();
    for cutoff in [0, 5, 59, good.len() - 1] {
        assert!(decode(&good[..cutoff]).is_err());
    }
    let mut bad = good.clone();
    bad[28] ^= 1;
    assert!(decode(&bad).is_err());
    let mut bad = good.clone();
    bad[12..20].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(decode(&bad).is_err());
    let mut bad = good.clone();
    bad.push(0);
    assert!(decode(&bad).is_err());
    // Even a matching outer length cannot conceal data after the DEFLATE stream.
    let length = (bad.len() - codec::HEADER_SIZE) as u64;
    bad[20..28].copy_from_slice(&length.to_le_bytes());
    assert!(decode(&bad).is_err());
    let mut truncated = good;
    truncated.pop();
    let length = (truncated.len() - codec::HEADER_SIZE) as u64;
    truncated[20..28].copy_from_slice(&length.to_le_bytes());
    assert!(decode(&truncated).is_err());
}

#[test]
fn schema_one_binary_keeps_its_exact_legacy_tuple_shape() {
    let member = (
        "Run",
        0u8,
        Vec::<String>::new(),
        None::<String>,
        2u8,
        0u8,
        None::<String>,
        Vec::<(String, String, u8, Option<String>)>::new(),
    );
    let state = ("Busy", true, vec![member.clone()]);
    let script = (
        "Demo",
        None::<String>,
        false,
        Vec::<String>::new(),
        Vec::<String>::new(),
        vec![member],
        vec![state],
        Some(("Demo.psc", 1u32, 1u32)),
    );
    let payload = rmp_serde::to_vec(&(
        1u32,
        "papyrus-skyrim",
        "fixture",
        None::<String>,
        vec![script],
    ))
    .unwrap();
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&payload).unwrap();
    let compressed = encoder.finish().unwrap();
    let mut carrier = MAGIC.to_vec();
    carrier.extend_from_slice(&1u32.to_le_bytes());
    carrier.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    carrier.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    carrier.extend_from_slice(blake3::hash(&payload).as_bytes());
    carrier.extend_from_slice(&compressed);
    let bundle = decode(&carrier).unwrap();
    assert_eq!(bundle.schema, 1);
    assert_eq!(bundle.scripts[0].name, "Demo");
    assert!(bundle.scripts[0].members[0].is_native());
    assert!(bundle.scripts[0].states[0].auto);
    assert_eq!(bundle.scripts[0].documentation, None);
    assert_eq!(bundle.scripts[0].states[0].documentation, None);
    assert_eq!(bundle.scripts[0].members[0].documentation, None);
    for format in [DeclarationFormat::Json, DeclarationFormat::Binary] {
        let upgraded = decode(&encode(&bundle, format).unwrap()).unwrap();
        assert_eq!(upgraded.schema, SCHEMA_VERSION);
        assert_eq!(
            crate::semantic_digest(&bundle),
            crate::semantic_digest(&upgraded)
        );
    }
    // A legacy-layout payload falsely advertised as schema 2 must remain invalid.
    carrier[8..12].copy_from_slice(&SCHEMA_VERSION.to_le_bytes());
    assert!(decode(&carrier).is_err());
    let mut carrier = encode(&bundle, DeclarationFormat::Binary).unwrap();
    carrier[8..12].copy_from_slice(&1u32.to_le_bytes());
    assert!(decode(&carrier).is_err());
}

#[test]
fn documentation_round_trips_and_does_not_change_semantic_identity() {
    let original = sample();
    let mut bundle = original;
    bundle.scripts[0].documentation = Some("Script docs\nSecond line".into());
    bundle.scripts[0].members[0].documentation = Some("Callable docs".into());
    bundle.scripts[0].states.push(crate::State {
        name: "Busy".into(),
        documentation: Some("State docs".into()),
        auto: false,
        members: vec![],
    });
    let mut without_docs = bundle.clone();
    without_docs.scripts[0].documentation = None;
    without_docs.scripts[0].members[0].documentation = None;
    without_docs.scripts[0].states[0].documentation = None;
    assert_eq!(
        crate::semantic_digest(&bundle),
        crate::semantic_digest(&without_docs)
    );
    for format in [DeclarationFormat::Json, DeclarationFormat::Binary] {
        assert_eq!(decode(&encode(&bundle, format).unwrap()).unwrap(), bundle);
    }
    bundle.schema = 1;
    assert!(validate(&bundle).is_err());
    bundle.schema = SCHEMA_VERSION;
    bundle.scripts[0].documentation = Some("x".repeat(crate::validation::MAX_STRING + 1));
    assert!(validate(&bundle).is_err());
    bundle.scripts[0].documentation = Some("   ".into());
    assert!(validate(&bundle).is_err());
}

#[test]
fn schema_one_json_loads_without_documentation_and_rejects_new_fields() {
    let legacy = br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Demo"}]}"#;
    let old = decode(legacy).unwrap();
    assert_eq!(old.scripts[0].documentation, None);
    let mut value = serde_json::from_slice::<serde_json::Value>(legacy).unwrap();
    value["scripts"][0]["documentation"] = "New docs".into();
    assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
}
