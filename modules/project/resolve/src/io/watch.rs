//! Watch inputs are planned before decoding any dependency carrier.

use super::*;

#[derive(Clone, Debug, Default)]
pub struct WatchPlan {
    pub files: Vec<PathBuf>,
    /// Existing directories watched recursively, including ancestors of missing inputs.
    pub directories: Vec<PathBuf>,
}

/// Best effort plan remains useful even when the manifest or dependencies are invalid.
pub fn watch_plan(start: &Path, explicit: Option<&Path>) -> WatchPlan {
    let home = FolioHome::from_env().ok();
    plan(start, explicit, home.as_ref())
}

pub fn watch_plan_with_home(start: &Path, explicit: Option<&Path>, home: &FolioHome) -> WatchPlan {
    plan(start, explicit, Some(home))
}

fn plan(start: &Path, explicit: Option<&Path>, home: Option<&FolioHome>) -> WatchPlan {
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let path = discover(start, explicit)
        .ok()
        .or_else(|| explicit.map(absolute));
    let Some(path) = path else {
        add_directory(start, &mut directories);
        return WatchPlan {
            files: files.into_iter().collect(),
            directories: directories.into_iter().collect(),
        };
    };
    files.insert(path.clone());
    let base = path.parent().unwrap_or(start);
    if let Ok(text) = fs::read_to_string(&path)
        && let Ok(manifest) = manifest::parse(MANIFEST_FILE, &text)
    {
        let source_root = base.join(&manifest.source_path.value);
        add_file(&source_root, &mut files, &mut directories);
        add_directory(&source_root, &mut directories);
        for dependency in manifest.dependencies {
            match dependency.kind {
                DependencyKind::Psc | DependencyKind::Pex => {
                    let path = base.join(dependency.path.value);
                    add_file(&path, &mut files, &mut directories);
                    add_directory(&path, &mut directories);
                }
                DependencyKind::Decl => add_file(
                    &base.join(dependency.path.value),
                    &mut files,
                    &mut directories,
                ),
                DependencyKind::Repo => {
                    if let Some(home) = home {
                        for candidate in repo::repo_candidates(&home.repo(), &dependency.path.value)
                        {
                            add_file(&candidate, &mut files, &mut directories);
                        }
                    }
                }
            }
        }
    }
    WatchPlan {
        files: files.into_iter().collect(),
        directories: directories.into_iter().collect(),
    }
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    }
}

fn add_file(path: &Path, files: &mut BTreeSet<PathBuf>, directories: &mut BTreeSet<PathBuf>) {
    let declared = absolute(path);
    files.insert(declared.clone());
    if let Ok(actual) = fs::canonicalize(path) {
        files.insert(actual);
    }
    // A link can be replaced without changing its old target; watch the alias itself.
    for ancestor in declared.ancestors() {
        if fs::symlink_metadata(ancestor).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            files.insert(ancestor.to_owned());
        }
    }
    if let Some(parent) = path.parent()
        && !parent.is_dir()
    {
        add_directory(parent, directories);
    }
}

fn add_directory(path: &Path, directories: &mut BTreeSet<PathBuf>) {
    let path = absolute(path);
    for ancestor in path.ancestors() {
        if ancestor.is_dir() {
            directories.insert(fs::canonicalize(ancestor).unwrap_or_else(|_| ancestor.to_owned()));
            break;
        }
    }
}
