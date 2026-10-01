use super::*;

// Each test target uses a subset of the shared in-memory project fixtures.
#[allow(dead_code)]
#[path = "../tests/common/mod.rs"]
pub(super) mod support;

fn source(path: &str, text: &str) -> ProjectSource {
    ProjectSource {
        package_key: "package".into(),
        canonical_path: PathBuf::from(path),
        display_path: path.into(),
        script_candidate: Path::new(path)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        dialect: PapyrusDialect::Skyrim,
        text: Arc::from(text),
    }
}

#[test]
fn stable_file_identity_and_header_feedback_use_shared_analysis() {
    let mut project = ProjectAnalysis::new();
    let first = project
        .sync_sources([source("Sky.psc", "Scriptname Sky\nFunction F() Native\n")])
        .unwrap();
    let file = first.file_for("package", Path::new("Sky.psc")).unwrap();
    assert!(first.issues.is_empty());
    let second = project
        .sync_sources([source("Sky.psc", "Scriptname Other\nFunction F() Native\n")])
        .unwrap();
    assert_eq!(second.file_for("package", Path::new("Sky.psc")), Some(file));
    assert_eq!(second.analysis.revision(file), Some(Revision(2)));
    assert_eq!(
        second.issues,
        vec![SourceIssue::NameMismatch {
            file,
            candidate: "Sky".into(),
            declared: "Other".into()
        }]
    );
    assert_eq!(first.analysis.revision(file), Some(Revision(1)));
    let empty = project.sync_sources([]).unwrap();
    assert!(empty.analysis.parse(file).is_none());
    let readded = project
        .sync_sources([source("Sky.psc", "Scriptname Sky\n")])
        .unwrap();
    assert_eq!(
        readded.file_for("package", Path::new("Sky.psc")),
        Some(file)
    );
    assert_eq!(readded.analysis.revision(file), Some(Revision(4)));
}

#[test]
fn duplicate_project_source_keeps_previous_generation() {
    let mut project = ProjectAnalysis::new();
    let first = project
        .sync_sources([source("One.psc", "Scriptname One\n")])
        .unwrap();
    let failure = project.sync_sources([
        source("Two.psc", "Scriptname Two\n"),
        source("Two.psc", "Scriptname Two\n"),
    ]);
    assert!(matches!(
        failure,
        Err(ProjectionError::DuplicateSource { .. })
    ));
    let same = project
        .sync_sources([source("One.psc", "Scriptname One\n")])
        .unwrap();
    assert_eq!(same.analysis.generation(), first.analysis.generation());
}
