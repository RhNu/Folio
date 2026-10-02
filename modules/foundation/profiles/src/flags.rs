//! Explicit metadata flag definitions, independent of manifests and source syntax.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A declaration category in the Skyrim metadata flag contract.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlagScope {
    Script,
    Property,
    Variable,
    Function,
}

/// A configured flag. An omitted bit is allocated deterministically before generation.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserFlag {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bit: Option<u8>,
    #[serde(default = "all_scopes")]
    pub scopes: Vec<FlagScope>,
}

fn all_scopes() -> Vec<FlagScope> {
    vec![
        FlagScope::Script,
        FlagScope::Property,
        FlagScope::Variable,
        FlagScope::Function,
    ]
}

impl UserFlag {
    pub fn applies_to(&self, scope: FlagScope) -> bool {
        self.scopes.contains(&scope)
    }
}

impl From<String> for UserFlag {
    fn from(name: String) -> Self {
        Self {
            name,
            bit: None,
            scopes: all_scopes(),
        }
    }
}

impl From<&str> for UserFlag {
    fn from(name: &str) -> Self {
        name.to_owned().into()
    }
}

/// Validate names/scopes and allocate free bits, preserving explicitly selected indexes.
pub fn resolve_user_flags(flags: &[UserFlag], maximum_bit: u8) -> Result<Vec<UserFlag>, String> {
    let mut names = BTreeSet::new();
    let mut bits = BTreeSet::from([0, 1]);
    for flag in flags {
        let name = flag.name.to_ascii_lowercase();
        if !name.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
            || !name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(format!("flag {} must be an identifier", flag.name));
        }
        if matches!(name.as_str(), "hidden" | "conditional") || is_skyrim_keyword(&name) {
            return Err(format!(
                "flag {} is a reserved language or standard flag name",
                flag.name
            ));
        }
        if !names.insert(name) {
            return Err(format!("duplicate flag {}", flag.name));
        }
        if flag.scopes.is_empty()
            || flag.scopes.iter().copied().collect::<BTreeSet<_>>().len() != flag.scopes.len()
        {
            return Err(format!(
                "flag {} must have distinct, nonempty scopes",
                flag.name
            ));
        }
        if let Some(bit) = flag.bit {
            if bit > maximum_bit || bit < 2 {
                return Err(format!(
                    "flag {} bit must be between 2 and {maximum_bit}",
                    flag.name
                ));
            }
            if !bits.insert(bit) {
                return Err(format!("duplicate flag bit {bit}"));
            }
        }
    }
    let mut resolved = flags.to_vec();
    resolved.sort_by_key(|flag| flag.name.to_ascii_lowercase());
    for flag in &mut resolved {
        if flag.bit.is_none() {
            let bit = (2..=maximum_bit)
                .find(|bit| !bits.contains(bit))
                .ok_or_else(|| "user flags exceed the target bit capacity".to_owned())?;
            bits.insert(bit);
            flag.bit = Some(bit);
        }
        flag.scopes.sort();
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests;

/// Whether a word is reserved by the Skyrim source language.
pub fn is_skyrim_keyword(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "as" | "auto"
            | "autoreadonly"
            | "bool"
            | "else"
            | "elseif"
            | "endevent"
            | "endfunction"
            | "endif"
            | "endproperty"
            | "endstate"
            | "endwhile"
            | "event"
            | "extends"
            | "false"
            | "float"
            | "function"
            | "global"
            | "if"
            | "import"
            | "int"
            | "length"
            | "native"
            | "new"
            | "none"
            | "parent"
            | "property"
            | "return"
            | "scriptname"
            | "self"
            | "state"
            | "string"
            | "true"
            | "while"
    )
}
