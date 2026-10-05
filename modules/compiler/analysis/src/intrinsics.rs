//! Authoritative compiler intrinsic signatures shared by checking and editor queries.
use folio_hir::{Symbol, Type};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntrinsicSignature {
    pub name: String,
    pub result: Type,
    pub parameters: Vec<(String, Type, Option<String>)>,
    pub callable: bool,
}

/// Script state methods and array operations are selected by the receiver's semantic type.
pub fn intrinsic_signature(name: &str, receiver: Option<&Type>) -> Option<IntrinsicSignature> {
    let (canonical, result, parameters, callable) = match receiver {
        Some(Type::Array(_)) if name.eq_ignore_ascii_case("Length") => {
            ("Length", Type::Int, Vec::new(), false)
        },
        Some(Type::Array(element))
            if name.eq_ignore_ascii_case("Find") || name.eq_ignore_ascii_case("RFind") =>
        {
            let reverse = name.eq_ignore_ascii_case("RFind");
            (
                if reverse { "RFind" } else { "Find" },
                Type::Int,
                vec![
                    ("akElement".into(), *element.clone(), None),
                    (
                        "aiStartIndex".into(),
                        Type::Int,
                        Some(if reverse { "-1" } else { "0" }.into()),
                    ),
                ],
                true,
            )
        },
        None | Some(Type::Script(_)) if name.eq_ignore_ascii_case("GetState") => {
            ("GetState", Type::String, Vec::new(), true)
        },
        None | Some(Type::Script(_)) if name.eq_ignore_ascii_case("GotoState") => (
            "GotoState",
            Type::Void,
            vec![("asNewState".into(), Type::String, None)],
            true,
        ),
        _ => return None,
    };
    Some(IntrinsicSignature {
        name: canonical.into(),
        result,
        parameters,
        callable,
    })
}

pub(crate) fn intrinsic_candidates(
    receiver: &Type,
    prefix: &str,
) -> Vec<super::CompletionCandidate> {
    ["Length", "Find", "RFind", "GetState", "GotoState"]
        .iter()
        .filter(|name| {
            name.get(..prefix.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        })
        .filter_map(|name| intrinsic_signature(name, Some(receiver)))
        .map(|signature| super::CompletionCandidate {
            symbol: Symbol::Intrinsic {
                name: signature.name,
            },
            ty: signature.result,
            definition: None,
        })
        .collect()
}
