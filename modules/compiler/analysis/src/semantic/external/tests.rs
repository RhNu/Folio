use super::*;
use crate::AnalysisHost;
use folio_papyrus::PapyrusDialect;
use folio_source::Revision;

#[test]
fn body_and_member_changes_reuse_external_validation_but_source_headers_invalidate_it() {
    let mut host = AnalysisHost::new();
    let bundle = folio_format_declarations::decode(br#"{"format":"folio-declarations","schema":1,"profile":"papyrus-skyrim","origin":{"source":"fixture"},"scripts":[{"name":"External","members":[{"name":"Use","kind":"function","parameters":[{"name":"value","ty":"Local"}]}]}]}"#).unwrap();
    host.set_external_declarations(vec![bundle]);
    let file = FileId(0);
    host.upsert(
        file,
        Revision(1),
        Arc::from("ScriptName Local\n"),
        PapyrusDialect::Skyrim,
    )
    .unwrap();
    let first = host.view();
    let initial = first
        .external_declarations
        .validate_world(&first.semantic().world, &|| false)
        .unwrap();
    assert!(initial.is_empty());
    host.upsert(
        file,
        Revision(2),
        Arc::from("ScriptName Local\nInt Function Work()\n Return 2\nEndFunction\n"),
        PapyrusDialect::Skyrim,
    )
    .unwrap();
    let second = host.view();
    let reused = second
        .external_declarations
        .validate_world(&second.semantic().world, &|| false)
        .unwrap();
    assert!(Arc::ptr_eq(&initial, &reused));
    host.upsert(
        file,
        Revision(3),
        Arc::from("ScriptName Renamed\n"),
        PapyrusDialect::Skyrim,
    )
    .unwrap();
    let third = host.view();
    let changed = third
        .external_declarations
        .validate_world(&third.semantic().world, &|| false)
        .unwrap();
    assert!(
        changed
            .iter()
            .any(|diagnostic| diagnostic.code == "semantic.unknown-type")
    );
    assert!(initial.is_empty());
}
