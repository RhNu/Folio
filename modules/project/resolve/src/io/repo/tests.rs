use super::*;

#[test]
fn version_periods_are_not_format_suffixes() {
    assert_eq!(
        repo_candidates(Path::new("/repo"), "ck/1.6.1170.0"),
        vec![
            PathBuf::from("/repo/ck/1.6.1170.0.fdecl"),
            PathBuf::from("/repo/ck/1.6.1170.0.json")
        ]
    );
    assert_eq!(
        repo_candidates(Path::new("/repo"), "ck/1.6.1170.0.json"),
        vec![PathBuf::from("/repo/ck/1.6.1170.0.json")]
    );
}

#[test]
fn ambiguity_requires_explicit_format() {
    let candidates = repo_candidates(Path::new("/repo"), "api");
    assert!(matches!(
        select_candidate(Path::new("/repo"), "api", &candidates, &candidates),
        Err(LoadError::RepoAmbiguous { .. })
    ));
    assert_eq!(
        select_candidate(Path::new("/repo"), "api", &candidates, &candidates[..1]).unwrap(),
        candidates[0]
    );
}

#[test]
fn explicit_home_must_be_absolute() {
    assert!(FolioHome::from_values(Some(std::ffi::OsStr::new("relative")), None).is_err());
}

#[test]
fn format_suffix_is_case_insensitive_without_changing_the_key() {
    assert_eq!(
        repo_candidates(Path::new("/repo"), "api.JSON"),
        vec![PathBuf::from("/repo/api.JSON")]
    );
    assert_eq!(
        explicit_format("api.FDECL"),
        Some(DeclarationFormat::Binary)
    );
}
