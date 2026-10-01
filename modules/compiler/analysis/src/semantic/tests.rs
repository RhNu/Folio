use super::*;
use folio_format_declarations::decode;

#[test]
fn external_declarations_preserve_states_variables_and_property_access() {
    let bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"count","kind":"variable","ty":"Int"},{"name":"Value","kind":"property","ty":"Int","access":{"kind":"manual","readable":true,"writable":false}}],"states":[{"name":"Busy","auto":true,"members":[{"name":"Pulse","kind":"function","return_type":"Int"}]}]}]}"#).unwrap();
    let world = script_from_external(&bundle.scripts[0]);
    assert_eq!(world.variables["count"].kind, MemberKind::Variable);
    assert!(world.members["value"].read_only);
    assert_eq!(world.states["busy"]["pulse"].ty, Type::Int);
}

#[test]
fn external_model_keeps_property_and_callable_namespaces() {
    let bundle = decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"Base","members":[{"name":"Check","kind":"property","ty":"Bool","access":{"kind":"auto"}},{"name":"Check","kind":"function","return_type":"Bool"}]}]}"#).unwrap();
    let external = script_from_external(&bundle.scripts[0]);
    assert_eq!(external.members["check"].kind, MemberKind::Property);
    assert_eq!(
        external.callable_overloads["check"].kind,
        MemberKind::Function
    );
}
