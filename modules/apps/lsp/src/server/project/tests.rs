use std::path::PathBuf;

use super::*;

#[test]
fn watchers_filter_unrelated_json_but_keep_sources_carriers_and_recovery_ancestors() {
    let plan = folio_project_resolve::io::WatchPlan {
        files: vec![
            PathBuf::from("project/folio.toml"),
            PathBuf::from("repo/missing/api.json"),
        ],
        directories: vec![PathBuf::from("project/src")],
    };
    for path in [
        "project/src/New.psc",
        "project/src/sub",
        "project/src/sub/Other.PSC",
        "repo/missing",
        "repo/missing/api.json",
        "project/folio.toml",
    ] {
        assert!(relevant_path(Path::new(path), &plan), "{path}");
    }
    for path in [
        "project/package.json",
        "project/.folio/log.json",
        "project/Scripts/output.pex",
        "project/src/config.json",
    ] {
        assert!(!relevant_path(Path::new(path), &plan), "{path}");
    }
}
