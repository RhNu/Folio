//! Raw filesystem snapshots and stable source-directory traversal.

use super::{
    Arc, Component, LoadError, LoadedSourceInput, Path, PathBuf, SourceEncoding, SourceFile, fs,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputSnapshot {
    File {
        path: PathBuf,
        bytes: Arc<[u8]>,
    },
    Directory {
        path: PathBuf,
        entries: Vec<SnapshotEntry>,
    },
    Probe {
        path: PathBuf,
        exists: bool,
    },
    /// Preserve the declared path's destination as well as the bytes at that destination.
    Resolution {
        path: PathBuf,
        canonical_path: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotEntry {
    pub name: String,
    pub kind: SnapshotEntryKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotEntryKind {
    File,
    Directory,
    Other,
}

pub(super) fn read_bytes(
    path: &Path,
    snapshots: &mut Vec<InputSnapshot>,
) -> Result<Arc<[u8]>, LoadError> {
    let bytes: Arc<[u8]> = fs::read(path)
        .map_err(|cause| io("read input", path, cause))?
        .into();
    snapshots.push(InputSnapshot::File {
        path: path.to_owned(),
        bytes: Arc::clone(&bytes),
    });
    Ok(bytes)
}

fn directory_entries(path: &Path) -> Result<Vec<SnapshotEntry>, LoadError> {
    let mut entries = fs::read_dir(path)
        .map_err(|cause| io("read input directory", path, cause))?
        .map(|entry| {
            let entry = entry.map_err(|cause| io("read input entry", path, cause))?;
            let name =
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_cause| LoadError::InvalidPath {
                        path: entry.path(),
                        reason: "input entry name is not UTF-8",
                    })?;
            let kind = entry
                .file_type()
                .map_err(|cause| io("inspect input entry", &entry.path(), cause))?;
            Ok(SnapshotEntry {
                name,
                kind: if kind.is_file() {
                    SnapshotEntryKind::File
                } else if kind.is_dir() {
                    SnapshotEntryKind::Directory
                } else {
                    SnapshotEntryKind::Other
                },
            })
        })
        .collect::<Result<Vec<_>, LoadError>>()?;
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

/// Verify the exact bytes and membership used by an operation before publication.
///
/// # Errors
/// Returns an error if any recorded input changes or cannot be inspected safely.
pub fn verify_snapshots(snapshots: &[InputSnapshot]) -> Result<(), LoadError> {
    for snapshot in snapshots {
        let (path, unchanged) = match snapshot {
            InputSnapshot::File { path, bytes } => (
                path,
                fs::read(path)
                    .map_err(|cause| io("verify input", path, cause))?
                    .as_slice()
                    == bytes.as_ref(),
            ),
            InputSnapshot::Directory { path, entries } => {
                (path, directory_entries(path)? == *entries)
            },
            InputSnapshot::Probe { path, exists } => (
                path,
                path.try_exists()
                    .map_err(|cause| io("verify declaration candidate", path, cause))?
                    == *exists,
            ),
            InputSnapshot::Resolution {
                path,
                canonical_path,
            } => (path, canonical_file(path)? == *canonical_path),
        };
        if !unchanged {
            return Err(LoadError::InputChanged(path.clone()));
        }
    }
    Ok(())
}

pub(super) fn collect_files(
    root: &Path,
    extensions: &[String],
    snapshots: &mut Vec<InputSnapshot>,
) -> Result<Vec<(String, PathBuf)>, LoadError> {
    let actual = canonical_file(root)?;
    if !actual.is_dir() {
        return Err(LoadError::InvalidPath {
            path: actual,
            reason: "input root is not a directory",
        });
    }
    let mut pending = vec![actual.clone()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = directory_entries(&directory)?;
        for entry in &entries {
            let path = directory.join(&entry.name);
            match entry.kind {
                SnapshotEntryKind::Directory => pending.push(path),
                SnapshotEntryKind::Other => {
                    return Err(LoadError::InvalidPath {
                        path,
                        reason: "input directory contains a link or unsupported file type",
                    });
                },
                SnapshotEntryKind::File
                    if path.extension().is_some_and(|extension| {
                        extensions
                            .iter()
                            .any(|item| extension.eq_ignore_ascii_case(item))
                    }) =>
                {
                    let relative = path
                        .strip_prefix(&actual)
                        .expect("enumerated under source root")
                        .to_str()
                        .ok_or_else(|| LoadError::InvalidPath {
                            path: path.clone(),
                            reason: "input path is not UTF-8",
                        })?
                        .replace('\\', "/");
                    files.push((relative, path));
                },
                SnapshotEntryKind::File => {},
            }
        }
        snapshots.push(InputSnapshot::Directory {
            path: directory,
            entries,
        });
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

pub(super) fn collect_sources(
    base: &Path,
    root: &str,
    extensions: &[String],
    package_key: &str,
    encoding: SourceEncoding,
    snapshots: &mut Vec<InputSnapshot>,
) -> Result<(Vec<SourceFile>, Vec<LoadedSourceInput>), LoadError> {
    let declared = base.join(root);
    let canonical_path = canonical_file(&declared)?;
    snapshots.push(InputSnapshot::Resolution {
        path: declared,
        canonical_path: canonical_path.clone(),
    });
    let files = collect_files(&canonical_path, extensions, snapshots)?;
    let mut source_files = Vec::new();
    let mut inputs = Vec::new();
    for (relative, path) in files {
        let display_path = if root.is_empty() {
            relative.clone()
        } else {
            format!("{root}/{relative}")
        };
        let candidate = Path::new(&relative)
            .file_stem()
            .and_then(|value| value.to_str())
            .expect("validated UTF-8 source path")
            .to_owned();
        let bytes = read_bytes(&path, snapshots)?;
        let text =
            decode_source(&bytes, encoding).map_err(|reason| LoadError::SourceDeclarations {
                path: path.clone(),
                reason,
            })?;
        source_files.push(SourceFile {
            path: display_path.clone(),
            display_path: display_path.clone(),
            script_candidate: candidate.clone(),
        });
        inputs.push(LoadedSourceInput {
            package_key: package_key.into(),
            canonical_path: path,
            display_path,
            script_candidate: candidate,
            text: text.into(),
        });
    }
    Ok((source_files, inputs))
}

/// Decode once at the filesystem boundary; semantics sees ordinary UTF-8 text.
///
/// # Errors
/// Returns an error when bytes are invalid for the selected encoding.
pub fn decode_source(bytes: &[u8], encoding: SourceEncoding) -> Result<String, String> {
    match encoding {
        SourceEncoding::Utf8 => std::str::from_utf8(bytes)
            .map(|value| value.strip_prefix('\u{feff}').unwrap_or(value).to_owned())
            .map_err(|cause| format!("source is not UTF-8: {cause}")),
        SourceEncoding::Windows1252 => {
            let (text, errors) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes);
            if errors {
                Err("source contains invalid Windows-1252 data".into())
            } else {
                Ok(text.into_owned())
            }
        },
    }
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

/// Portable identity for paths known to lie on the same filesystem root.
///
/// # Errors
/// Returns an error for different or incompatible filesystem roots or non-UTF-8 target components.
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
            Component::CurDir => {},
            _ => {
                return Err(LoadError::InvalidPath {
                    path: target.to_owned(),
                    reason: "path has incompatible root",
                });
            },
        }
    }
    Ok(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}

#[cfg(test)]
mod tests;
