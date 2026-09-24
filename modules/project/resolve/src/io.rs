//! Filesystem shell. Pure manifest and graph decisions live in sibling modules.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use folio_declaration_tools::{GenerationOptions, SourceInput, generate};
use folio_format_declarations::{DeclarationBundle, builtin, decode, encode};
use folio_project_model::{
    DeclarationLocation, DeclaredScript, DependencyKind, LoadedCarrier, LoadedLink, LoadedPackage,
    LoadedSdk, SourceFile, SourceId,
};
use tracing::{debug, info, instrument};

use crate::{graph, manifest};

pub const MANIFEST_FILE: &str = "folio.toml";

/// Loaded declarations stay intact here for a later project-to-analysis projection.
pub struct LoadedProject {
    pub root_key: String,
    pub packages: Vec<LoadedPackage>,
    pub declaration_bundles: BTreeMap<String, DeclarationBundle>,
    pub source_inputs: Vec<LoadedSourceInput>,
}

/// Source text and canonical host identity provided to the analysis adapter.
pub struct LoadedSourceInput {
    pub package_key: String,
    pub canonical_path: PathBuf,
    pub display_path: String,
    pub script_candidate: String,
    pub text: Arc<str>,
}

/// Root sources selected by the manifest, independent of SDK availability.
pub struct RootSources {
    pub language: String,
    pub dialect: String,
    pub inputs: Vec<LoadedSourceInput>,
}

/// Loads only the root package's editable sources for syntax-only tools.
#[instrument(name = "project.load_root_sources", skip_all, fields(phase = "load"))]
pub fn load_root_sources(start: &Path, explicit: Option<&Path>) -> Result<RootSources, LoadError> {
    let path = discover(start, explicit)?;
    let base = path.parent().expect("canonical manifest has parent");
    let key = path_key(&path)?;
    let content = fs::read_to_string(&path).map_err(|cause| io("read manifest", &path, cause))?;
    let manifest = manifest::parse(MANIFEST_FILE, &content).map_err(LoadError::Manifest)?;
    validate_workspace_directory(base, &manifest.source_path.value, true)?;
    let (_, inputs) = collect_sources(
        base,
        &manifest.source_path.value,
        &manifest.extensions,
        &key,
    )?;
    info!(
        sources = inputs.len(),
        "loaded root sources for syntax tool"
    );
    Ok(RootSources {
        language: manifest.language,
        dialect: manifest.dialect,
        inputs,
    })
}

#[derive(Debug)]
pub enum LoadError {
    NotFound(PathBuf),
    Io {
        operation: &'static str,
        path: PathBuf,
        cause: std::io::Error,
    },
    Manifest(manifest::ManifestError),
    Declaration {
        path: PathBuf,
        cause: folio_format_declarations::DecodeError,
    },
    SourceDeclarations {
        path: PathBuf,
        reason: String,
    },
    Builtin {
        name: String,
        reason: String,
    },
    SdkNaming {
        path: PathBuf,
        language: String,
        case_sensitive: bool,
    },
    Resolve(Box<graph::ResolveError>),
    InvalidPath {
        path: PathBuf,
        reason: &'static str,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "no {MANIFEST_FILE} found from {}", path.display()),
            Self::Io {
                operation,
                path,
                cause,
            } => write!(f, "{operation} {}: {cause}", path.display()),
            Self::Manifest(cause) => write!(f, "{cause}"),
            Self::Declaration { path, cause } => write!(f, "{}: {cause}", path.display()),
            Self::SourceDeclarations { path, reason } => write!(f, "{}: {reason}", path.display()),
            Self::Builtin { name, reason } => write!(f, "built-in {name}: {reason}"),
            Self::SdkNaming {
                path,
                language,
                case_sensitive,
            } => write!(
                f,
                "{}: unsupported SDK naming policy (language={language}, case_sensitive={case_sensitive})",
                path.display()
            ),
            Self::Resolve(cause) => write!(f, "{cause}"),
            Self::InvalidPath { path, reason } => write!(f, "{}: {reason}", path.display()),
        }
    }
}

impl std::error::Error for LoadError {}

/// Find the nearest manifest, or use the explicitly selected manifest path.
pub fn discover(start: &Path, explicit: Option<&Path>) -> Result<PathBuf, LoadError> {
    if let Some(path) = explicit {
        if path.file_name().is_none_or(|name| name != MANIFEST_FILE) {
            return Err(LoadError::InvalidPath {
                path: path.into(),
                reason: "manifest name must be folio.toml",
            });
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if !contains_manifest(parent)? {
            return Err(LoadError::NotFound(path.into()));
        }
        return canonical_file(path);
    }
    let start = canonical_file(start)?;
    let directory = if start.is_dir() {
        start
    } else {
        start
            .parent()
            .ok_or_else(|| LoadError::InvalidPath {
                path: start.clone(),
                reason: "has no parent directory",
            })?
            .to_owned()
    };
    for directory in directory.ancestors() {
        let candidate = directory.join(MANIFEST_FILE);
        if contains_manifest(directory)? {
            return canonical_file(&candidate);
        }
    }
    Err(LoadError::NotFound(directory))
}

fn contains_manifest(directory: &Path) -> Result<bool, LoadError> {
    let entries =
        fs::read_dir(directory).map_err(|cause| io("read project directory", directory, cause))?;
    for entry in entries {
        let entry = entry.map_err(|cause| io("read project entry", directory, cause))?;
        if entry.file_name() == MANIFEST_FILE
            && entry
                .file_type()
                .map_err(|cause| io("inspect manifest", &entry.path(), cause))?
                .is_file()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Read only manifest-declared dependencies and files under declared source roots.
#[instrument(name = "project.load", skip_all, fields(phase = "load"))]
pub fn load(start: &Path, explicit: Option<&Path>) -> Result<LoadedProject, LoadError> {
    let root_path = discover(start, explicit)?;
    let root_dir = root_path
        .parent()
        .expect("canonical manifest has parent")
        .to_owned();
    let root_key = path_key(&root_path)?;
    info!(manifest = %root_path.display(), "loading project");
    let mut pending = BTreeMap::from([(
        root_key.clone(),
        (root_path, DependencyKind::Package, None::<String>),
    )]);
    let mut packages = BTreeMap::new();
    let mut bundles = BTreeMap::new();
    let mut source_inputs = Vec::new();
    while let Some((key, (path, kind, declared_name))) = pending.pop_first() {
        if packages.contains_key(&key) {
            continue;
        }
        let portable = if kind == DependencyKind::Builtin {
            format!("builtin:{}", path.to_string_lossy())
        } else {
            relative_portable(&root_dir, &path)?
        };
        match kind {
            DependencyKind::Package => {
                let input =
                    fs::read_to_string(&path).map_err(|cause| io("read manifest", &path, cause))?;
                let manifest = manifest::parse(&portable, &input).map_err(LoadError::Manifest)?;
                let base = path.parent().expect("canonical manifest has parent");
                validate_workspace_directory(base, &manifest.source_path.value, true)?;
                validate_workspace_directory(base, &manifest.output_path.value, false)?;
                let (source_files, mut inputs) = collect_sources(
                    base,
                    &manifest.source_path.value,
                    &manifest.extensions,
                    &key,
                )?;
                source_inputs.append(&mut inputs);
                let mut links = Vec::new();
                for (index, dependency) in manifest.dependencies.iter().enumerate() {
                    if dependency.kind == DependencyKind::Builtin {
                        let id = dependency.path.value.clone();
                        let target_key = format!("builtin:{id}");
                        links.push(LoadedLink {
                            dependency_index: index,
                            source_key: target_key.clone(),
                        });
                        pending.entry(target_key).or_insert((
                            PathBuf::from(id),
                            DependencyKind::Builtin,
                            Some(dependency.name.value.clone()),
                        ));
                        continue;
                    }
                    let declared = base.join(&dependency.path.value);
                    let candidate =
                        if dependency.kind == DependencyKind::Package && declared.is_dir() {
                            declared.join(MANIFEST_FILE)
                        } else {
                            declared
                        };
                    let actual = if dependency.kind == DependencyKind::Package {
                        discover(base, Some(&candidate))?
                    } else {
                        canonical_file(&candidate)?
                    };
                    let target_key = path_key(&actual)?;
                    links.push(LoadedLink {
                        dependency_index: index,
                        source_key: target_key.clone(),
                    });
                    pending.entry(target_key).or_insert((
                        actual,
                        dependency.kind,
                        Some(dependency.name.value.clone()),
                    ));
                }
                let source_id = if key == root_key {
                    SourceId::Project
                } else {
                    SourceId::Local { path: portable }
                };
                debug!(package_id = %manifest.name, source_count = source_files.len(), dependency_count = links.len(), "loaded source package");
                packages.insert(
                    key.clone(),
                    LoadedPackage {
                        source_key: key,
                        source_id,
                        carrier: LoadedCarrier::Manifest(manifest),
                        source_files,
                        links,
                    },
                );
            }
            DependencyKind::Sdk | DependencyKind::Builtin | DependencyKind::Psc => {
                let (bundle, digest) = if kind == DependencyKind::Builtin {
                    let name = path.to_string_lossy().to_string();
                    let bundle = builtin(&name)
                        .map_err(|reason| LoadError::Builtin {
                            name: name.clone(),
                            reason,
                        })?
                        .ok_or_else(|| LoadError::Builtin {
                            name: name.clone(),
                            reason: "unknown package".into(),
                        })?;
                    let bytes = encode(&bundle).map_err(|cause| LoadError::Builtin {
                        name,
                        reason: cause.to_string(),
                    })?;
                    let digest = blake3::hash(&bytes).to_hex().to_string();
                    (bundle, digest)
                } else if kind == DependencyKind::Psc {
                    if !path.is_dir() {
                        return Err(LoadError::InvalidPath {
                            path,
                            reason: "PSC dependency is not a directory",
                        });
                    }
                    let name = declared_name.expect("PSC dependency has a name");
                    let (_, inputs) = collect_sources(&path, "", &["psc".into()], &key)?;
                    if inputs.is_empty() {
                        return Err(LoadError::InvalidPath {
                            path,
                            reason: "PSC dependency contains no scripts",
                        });
                    }
                    let bundle = psc_declarations(&name, &inputs).map_err(|cause| {
                        LoadError::SourceDeclarations {
                            path: path.clone(),
                            reason: cause.to_string(),
                        }
                    })?;
                    let digest = bundle
                        .package
                        .source_digest
                        .clone()
                        .expect("generator hashes sources");
                    (bundle, digest)
                } else {
                    let bytes =
                        fs::read(&path).map_err(|cause| io("read declaration", &path, cause))?;
                    let digest = blake3::hash(&bytes).to_hex().to_string();
                    let bundle = decode(&bytes).map_err(|cause| LoadError::Declaration {
                        path: path.clone(),
                        cause,
                    })?;
                    (bundle, digest)
                };
                if bundle.naming.language != "papyrus" || bundle.naming.case_sensitive {
                    return Err(LoadError::SdkNaming {
                        path: path.clone(),
                        language: bundle.naming.language.clone(),
                        case_sensitive: bundle.naming.case_sensitive,
                    });
                }
                let scripts = bundle
                    .scripts
                    .iter()
                    .enumerate()
                    .map(|(script_index, script)| DeclaredScript {
                        name: script.name.clone(),
                        location: DeclarationLocation {
                            carrier_path: portable.clone(),
                            script_index,
                            source_path: script.source.as_ref().map(|source| source.path.clone()),
                            line: script.source.as_ref().map(|source| source.line),
                            column: script.source.as_ref().map(|source| source.column),
                        },
                    })
                    .collect();
                let sdk = LoadedSdk {
                    name: bundle.package.name.clone(),
                    version: bundle.package.version.clone(),
                    target: bundle.compatibility.target.clone(),
                    abi: bundle.compatibility.abi.clone(),
                    scripts,
                };
                let source_id = if kind == DependencyKind::Psc {
                    SourceId::Local {
                        path: portable.clone(),
                    }
                } else {
                    SourceId::DeclarationSdk {
                        path: portable.clone(),
                        digest,
                    }
                };
                debug!(package_id = %sdk.name, ?kind, script_count = sdk.scripts.len(), "loaded dependency declarations");
                packages.insert(
                    key.clone(),
                    LoadedPackage {
                        source_key: key.clone(),
                        source_id,
                        carrier: LoadedCarrier::Declarations { kind, sdk },
                        source_files: Vec::new(),
                        links: Vec::new(),
                    },
                );
                bundles.insert(key, bundle);
            }
        }
    }
    source_inputs.sort_by(|left, right| {
        (&left.package_key, &left.canonical_path).cmp(&(&right.package_key, &right.canonical_path))
    });
    Ok(LoadedProject {
        root_key,
        packages: packages.into_values().collect(),
        declaration_bundles: bundles,
        source_inputs,
    })
}

/// Extract a local source directory's API without publishing a carrier file.
fn psc_declarations(
    name: &str,
    inputs: &[LoadedSourceInput],
) -> Result<DeclarationBundle, folio_declaration_tools::GenerationError> {
    let sources = inputs
        .iter()
        .map(|input| SourceInput {
            path: &input.display_path,
            text: &input.text,
        })
        .collect::<Vec<_>>();
    generate(
        GenerationOptions {
            name,
            version: "local",
            source: "local-psc",
        },
        &sources,
    )
}

/// Reject links and non-directory components before using a workspace path.
fn validate_workspace_directory(
    base: &Path,
    relative: &str,
    required: bool,
) -> Result<(), LoadError> {
    let mut path = base.to_owned();
    let mut missing = false;
    for component in Path::new(relative).components() {
        path.push(component.as_os_str());
        if missing {
            continue;
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(LoadError::InvalidPath {
                    path,
                    reason: "workspace path is not a plain directory",
                });
            }
            Ok(_) => {}
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound && !required => {
                missing = true
            }
            Err(cause) => return Err(io("inspect workspace path", &path, cause)),
        }
    }
    Ok(())
}

/// Load and resolve a project, preserving the loaded carriers for semantic use.
pub fn load_and_resolve(
    start: &Path,
    explicit: Option<&Path>,
) -> Result<(LoadedProject, folio_project_model::Metadata), LoadError> {
    let loaded = load(start, explicit)?;
    match graph::resolve(&loaded.root_key, &loaded.packages) {
        Ok(metadata) => Ok((loaded, metadata)),
        Err(cause) => {
            debug!(phase = "resolve", cause = %cause, "project resolution failed");
            Err(LoadError::Resolve(Box::new(cause)))
        }
    }
}

fn collect_sources(
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

fn canonical_file(path: &Path) -> Result<PathBuf, LoadError> {
    fs::canonicalize(path).map_err(|cause| io("resolve path", path, cause))
}

fn path_key(path: &Path) -> Result<String, LoadError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| LoadError::InvalidPath {
            path: path.to_owned(),
            reason: "canonical path is not UTF-8",
        })
}

fn io(operation: &'static str, path: &Path, cause: std::io::Error) -> LoadError {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_keeps_sibling_identity_without_absolute_prefix() {
        let result =
            relative_portable(Path::new("C:/work/app"), Path::new("C:/work/sdk/decl.json"))
                .unwrap();
        assert_eq!(result, "../sdk/decl.json");
    }

    #[test]
    fn psc_directory_inputs_supply_declarations_without_a_carrier_file() {
        let input = LoadedSourceInput {
            package_key: "scripts".into(),
            canonical_path: PathBuf::from("/unused/Actor.psc"),
            display_path: "Actor.psc".into(),
            script_candidate: "Actor".into(),
            text: Arc::from("ScriptName Actor\nInt Function Value(Int count = 2) Native\n"),
        };
        let bundle = psc_declarations("other-mod", &[input]).unwrap();
        assert_eq!(bundle.package.name, "other-mod");
        assert_eq!(bundle.scripts[0].name, "Actor");
        assert_eq!(
            bundle.scripts[0].members[0].parameters[0]
                .default_literal
                .as_deref(),
            Some("2")
        );
    }
}
