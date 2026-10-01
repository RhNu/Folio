//! Portable declaration facts with parallel JSON and compressed binary carriers.

mod codec;
mod model;
mod validation;
mod wire;

pub use codec::{DeclarationFormat, DecodeError, decode, encode};
pub use model::*;
pub use validation::validate;

pub const FORMAT: &str = "folio-declarations";
pub const SCHEMA_VERSION: u32 = 2;
pub const PROFILE: &str = "papyrus-skyrim";

/// API identity excludes provenance and ordering of unordered declarations.
/// Callers validate inputs before using this identity; parameter order is retained.
pub fn semantic_digest(bundle: &DeclarationBundle) -> String {
    let mut normalized = bundle.clone();
    normalized.schema = SCHEMA_VERSION;
    normalized.origin = Origin {
        source: String::new(),
        input_digest: None,
    };
    for script in &mut normalized.scripts {
        script.source = None;
        script.documentation = None;
        script.flags.sort();
        script
            .imports
            .sort_by_key(|name| (name.to_ascii_lowercase(), name.clone()));
        normalize_members(&mut script.members);
        for state in &mut script.states {
            state.documentation = None;
            normalize_members(&mut state.members);
        }
        script
            .states
            .sort_by_key(|state| state.name.to_ascii_lowercase());
    }
    normalized
        .scripts
        .sort_by_key(|script| script.name.to_ascii_lowercase());
    let bytes = serde_json::to_vec(&normalized).expect("declaration facts are serializable");
    blake3::hash(&bytes).to_hex().to_string()
}

fn normalize_members(members: &mut [Member]) {
    for member in members.iter_mut() {
        member.documentation = None;
        member.flags.sort();
    }
    members.sort_by_key(|member| (member.name.to_ascii_lowercase(), member.kind()));
}
