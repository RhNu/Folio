//! Managed generation, discardable cache and replaceable success index.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Persisted provenance for one generated PEX.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub kind: String,
    pub path: String,
    pub digest: String,
    pub format: String,
    pub script: String,
    pub source_package: String,
    pub source_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitRecord {
    pub unit_id: String,
    pub package: String,
    pub package_version: String,
    pub package_source: serde_json::Value,
    pub target: String,
    pub profile: String,
    pub fingerprint: String,
    pub generation: String,
    pub artifacts: Vec<ArtifactRecord>,
}

/// Last complete command selection in the success index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessRecord {
    pub schema: u32,
    pub output: String,
    pub units: Vec<UnitRecord>,
    pub external_requirements: Vec<ExternalRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRecord {
    pub package: String,
    pub package_version: Option<String>,
    pub package_source: serde_json::Value,
    pub target: String,
    pub abi: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheDecision {
    Hit,
    Missing,
    Invalid(&'static str),
}

pub type CachedArtifact = (ArtifactRecord, Vec<u8>);
pub type CacheRead = (CacheDecision, Vec<CachedArtifact>);

#[derive(Debug)]
pub struct OutputError {
    pub operation: &'static str,
    pub path: PathBuf,
    pub cause: String,
}

impl std::fmt::Display for OutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}: {}",
            self.operation,
            self.path.display(),
            self.cause
        )
    }
}

impl std::error::Error for OutputError {}

fn error(operation: &'static str, path: &Path, cause: impl ToString) -> OutputError {
    OutputError {
        operation,
        path: path.to_owned(),
        cause: cause.to_string(),
    }
}

/// Keep configured output and persisted index paths inside the workspace.
pub fn safe_workspace_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !value.to_ascii_lowercase().starts_with(".folio/")
        && !value.eq_ignore_ascii_case(".folio")
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn output_directory(project_root: &Path, relative: &str) -> Result<PathBuf, OutputError> {
    if !safe_workspace_path(relative) {
        return Err(error("validate output path", project_root, relative));
    }
    let mut directory = project_root.to_owned();
    for part in Path::new(relative).components() {
        directory.push(part.as_os_str());
        ensure_plain_directory(&directory)?;
    }
    Ok(directory)
}

fn existing_output_directory(
    project_root: &Path,
    relative: &str,
) -> Result<Option<PathBuf>, OutputError> {
    if !safe_workspace_path(relative) {
        return Ok(None);
    }
    let mut directory = project_root.to_owned();
    for part in Path::new(relative).components() {
        directory.push(part.as_os_str());
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Ok(None);
            }
            Ok(_) => {}
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(cause) => return Err(error("inspect output directory", &directory, cause)),
        }
    }
    Ok(Some(directory))
}

/// The visible output is valid only when every selected artifact matches its digest.
pub fn verify_published(project_root: &Path, result: &SuccessRecord) -> Result<bool, OutputError> {
    if !safe_workspace_path(&result.output) {
        return Ok(false);
    }
    let Some(directory) = existing_output_directory(project_root, &result.output)? else {
        return Ok(false);
    };
    for unit in &result.units {
        for artifact in &unit.artifacts {
            if !safe_artifact_name(&artifact.path)
                || read_verified(&directory.join(&artifact.path), &artifact.digest)?.is_none()
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Validate that a relative artifact name cannot escape a managed directory.
pub fn safe_artifact_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".pex") else {
        return false;
    };
    if stem.is_empty()
        || !stem
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return false;
    }
    let upper = stem.to_ascii_uppercase();
    !matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn ensure_plain_directory(path: &Path) -> Result<(), OutputError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(error(
            "inspect managed directory",
            path,
            "not a plain directory",
        )),
        Ok(_) => Ok(()),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|cause| error("create managed directory", path, cause))
        }
        Err(cause) => Err(error("inspect managed directory", path, cause)),
    }
}

/// The output root is private to Folio; each component is checked for symlinks.
pub fn managed_paths(project_root: &Path) -> Result<(PathBuf, PathBuf), OutputError> {
    let folio = project_root.join(".folio");
    ensure_plain_directory(&folio)?;
    let build = folio.join("build");
    let cache = folio.join("cache");
    ensure_plain_directory(&build)?;
    ensure_plain_directory(&cache)?;
    Ok((build, cache))
}

/// Locate an existing managed build root without changing the project.
pub fn existing_build_root(project_root: &Path) -> Result<Option<PathBuf>, OutputError> {
    let folio = project_root.join(".folio");
    let build = folio.join("build");
    for path in [&folio, &build] {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(error(
                    "inspect managed directory",
                    path,
                    "not a plain directory",
                ));
            }
            Ok(_) => {}
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(cause) => return Err(error("inspect managed directory", path, cause)),
        }
    }
    Ok(Some(build))
}

pub fn create_generation(build: &Path, unit_id: &str) -> Result<(String, PathBuf), OutputError> {
    let unit_root = build.join(unit_id);
    ensure_plain_directory(&unit_root)?;
    let generations = unit_root.join("generations");
    ensure_plain_directory(&generations)?;
    for attempt in 0..16_u32 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|cause| error("read clock", &generations, cause))?
            .as_nanos();
        let id = format!("g{nanos:x}-{:x}-{attempt:x}", std::process::id());
        let relative = format!("{unit_id}/generations/{id}");
        let path = build.join(&relative);
        match fs::create_dir(&path) {
            Ok(()) => return Ok((relative, path)),
            Err(cause) if cause.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(cause) => return Err(error("create generation", &path, cause)),
        }
    }
    Err(error(
        "create generation",
        &generations,
        "exhausted unique names",
    ))
}

pub fn write_artifact(dir: &Path, name: &str, bytes: &[u8]) -> Result<String, OutputError> {
    if !safe_artifact_name(name) {
        return Err(error("validate artifact name", dir, name));
    }
    let path = dir.join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|cause| error("create artifact", &path, cause))?;
    file.write_all(bytes)
        .map_err(|cause| error("write artifact", &path, cause))?;
    file.sync_all()
        .map_err(|cause| error("sync artifact", &path, cause))?;
    Ok(blake3::hash(bytes).to_hex().to_string())
}

fn read_verified(path: &Path, digest: &str) -> Result<Option<Vec<u8>>, OutputError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(cause) => return Err(error("inspect artifact", path, cause)),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|cause| error("read artifact", path, cause))?;
    Ok((blake3::hash(&bytes).to_hex().as_str() == digest).then_some(bytes))
}

fn existing_plain_artifact(path: &Path) -> Result<bool, OutputError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(error("inspect output artifact", path, "not a plain file"))
        }
        Ok(_) => Ok(true),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(cause) => Err(error("inspect output artifact", path, cause)),
    }
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    schema: u32,
    fingerprint: String,
    artifacts: Vec<ArtifactRecord>,
}

/// A cache hit requires an exact artifact list, valid metadata and content digests.
pub fn read_cache(
    cache: &Path,
    fingerprint: &str,
    expected: &[String],
) -> Result<CacheRead, OutputError> {
    let dir = cache.join(fingerprint);
    if !safe_hex(fingerprint) {
        return Ok((CacheDecision::Invalid("fingerprint"), Vec::new()));
    }
    match fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Ok((CacheDecision::Invalid("cache directory"), Vec::new()));
        }
        Ok(_) => {}
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok((CacheDecision::Missing, Vec::new()));
        }
        Err(cause) => return Err(error("inspect cache directory", &dir, cause)),
    }
    let manifest = dir.join("manifest.json");
    let manifest_metadata = match fs::symlink_metadata(&manifest) {
        Ok(value) => value,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok((CacheDecision::Invalid("manifest missing"), Vec::new()));
        }
        Err(cause) => return Err(error("inspect cache manifest", &manifest, cause)),
    };
    if !manifest_metadata.is_file() || manifest_metadata.file_type().is_symlink() {
        return Ok((CacheDecision::Invalid("manifest file"), Vec::new()));
    }
    let bytes = match fs::read(&manifest) {
        Ok(bytes) => bytes,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok((CacheDecision::Invalid("manifest missing"), Vec::new()));
        }
        Err(cause) => return Err(error("read cache manifest", &manifest, cause)),
    };
    let entry: CacheEntry = match serde_json::from_slice(&bytes) {
        Ok(entry) => entry,
        Err(_) => return Ok((CacheDecision::Invalid("manifest decode"), Vec::new())),
    };
    if entry.schema != 1 || entry.fingerprint != fingerprint {
        return Ok((CacheDecision::Invalid("schema or fingerprint"), Vec::new()));
    }
    let actual = entry
        .artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<Vec<_>>();
    if actual != expected || actual.iter().any(|name| !safe_artifact_name(name)) {
        return Ok((CacheDecision::Invalid("artifact list"), Vec::new()));
    }
    let mut artifacts = Vec::new();
    for artifact in entry.artifacts {
        match read_verified(&dir.join(&artifact.path), &artifact.digest)? {
            Some(bytes) => artifacts.push((artifact, bytes)),
            None => return Ok((CacheDecision::Invalid("artifact digest"), Vec::new())),
        }
    }
    Ok((CacheDecision::Hit, artifacts))
}

fn safe_hex(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Discard only a fingerprint-named directory directly beneath Folio's cache.
pub fn discard_cache(cache: &Path, fingerprint: &str) -> Result<(), OutputError> {
    if !safe_hex(fingerprint) {
        return Err(error("discard cache", cache, "invalid fingerprint"));
    }
    let dir = cache.join(fingerprint);
    let canonical_cache =
        fs::canonicalize(cache).map_err(|cause| error("resolve cache root", cache, cause))?;
    if dir.parent() != Some(cache) || !canonical_cache.is_dir() {
        return Err(error("discard cache", &dir, "outside cache root"));
    }
    match fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::remove_file(&dir).map_err(|cause| error("discard cache symlink", &dir, cause))
        }
        Ok(metadata) if metadata.is_dir() => {
            fs::remove_dir_all(&dir).map_err(|cause| error("discard cache entry", &dir, cause))
        }
        Ok(_) => fs::remove_file(&dir).map_err(|cause| error("discard cache entry", &dir, cause)),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(cause) => Err(error("inspect cache entry", &dir, cause)),
    }
}

/// Cache entries are immutable; concurrent or abandoned partial entries are misses.
pub fn write_cache(
    cache: &Path,
    fingerprint: &str,
    artifacts: &[(ArtifactRecord, Vec<u8>)],
) -> Result<(), OutputError> {
    if !safe_hex(fingerprint) {
        return Err(error("write cache", cache, "invalid fingerprint"));
    }
    let dir = cache.join(fingerprint);
    match fs::create_dir(&dir) {
        Ok(()) => {}
        Err(cause) if cause.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(cause) => return Err(error("create cache entry", &dir, cause)),
    }
    for (artifact, bytes) in artifacts {
        write_artifact(&dir, &artifact.path, bytes)?;
    }
    let entry = CacheEntry {
        schema: 1,
        fingerprint: fingerprint.to_owned(),
        artifacts: artifacts
            .iter()
            .map(|(artifact, _)| artifact.clone())
            .collect(),
    };
    let bytes =
        serde_json::to_vec(&entry).map_err(|cause| error("encode cache manifest", &dir, cause))?;
    let path = dir.join("manifest.json");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|cause| error("create cache manifest", &path, cause))?;
    file.write_all(&bytes)
        .map_err(|cause| error("write cache manifest", &path, cause))?;
    file.sync_all()
        .map_err(|cause| error("sync cache manifest", &path, cause))?;
    Ok(())
}

pub fn write_generation_manifest(dir: &Path, unit: &UnitRecord) -> Result<(), OutputError> {
    let path = dir.join("manifest.json");
    let bytes = serde_json::to_vec_pretty(unit)
        .map_err(|cause| error("encode generation manifest", &path, cause))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|cause| error("create generation manifest", &path, cause))?;
    file.write_all(&bytes)
        .map_err(|cause| error("write generation manifest", &path, cause))?;
    file.sync_all()
        .map_err(|cause| error("sync generation manifest", &path, cause))?;
    Ok(())
}

/// Verify every declared generation and artifact before considering an index valid.
pub fn verify_success(build: &Path, result: &SuccessRecord) -> Result<bool, OutputError> {
    if result.schema != 2 || !safe_workspace_path(&result.output) {
        return Ok(false);
    }
    for unit in &result.units {
        if !safe_generation(&unit.generation, &unit.unit_id) {
            return Ok(false);
        }
        let dir = build.join(&unit.generation);
        let unit_root = build.join(&unit.unit_id);
        let generations = unit_root.join("generations");
        for part in [&unit_root, &generations, &dir] {
            let Ok(metadata) = fs::symlink_metadata(part) else {
                return Ok(false);
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Ok(false);
            }
        }
        let manifest = dir.join("manifest.json");
        let Ok(manifest_metadata) = fs::symlink_metadata(&manifest) else {
            return Ok(false);
        };
        if !manifest_metadata.is_file() || manifest_metadata.file_type().is_symlink() {
            return Ok(false);
        }
        let manifest_bytes = match fs::read(&manifest) {
            Ok(bytes) => bytes,
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(cause) => return Err(error("read generation manifest", &manifest, cause)),
        };
        let Ok(persisted) = serde_json::from_slice::<UnitRecord>(&manifest_bytes) else {
            return Ok(false);
        };
        if &persisted != unit {
            return Ok(false);
        }
        for artifact in &unit.artifacts {
            if !safe_artifact_name(&artifact.path)
                || read_verified(&dir.join(&artifact.path), &artifact.digest)?.is_none()
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub fn safe_generation(path: &str, unit_id: &str) -> bool {
    let Some(id) = path.strip_prefix(&format!("{unit_id}/generations/")) else {
        return false;
    };
    !unit_id.is_empty()
        && unit_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        && !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'g' || byte == b'-')
}

/// Current verified selection, with invalid state surfaced to inspection.
pub struct IndexRead {
    pub result: Option<SuccessRecord>,
    pub invalid: bool,
}

pub fn read_index(build: &Path) -> Result<IndexRead, OutputError> {
    let path = build.join("last-success.json");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Ok(IndexRead {
                result: None,
                invalid: true,
            });
        }
        Ok(_) => {}
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok(IndexRead {
                result: None,
                invalid: false,
            });
        }
        Err(cause) => return Err(error("inspect success index", &path, cause)),
    }
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            return Ok(IndexRead {
                result: None,
                invalid: false,
            });
        }
        Err(cause) => return Err(error("read success index", &path, cause)),
    };
    let Ok(record) = serde_json::from_slice::<SuccessRecord>(&bytes) else {
        return Ok(IndexRead {
            result: None,
            invalid: true,
        });
    };
    if !verify_success(build, &record)? {
        return Ok(IndexRead {
            result: None,
            invalid: true,
        });
    }
    Ok(IndexRead {
        result: Some(record),
        invalid: false,
    })
}

/// Publish one complete selection through a synced same-directory temporary file.
/// Rename replaces the prior index only after every generation has been verified.
pub fn publish(build: &Path, result: &SuccessRecord) -> Result<(), OutputError> {
    if !verify_success(build, result)? {
        return Err(error(
            "publish build",
            build,
            "generation validation failed",
        ));
    }
    let path = build.join("last-success.json");
    let body = serde_json::to_vec_pretty(result)
        .map_err(|cause| error("encode success index", &path, cause))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|cause| error("read clock", build, cause))?
        .as_nanos();
    let staging = build.join(format!(
        ".last-success-{nanos:x}-{:x}.tmp",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)
        .map_err(|cause| error("create staged success index", &staging, cause))?;
    file.write_all(&body)
        .map_err(|cause| error("write staged success index", &staging, cause))?;
    file.sync_all()
        .map_err(|cause| error("sync staged success index", &staging, cause))?;
    drop(file);
    // On Windows rename can fail while another process holds the destination open.
    // The staged file is retained for recovery and the old index remains selected.
    fs::rename(&staging, &path).map_err(|cause| {
        error(
            "publish success index; staged file retained",
            &staging,
            cause,
        )
    })?;
    info!(units = result.units.len(), path = %path.display(), "published build selection");
    Ok(())
}

struct PublishedFile {
    destination: PathBuf,
    staging: Option<PathBuf>,
    backup: Option<PathBuf>,
    installed: bool,
}

/// Publish the complete selected set to the configured output, preserving unrelated files.
/// Normal failures restore the prior files; abrupt termination is detectable by digest checks.
pub fn publish_project(
    project_root: &Path,
    build: &Path,
    previous: Option<&SuccessRecord>,
    result: &SuccessRecord,
) -> Result<(), OutputError> {
    if !verify_success(build, result)? {
        return Err(error(
            "publish output",
            build,
            "generation validation failed",
        ));
    }
    let output = output_directory(project_root, &result.output)?;
    let old_directory = previous
        .map(|old| existing_output_directory(project_root, &old.output))
        .transpose()?
        .flatten();
    let mut owned = BTreeMap::new();
    if let (Some(old), Some(directory)) = (previous, &old_directory) {
        for artifact in old.units.iter().flat_map(|unit| &unit.artifacts) {
            owned.insert(directory.join(&artifact.path), &artifact.digest);
        }
    }
    let mut selected = BTreeMap::new();
    for unit in &result.units {
        for artifact in &unit.artifacts {
            let destination = output.join(&artifact.path);
            selected.insert(
                destination,
                (
                    build.join(&unit.generation).join(&artifact.path),
                    &artifact.digest,
                ),
            );
        }
    }
    // Validate ownership before touching any visible output.
    for destination in selected.keys() {
        if existing_plain_artifact(destination)? {
            let Some(digest) = owned.get(destination) else {
                return Err(error(
                    "publish output",
                    destination,
                    "existing file is not owned by Folio",
                ));
            };
            if read_verified(destination, digest)?.is_none() {
                return Err(error(
                    "publish output",
                    destination,
                    "owned file was modified",
                ));
            }
        }
    }
    for (destination, digest) in &owned {
        if existing_plain_artifact(destination)? && read_verified(destination, digest)?.is_none() {
            return Err(error(
                "publish output",
                destination,
                "owned file was modified",
            ));
        }
    }
    let mut stale = Vec::new();
    for destination in owned.keys().filter(|path| !selected.contains_key(*path)) {
        if existing_plain_artifact(destination)? {
            stale.push(destination.clone());
        }
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|cause| error("read clock", &output, cause))?
        .as_nanos();
    let mut changes = Vec::new();
    let staging_result = (|| {
        for (index, (destination, (source, digest))) in selected.iter().enumerate() {
            let bytes = read_verified(source, digest)?
                .ok_or_else(|| error("stage output", source, "artifact digest mismatch"))?;
            let staging = output.join(format!(
                ".folio-stage-{nonce:x}-{:x}-{index:x}",
                std::process::id()
            ));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)
                .map_err(|cause| error("create staged output", &staging, cause))?;
            changes.push(PublishedFile {
                destination: destination.clone(),
                staging: Some(staging.clone()),
                backup: None,
                installed: false,
            });
            file.write_all(&bytes)
                .map_err(|cause| error("write staged output", &staging, cause))?;
            file.sync_all()
                .map_err(|cause| error("sync staged output", &staging, cause))?;
        }
        Ok::<(), OutputError>(())
    })();
    if let Err(cause) = staging_result {
        for change in &changes {
            if let Some(staging) = &change.staging {
                let _ = fs::remove_file(staging);
            }
        }
        return Err(cause);
    }
    for destination in stale {
        changes.push(PublishedFile {
            destination,
            staging: None,
            backup: None,
            installed: false,
        });
    }
    let operation = (|| {
        for (index, change) in changes.iter_mut().enumerate() {
            if existing_plain_artifact(&change.destination)? {
                let Some(digest) = owned.get(&change.destination) else {
                    return Err(error(
                        "publish output",
                        &change.destination,
                        "existing file is not owned by Folio",
                    ));
                };
                if read_verified(&change.destination, digest)?.is_none() {
                    return Err(error(
                        "publish output",
                        &change.destination,
                        "owned file changed during publication",
                    ));
                }
                let backup = change.destination.with_file_name(format!(
                    ".folio-backup-{nonce:x}-{:x}-{index:x}",
                    std::process::id()
                ));
                if fs::symlink_metadata(&backup).is_ok() {
                    return Err(error(
                        "backup output",
                        &backup,
                        "backup path already exists",
                    ));
                }
                fs::rename(&change.destination, &backup)
                    .map_err(|cause| error("backup output", &change.destination, cause))?;
                change.backup = Some(backup);
            }
            if let Some(staging) = &change.staging {
                fs::rename(staging, &change.destination)
                    .map_err(|cause| error("install output", &change.destination, cause))?;
                change.installed = true;
            }
        }
        publish(build, result)
    })();
    if let Err(cause) = operation {
        for change in changes.iter().rev() {
            if change.installed
                && let Err(restore) = fs::remove_file(&change.destination)
            {
                warn!(path = %change.destination.display(), %restore, "could not remove incomplete output");
            }
            if let Some(backup) = &change.backup
                && let Err(restore) = fs::rename(backup, &change.destination)
            {
                warn!(path = %change.destination.display(), %restore, "could not restore prior output");
            }
        }
        for change in &changes {
            if let Some(staging) = &change.staging {
                let _ = fs::remove_file(staging);
            }
        }
        return Err(cause);
    }
    for change in &changes {
        if let Some(backup) = &change.backup
            && let Err(cause) = fs::remove_file(backup)
        {
            warn!(path = %backup.display(), %cause, "could not remove prior output backup");
        }
    }
    info!(output = %output.display(), artifacts = selected.len(), "published workspace output");
    Ok(())
}

/// An OS file lock is released on process exit, including abrupt termination.
pub struct BuildLock {
    _file: File,
}

impl BuildLock {
    pub fn acquire(build: &Path) -> Result<Self, OutputError> {
        let path = build.join(".publish.guard");
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(error("inspect build lock", &path, "not a plain file"));
        }
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|cause| error("acquire build lock", &path, cause))?;
        file.try_lock()
            .map_err(|cause| error("lock build output", &path, cause))?;
        debug!(path = %path.display(), "acquired build lock");
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
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
}
