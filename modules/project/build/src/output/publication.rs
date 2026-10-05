//! Output ownership checks, staged installation, and ordinary-failure rollback.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use tracing::{info, warn};

use super::{
    OutputError, SuccessRecord, error, existing_output_directory, existing_plain_artifact,
    output_directory, publish, read_verified, verify_success,
};

type OwnedOutputs<'a> = BTreeMap<PathBuf, &'a String>;
type SelectedOutputs<'a> = BTreeMap<PathBuf, (PathBuf, &'a String)>;

struct PublishedFile {
    destination: PathBuf,
    staging: Option<PathBuf>,
    backup: Option<PathBuf>,
    installed: bool,
}

/// Publish the complete selected set to the configured output, preserving unrelated files.
/// Normal failures restore the prior files; abrupt termination is detectable by digest checks.
///
/// # Errors
/// Returns an error for unsafe outputs, ownership or digest conflicts, staging failures, or publication failures.
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
    verify_ownership(&selected, &owned)?;
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
    let staging_result = stage_outputs(&output, &selected, nonce, &mut changes);
    if let Err(cause) = staging_result {
        remove_staging(&changes);
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
    let operation =
        install_outputs(&mut changes, &owned, nonce).and_then(|()| publish(build, result));
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
        remove_staging(&changes);
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

/// Verify all existing selected and previously owned paths before staging changes.
fn verify_ownership(
    selected: &SelectedOutputs<'_>,
    owned: &OwnedOutputs<'_>,
) -> Result<(), OutputError> {
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
    for (destination, digest) in owned {
        if existing_plain_artifact(destination)? && read_verified(destination, digest)?.is_none() {
            return Err(error(
                "publish output",
                destination,
                "owned file was modified",
            ));
        }
    }
    Ok(())
}

/// Stage complete artifacts and retain paths even when a later write fails.
fn stage_outputs(
    output: &Path,
    selected: &SelectedOutputs<'_>,
    nonce: u128,
    changes: &mut Vec<PublishedFile>,
) -> Result<(), OutputError> {
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
    Ok(())
}

/// Recheck digests immediately before replacing files and retain rollback backups.
fn install_outputs(
    changes: &mut [PublishedFile],
    owned: &OwnedOutputs<'_>,
    nonce: u128,
) -> Result<(), OutputError> {
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
    Ok(())
}

/// Remove partial staging files while retaining diagnostics for cleanup failures.
fn remove_staging(changes: &[PublishedFile]) {
    for change in changes {
        if let Some(staging) = &change.staging
            && let Err(cause) = fs::remove_file(staging)
            && cause.kind() != std::io::ErrorKind::NotFound
        {
            warn!(path = %staging.display(), %cause, "could not remove staged output");
        }
    }
}
