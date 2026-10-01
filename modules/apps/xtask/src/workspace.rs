//! Locate the Cargo workspace, scan authored Rust sources, and report size diagnostics.

use std::{
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use serde::Deserialize;
use tracing::{debug, info};

use crate::lines::{ERROR_LINES, Level, WARN_LINES, code_lines};

type TaskResult<T> = Result<T, Box<dyn Error>>;

#[derive(Deserialize)]
struct Workspace {
    workspace_root: PathBuf,
    target_directory: PathBuf,
}

/// Let Cargo resolve workspace membership and the configured target directory.
fn locate(manifest: Option<&Path>) -> TaskResult<Workspace> {
    let default_manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = manifest.unwrap_or(&default_manifest);
    debug!(path = %manifest.display(), "locating Cargo workspace");
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata for {} failed ({}): {}",
            manifest.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// Attach the affected path to filesystem failures instead of reporting an opaque I/O error.
fn path_error(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

/// Include tests and disabled-feature sources; skip generated trees and never follow links.
/// Abort on unreadable paths so a partial scan cannot appear successful.
fn rust_sources(workspace: &Workspace) -> io::Result<Vec<PathBuf>> {
    let mut pending = vec![workspace.workspace_root.clone()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| path_error(&directory, error))? {
            let entry = entry.map_err(|error| path_error(&directory, error))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|error| path_error(&path, error))?;
            if kind.is_symlink() {
                debug!(path = %path.display(), "skipping linked source path");
            } else if kind.is_dir() {
                let name = entry.file_name();
                if path == workspace.target_directory
                    || [".git", "target", "node_modules", ".folio"]
                        .iter()
                        .any(|excluded| name == *excluded)
                {
                    debug!(path = %path.display(), "skipping generated directory");
                } else {
                    pending.push(path);
                }
            } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "rs")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Report all threshold breaches in path order; warnings do not fail the check.
pub fn check_lines(manifest: Option<&Path>, all: bool) -> TaskResult<bool> {
    let workspace = locate(manifest)?;
    info!(
        root = %workspace.workspace_root.display(),
        target = %workspace.target_directory.display(),
        "checking Rust source lines"
    );
    let files = rust_sources(&workspace)?;
    let mut warnings = 0;
    let mut errors = 0;
    let mut maximum = 0;
    for file in &files {
        let source = fs::read_to_string(file).map_err(|error| path_error(file, error))?;
        let count = code_lines(&source);
        maximum = maximum.max(count);
        let relative = file.strip_prefix(&workspace.workspace_root)?;
        debug!(path = %relative.display(), lines = count, "counted source lines");
        match Level::for_lines(count) {
            Level::Ok if all => println!("ok: {}: {count} code lines", relative.display()),
            Level::Ok => {}
            Level::Warning => {
                warnings += 1;
                println!(
                    "warning: {}: {count} code lines (limit {WARN_LINES})",
                    relative.display()
                );
            }
            Level::Error => {
                errors += 1;
                println!(
                    "error: {}: {count} code lines (limit {ERROR_LINES}; split this file)",
                    relative.display()
                );
            }
        }
    }
    println!(
        "Checked {} Rust files: maximum {maximum} code lines, {warnings} warnings (>{WARN_LINES}), {errors} errors (>{ERROR_LINES})",
        files.len()
    );
    info!(
        files = files.len(),
        maximum, warnings, errors, "source line check complete"
    );
    Ok(errors == 0)
}
