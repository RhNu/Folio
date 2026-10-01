//! Minimal declaration fixture shared by public and internal codec tests.
use crate::{DeclarationBundle, decode};

pub(crate) fn sample() -> DeclarationBundle {
    decode(br#"{"format":"folio-declarations","schema":2,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","source":{"path":"Base.psc","line":1,"column":1},"members":[{"name":"Read","kind":"unknown-callable","return_type":"Int","parameters":[{"name":"required","ty":"Int"},{"name":"known","ty":"Int","default":{"kind":"literal","value":"7"}},{"name":"unknown","ty":"Int","default":{"kind":"unknown"}}]},{"name":"Value","kind":"property","ty":"Int","access":{"kind":"auto-read-only"}}]}]}"#).unwrap()
}
