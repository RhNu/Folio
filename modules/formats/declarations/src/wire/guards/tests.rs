use super::*;

#[test]
fn messagepack_rejects_excess_capacity_unknown_tags_and_trailing_values() {
    for bytes in [
        &[0xdd, 0xff, 0xff, 0xff, 0xff][..],
        &[0xc1],
        &[0xc0, 0xc0],
        &[0xa1, 0xff],
        &[0x92, 0xc0],
    ] {
        assert!(guard_messagepack(bytes).is_err());
    }
    let mut deep = vec![0x91; MAX_DEPTH + 1];
    deep.push(0xc0);
    assert!(guard_messagepack(&deep).is_err());
}

#[test]
fn json_bounds_strings_keys_containers_and_depth_before_allocating_model() {
    let large = "x".repeat(MAX_STRING + 1);
    assert!(guard_json(format!("\"{large}\"").as_bytes()).is_err());
    assert!(guard_json(format!("{{\"{large}\":null}}").as_bytes()).is_err());
    let array = format!("[{}0]", "0,".repeat(MAX_CONTAINER));
    assert!(guard_json(array.as_bytes()).is_err());
    let deep = format!(
        "{}0{}",
        "[".repeat(MAX_DEPTH + 1),
        "]".repeat(MAX_DEPTH + 1)
    );
    assert!(guard_json(deep.as_bytes()).is_err());
    assert!(guard_json(b"{} false").is_err());
}
