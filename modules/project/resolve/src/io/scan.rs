//! Shared directory traversal and path identity for project inputs.

use super::*;

pub(super) fn collect_pex_files(root: &Path) -> Result<Vec<(String, Vec<u8>)>, LoadError> {
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|cause| io("read PEX directory", &directory, cause))?;
        for entry in entries {
            let entry = entry.map_err(|cause| io("read PEX entry", &directory, cause))?;
            let kind = entry
                .file_type()
                .map_err(|cause| io("inspect PEX entry", &entry.path(), cause))?;
            if kind.is_symlink() {
                return Err(LoadError::InvalidPath {
                    path: entry.path(),
                    reason: "PEX dependency contains a symlink",
                });
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("pex"))
            {
                let path = entry.path();
                let relative = path
                    .strip_prefix(root)
                    .expect("enumerated under PEX root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = fs::read(&path).map_err(|cause| io("read PEX file", &path, cause))?;
                files.push((relative, bytes));
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

pub(super) fn collect_sources(
    base: &Path,
    root: &str,
    extensions: &[String],
    package_key: &str,
) -> Result<(Vec<SourceFile>, Vec<LoadedSourceInput>), LoadError> {
    let mut files = BTreeMap::<String, (SourceFile, PathBuf)>::new();
    {
        let declared = base.join(root);
        let actual = canonical_file(&declared)?;
        if !actual.is_dir() {
            return Err(LoadError::InvalidPath {
                path: actual,
                reason: "source root is not a directory",
            });
        }
        let mut pending = BTreeMap::from([(root.to_owned(), actual)]);
        let mut visited = BTreeSet::new();
        while let Some((display_directory, directory)) = pending.pop_first() {
            let directory = canonical_file(&directory)?;
            if !visited.insert(directory.clone()) {
                continue;
            }
            let entries = fs::read_dir(&directory)
                .map_err(|cause| io("read source directory", &directory, cause))?;
            let mut entries = entries
                .collect::<Result<Vec<_>, _>>()
                .map_err(|cause| io("read source entry", &directory, cause))?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let display_path = PathBuf::from(&display_directory).join(entry.file_name());
                let path = canonical_file(&entry.path())?;
                if path.is_dir() {
                    let display = display_path
                        .to_str()
                        .ok_or_else(|| LoadError::InvalidPath {
                            path: display_path.clone(),
                            reason: "source display path is not UTF-8",
                        })?
                        .replace('\\', "/");
                    pending.insert(display, path);
                } else if path.is_file()
                    && display_path.extension().is_some_and(|extension| {
                        extensions
                            .iter()
                            .any(|item| extension.eq_ignore_ascii_case(item))
                    })
                {
                    let candidate = display_path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .ok_or_else(|| LoadError::InvalidPath {
                            path: display_path.clone(),
                            reason: "source file name is not UTF-8",
                        })?;
                    let portable = relative_portable(base, &path)?;
                    let display = display_path
                        .to_str()
                        .ok_or_else(|| LoadError::InvalidPath {
                            path: display_path.clone(),
                            reason: "source display path is not UTF-8",
                        })?
                        .replace('\\', "/");
                    let candidate_file = SourceFile {
                        path: portable,
                        display_path: display.clone(),
                        script_candidate: candidate.into(),
                    };
                    match files.entry(path_key(&path)?) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert((candidate_file, path));
                        }
                        std::collections::btree_map::Entry::Occupied(mut entry)
                            if display < entry.get().0.display_path =>
                        {
                            entry.insert((candidate_file, path));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    let mut source_files = Vec::new();
    let mut inputs = Vec::new();
    for (file, path) in files.into_values() {
        let text =
            fs::read_to_string(&path).map_err(|cause| io("read source file", &path, cause))?;
        inputs.push(LoadedSourceInput {
            package_key: package_key.into(),
            canonical_path: path,
            display_path: file.display_path.clone(),
            script_candidate: file.script_candidate.clone(),
            text: Arc::from(text),
        });
        source_files.push(file);
    }
    Ok((source_files, inputs))
}

pub(super) fn canonical_file(path: &Path) -> Result<PathBuf, LoadError> {
    fs::canonicalize(path).map_err(|cause| io("resolve path", path, cause))
}

pub(super) fn path_key(path: &Path) -> Result<String, LoadError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| LoadError::InvalidPath {
            path: path.to_owned(),
            reason: "canonical path is not UTF-8",
        })
}

pub(super) fn io(operation: &'static str, path: &Path, cause: std::io::Error) -> LoadError {
    LoadError::Io {
        operation,
        path: path.to_owned(),
        cause,
    }
}

/// Compute a portable relative identity after the host has canonicalized aliases.
pub fn relative_portable(base: &Path, target: &Path) -> Result<String, LoadError> {
    let base: Vec<_> = base.components().collect();
    let target_components: Vec<_> = target.components().collect();
    if base.first() != target_components.first() {
        return Err(LoadError::InvalidPath {
            path: target.to_owned(),
            reason: "path is on another filesystem root",
        });
    }
    let shared = base
        .iter()
        .zip(&target_components)
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = Vec::new();
    for component in &base[shared..] {
        if matches!(component, Component::Normal(_)) {
            parts.push("..".to_owned());
        }
    }
    for component in &target_components[shared..] {
        match component {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| LoadError::InvalidPath {
                        path: target.to_owned(),
                        reason: "path component is not UTF-8",
                    })?
                    .to_owned(),
            ),
            Component::CurDir => {}
            _ => {
                return Err(LoadError::InvalidPath {
                    path: target.to_owned(),
                    reason: "path has incompatible root",
                });
            }
        }
    }
    Ok(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}
