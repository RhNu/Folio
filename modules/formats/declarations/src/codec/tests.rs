use super::*;
use crate::codec;
#[path = "../../tests/common/mod.rs"]
mod common;
use common::sample;

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
