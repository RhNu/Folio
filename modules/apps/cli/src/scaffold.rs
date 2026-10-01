//! Create project manifests and starter Papyrus sources.
use super::CliError;
use std::{fs, io::Write, path::Path};
use tracing::info;

pub(super) fn create_project(
    directory: &Path,
    explicit_name: Option<&str>,
) -> Result<(), CliError> {
    let name = explicit_name
        .or_else(|| directory.file_name().and_then(|part| part.to_str()))
        .unwrap_or("folio-project");
    let (script, text) = scaffold(name)?;
    let manifest = directory.join("folio.toml");
    if manifest.exists() {
        return Err(CliError::ProjectCreation(format!(
            "{} already exists",
            manifest.display()
        )));
    }
    let source = directory.join("Source");
    let src = source.join("Scripts");
    for path in [&source, &src] {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(CliError::ProjectCreation(format!(
                    "{} is not a plain directory",
                    path.display()
                )));
            }
            Ok(_) => {}
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(path).map_err(|cause| {
                    CliError::ProjectCreation(format!("create {}: {cause}", path.display()))
                })?;
            }
            Err(cause) => {
                return Err(CliError::ProjectCreation(format!(
                    "inspect {}: {cause}",
                    path.display()
                )));
            }
        }
    }
    let script_path = src.join(format!("{script}.psc"));
    let script_text = format!("Scriptname {script}\n");
    let mut source = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&script_path)
        .map_err(|cause| {
            CliError::ProjectCreation(format!("create {}: {cause}", script_path.display()))
        })?;
    source
        .write_all(script_text.as_bytes())
        .map_err(CliError::Output)?;
    source.sync_all().map_err(CliError::Output)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest)
        .map_err(|cause| {
            CliError::ProjectCreation(format!("create {}: {cause}", manifest.display()))
        })?;
    file.write_all(text.as_bytes()).map_err(CliError::Output)?;
    file.sync_all().map_err(CliError::Output)?;
    info!(project = %directory.display(), package = name, "created project");
    Ok(())
}

/// Derive a valid starter script and manifest before touching the filesystem.
pub(super) fn scaffold(name: &str) -> Result<(String, String), CliError> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        || !name.bytes().any(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(CliError::ProjectCreation(
            "project name must use ASCII letters, digits, '-' or '_'".into(),
        ));
    }
    let suffix = name
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut letters = part.chars();
            let first = letters.next().expect("nonempty segment");
            format!("{}{}", first.to_ascii_uppercase(), letters.as_str())
        })
        .collect::<String>();
    let script = format!("Folio{suffix}");
    let text = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[languages.papyrus]\ndialect = \"skyrim\"\nextensions = [\"psc\"]\n\n[build]\ntarget = \"skyrim-se\"\nprofile = \"dev\"\nemit = [\"pex\"]\n"
    );
    Ok((script, text))
}

#[cfg(test)]
mod tests;
