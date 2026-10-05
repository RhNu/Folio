//! Local declaration repository location, containment and atomic publication.

use std::{
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
};

use folio_format_declarations::DeclarationFormat;

use super::{InputSnapshot, LoadError, Path, PathBuf, decode, fs, info, io, manifest, scan};

#[derive(Clone, Debug)]
pub struct FolioHome {
    pub path: PathBuf,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RepoEntry {
    pub key: String,
    pub path: PathBuf,
    pub profile: String,
    pub source: String,
    pub scripts: usize,
}

impl FolioHome {
    /// Resolve once at application startup; an explicit home must be absolute.
    ///
    /// # Errors
    /// Returns an error when environment values do not identify an absolute Folio home.
    pub fn from_env() -> Result<Self, LoadError> {
        let value = std::env::var_os("FOLIO_HOME");
        let user = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
        Self::from_values(value.as_deref(), user.as_deref())
    }

    /// Pure environment-value handling shared by hosts and unit tests.
    ///
    /// # Errors
    /// Returns an error if neither value provides a valid absolute Folio home.
    pub fn from_values(
        value: Option<&std::ffi::OsStr>,
        user: Option<&std::ffi::OsStr>,
    ) -> Result<Self, LoadError> {
        let path = if let Some(value) = value {
            PathBuf::from(value)
        } else {
            PathBuf::from(user.ok_or_else(|| LoadError::InvalidPath {
                path: PathBuf::new(),
                reason: "user home is unavailable; set FOLIO_HOME to an absolute directory",
            })?)
            .join(".folio")
        };
        if !path.is_absolute() {
            return Err(LoadError::InvalidPath {
                path,
                reason: "FOLIO_HOME must be an absolute directory",
            });
        }
        Ok(Self { path })
    }

    pub fn repo(&self) -> PathBuf { self.path.join("repo") }

    fn contained(&self, path: &Path) -> Result<PathBuf, LoadError> {
        let home = canonical_allow_missing(&self.path)?;
        let repo = canonical_allow_missing(&self.repo())?;
        let actual = canonical_allow_missing(path)?;
        if !repo.starts_with(&home) || !actual.starts_with(&repo) {
            return Err(LoadError::InvalidPath {
                path: path.to_owned(),
                reason: "repository path escapes FOLIO_HOME/repo through a link",
            });
        }
        Ok(actual)
    }

    /// # Errors
    /// Returns an error for invalid repository keys or unsafe repository paths.
    pub fn repo_output(&self, key: &str, format: DeclarationFormat) -> Result<PathBuf, LoadError> {
        manifest::validate_repo_key(key).map_err(|reason| LoadError::InvalidPath {
            path: PathBuf::from(key),
            reason,
        })?;
        let extension = format_extension(format);
        let key = match explicit_format(key) {
            Some(actual) if actual != format => {
                return Err(LoadError::InvalidPath {
                    path: PathBuf::from(key),
                    reason: "repository file suffix does not match output format",
                });
            },
            Some(_) => key.to_owned(),
            None => format!("{key}.{extension}"),
        };
        let path = self.repo().join(key);
        self.contained(&path)?;
        Ok(path)
    }

    /// Recheck containment at publication time, after declaration generation completes.
    ///
    /// # Errors
    /// Returns an error for invalid keys, unsafe repository directories, existing destinations, or publication failures.
    ///
    /// # Panics
    /// Panics if a validated repository output path has no parent directory.
    pub fn publish_repo(
        &self,
        key: &str,
        bytes: &[u8],
        format: DeclarationFormat,
    ) -> Result<PathBuf, LoadError> {
        let path = self.repo_output(key, format)?;
        let parent = path.parent().expect("repository file has a parent");
        fs::create_dir_all(parent)
            .map_err(|cause| io("create repository declaration directory", parent, cause))?;
        self.contained(&path)?;
        publish_declaration(&path, bytes)?;
        Ok(path)
    }

    pub(super) fn locate(
        &self,
        key: &str,
        snapshots: &mut Vec<InputSnapshot>,
    ) -> Result<PathBuf, LoadError> {
        manifest::validate_repo_key(key).map_err(|reason| LoadError::InvalidPath {
            path: PathBuf::from(key),
            reason,
        })?;
        let candidates = repo_candidates(&self.repo(), key);
        let mut existing = Vec::new();
        for candidate in &candidates {
            self.contained(candidate)?;
            let exists = candidate
                .try_exists()
                .map_err(|cause| io("inspect repository declaration", candidate, cause))?;
            snapshots.push(InputSnapshot::Probe {
                path: candidate.clone(),
                exists,
            });
            if exists {
                existing.push(candidate.clone());
            }
        }
        let path = select_candidate(&self.repo(), key, &candidates, &existing)?;
        let actual = self.contained(&path)?;
        if !actual.is_file() {
            return Err(LoadError::InvalidPath {
                path,
                reason: "repository declaration is not a file",
            });
        }
        snapshots.push(InputSnapshot::Resolution {
            path,
            canonical_path: actual.clone(),
        });
        info!(key, path = %actual.display(), "located repository declaration");
        Ok(actual)
    }

    /// Enumerate actual JSON and binary declaration files in stable repository order.
    ///
    /// # Errors
    /// Returns an error if repository traversal encounters unsafe entries or filesystem failures.
    pub fn list_repo(&self) -> Result<Vec<RepoEntry>, LoadError> {
        if !self
            .repo()
            .try_exists()
            .map_err(|cause| io("inspect repository", &self.repo(), cause))?
        {
            return Ok(Vec::new());
        }
        let root = self.contained(&self.repo())?;
        let mut snapshots = Vec::new();
        let files = scan::collect_files(&root, &["fdecl".into(), "json".into()], &mut snapshots)?;
        files
            .into_iter()
            .map(|(key, path)| {
                self.contained(&path)?;
                let bytes = fs::read(&path)
                    .map_err(|cause| io("read repository declaration", &path, cause))?;
                let bundle = decode(&bytes).map_err(|cause| LoadError::Declaration {
                    path: path.clone(),
                    cause,
                })?;
                Ok(RepoEntry {
                    key,
                    path,
                    profile: bundle.profile,
                    source: bundle.origin.source,
                    scripts: bundle.scripts.len(),
                })
            })
            .collect()
    }
}

fn format_extension(format: DeclarationFormat) -> &'static str {
    match format {
        DeclarationFormat::Json => "json",
        DeclarationFormat::Binary => "fdecl",
    }
}

fn explicit_format(key: &str) -> Option<DeclarationFormat> {
    let extension = key.rsplit_once('.')?.1;
    if extension.eq_ignore_ascii_case("json") {
        Some(DeclarationFormat::Json)
    } else if extension.eq_ignore_ascii_case("fdecl") {
        Some(DeclarationFormat::Binary)
    } else {
        None
    }
}

pub(super) fn repo_candidates(root: &Path, key: &str) -> Vec<PathBuf> {
    if explicit_format(key).is_some() {
        vec![root.join(key)]
    } else {
        vec![
            root.join(format!("{key}.fdecl")),
            root.join(format!("{key}.json")),
        ]
    }
}

fn select_candidate(
    repo: &Path,
    key: &str,
    candidates: &[PathBuf],
    existing: &[PathBuf],
) -> Result<PathBuf, LoadError> {
    match existing {
        [path] => Ok(path.clone()),
        [] => Err(LoadError::RepoMissing {
            repo: repo.to_owned(),
            key: key.to_owned(),
            candidates: candidates.to_vec(),
        }),
        _ => Err(LoadError::RepoAmbiguous {
            key: key.to_owned(),
            candidates: existing.to_vec(),
        }),
    }
}

/// Follow existing parents while preserving a missing descendant's lexical suffix.
fn canonical_allow_missing(path: &Path) -> Result<PathBuf, LoadError> {
    let mut current = path;
    let mut suffix = Vec::new();
    loop {
        match fs::canonicalize(current) {
            Ok(mut actual) => {
                for component in suffix.iter().rev() {
                    actual.push(component);
                }
                return Ok(actual);
            },
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                let name = current.file_name().ok_or_else(|| LoadError::InvalidPath {
                    path: path.to_owned(),
                    reason: "path has no existing filesystem ancestor",
                })?;
                suffix.push(name.to_os_string());
                current = current.parent().ok_or_else(|| LoadError::InvalidPath {
                    path: path.to_owned(),
                    reason: "path has no filesystem parent",
                })?;
            },
            Err(cause) => return Err(io("resolve repository path", current, cause)),
        }
    }
}

/// Publish already validated bytes atomically, refusing accidental replacement.
///
/// # Errors
/// Returns an error for unsafe paths, existing destinations, or staging and publication failures.
pub fn publish_declaration(path: &Path, bytes: &[u8]) -> Result<(), LoadError> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|cause| io("create declaration directory", parent, cause))?;
    if path
        .try_exists()
        .map_err(|cause| io("inspect declaration output", path, cause))?
    {
        return Err(LoadError::InvalidPath {
            path: path.to_owned(),
            reason: "declaration output already exists",
        });
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| LoadError::InvalidPath {
            path: path.to_owned(),
            reason: "declaration output has no file name",
        })?
        .to_string_lossy();
    let temporary = parent.join(format!(
        ".{file_name}.folio-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut created_temporary = false;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|cause| io("create declaration temporary file", &temporary, cause))?;
        created_temporary = true;
        file.write_all(bytes)
            .map_err(|cause| io("write declaration", &temporary, cause))?;
        file.sync_all()
            .map_err(|cause| io("flush declaration", &temporary, cause))?;
        drop(file);
        fs::hard_link(&temporary, path)
            .map_err(|cause| io("publish declaration without replacement", path, cause))?;
        Ok(())
    })();
    if created_temporary {
        drop(fs::remove_file(&temporary));
    }
    if result.is_ok() {
        info!(path = %path.display(), bytes = bytes.len(), "published declaration");
    }
    result
}

#[cfg(test)]
mod tests;
