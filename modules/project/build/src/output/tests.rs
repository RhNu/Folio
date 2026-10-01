use super::*;

#[test]
fn artifact_and_generation_paths_remain_within_managed_output() {
    assert!(safe_workspace_path("Scripts"));
    assert!(safe_workspace_path("Build/Scripts"));
    for path in [
        "../Scripts",
        ".folio/build",
        ".FOLIO/cache",
        "Source\\Scripts",
    ] {
        assert!(!safe_workspace_path(path));
    }
    assert!(safe_artifact_name("Sky_01.pex"));
    for name in [
        "../Sky.pex",
        "Sky/Other.pex",
        "NUL.pex",
        "com1.pex",
        "Sky:Other.pex",
        "Sky.txt",
    ] {
        assert!(!safe_artifact_name(name), "{name}");
    }
    assert!(safe_generation("abc123/generations/g123-abc-0", "abc123"));
    assert!(!safe_generation("../abc123/generations/g123", "abc123"));
    assert!(!safe_generation("abc123/generations/../other", "abc123"));
}
