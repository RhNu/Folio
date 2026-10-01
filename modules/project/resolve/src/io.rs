//! Filesystem adapters for one root and four parallel declaration inputs.

use crate::{graph, manifest};
use folio_declaration_tools::{GenerationOptions, PexInput, SourceInput, extract_pex, generate};
use folio_format_declarations::{DeclarationBundle, decode, semantic_digest};
use folio_project_model::{
    DeclarationLocation, DeclaredScript, DependencyKind, LoadedDependency, LoadedRoot,
    SourceEncoding, SourceFile, SourceId,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tracing::{debug, info, instrument};

mod repo;
mod scan;
mod watch;
pub use repo::{FolioHome, RepoEntry, publish_declaration};
pub use scan::{
    InputSnapshot, SnapshotEntry, SnapshotEntryKind, decode_source, relative_portable,
    verify_snapshots,
};
use scan::{canonical_file, collect_sources, io, path_key, read_bytes};
pub use watch::{WatchPlan, watch_plan, watch_plan_with_home};

pub const MANIFEST_FILE: &str = "folio.toml";

/// Every dependency occurrence owns a declaration projection; root sources are separate.
#[derive(Clone)]
pub struct LoadedProject {
    pub root_key: String,
    pub root: LoadedRoot,
    pub dependencies: Vec<LoadedDependency>,
    pub declaration_bundles: BTreeMap<String, DeclarationBundle>,
    pub source_inputs: Vec<LoadedSourceInput>,
    pub input_snapshots: Vec<InputSnapshot>,
    pub watch_plan: WatchPlan,
    pub folio_home: FolioHome,
}

#[derive(Clone)]
pub struct LoadedSourceInput {
    pub package_key: String,
    pub canonical_path: PathBuf,
    pub display_path: String,
    pub script_candidate: String,
    pub text: Arc<str>,
}

pub struct RootSources {
    pub language: String,
    pub dialect: String,
    pub inputs: Vec<LoadedSourceInput>,
}

/// Load only editable sources for tools that do not require dependency APIs.
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
        SourceEncoding::Utf8,
        &mut Vec::new(),
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
    PexDeclarations {
        path: PathBuf,
        reason: String,
    },
    ExperimentalDependency {
        name: String,
    },
    RepoMissing {
        repo: PathBuf,
        key: String,
        candidates: Vec<PathBuf>,
    },
    RepoAmbiguous {
        key: String,
        candidates: Vec<PathBuf>,
    },
    InputChanged(PathBuf),
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
            Self::SourceDeclarations { path, reason } | Self::PexDeclarations { path, reason } => {
                write!(f, "{}: {reason}", path.display())
            }
            Self::ExperimentalDependency { name } => write!(
                f,
                "PEX dependency {name} requires [experimental] pex-dependencies = true"
            ),
            Self::RepoMissing {
                repo,
                key,
                candidates,
            } => {
                write!(
                    f,
                    "repository declaration {key} not found in {}; tried",
                    repo.display()
                )?;
                for candidate in candidates {
                    write!(f, " {}", candidate.display())?;
                }
                Ok(())
            }
            Self::RepoAmbiguous { key, candidates } => {
                write!(
                    f,
                    "repository declaration {key} is ambiguous; specify .json or .fdecl explicitly:"
                )?;
                for candidate in candidates {
                    write!(f, " {}", candidate.display())?;
                }
                Ok(())
            }
            Self::InputChanged(path) => {
                write!(f, "input changed during operation: {}", path.display())
            }
            Self::Resolve(cause) => write!(f, "{cause}"),
            Self::InvalidPath { path, reason } => write!(f, "{}: {reason}", path.display()),
        }
    }
}
impl std::error::Error for LoadError {}

/// Find the nearest manifest, or use an explicitly selected manifest.
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
    for entry in
        fs::read_dir(directory).map_err(|cause| io("read project directory", directory, cause))?
    {
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

pub fn load(start: &Path, explicit: Option<&Path>) -> Result<LoadedProject, LoadError> {
    load_with_home(start, explicit, &FolioHome::from_env()?)
}

/// Read all dependency modes through the same normalized declaration boundary.
#[instrument(name = "project.load", skip_all, fields(phase = "load"))]
pub fn load_with_home(
    start: &Path,
    explicit: Option<&Path>,
    home: &FolioHome,
) -> Result<LoadedProject, LoadError> {
    let watch_plan = watch_plan_with_home(start, explicit, home);
    let path = discover(start, explicit)?;
    let base = path.parent().expect("canonical manifest has parent");
    let root_key = path_key(&path)?;
    let mut snapshots = Vec::new();
    let bytes = read_bytes(&path, &mut snapshots)?;
    let text = std::str::from_utf8(&bytes).map_err(|cause| LoadError::SourceDeclarations {
        path: path.clone(),
        reason: format!("manifest is not UTF-8: {cause}"),
    })?;
    let manifest = manifest::parse(MANIFEST_FILE, text.strip_prefix('\u{feff}').unwrap_or(text))
        .map_err(LoadError::Manifest)?;
    validate_workspace_directory(base, &manifest.source_path.value, true)?;
    validate_workspace_directory(base, &manifest.output_path.value, false)?;
    let (source_files, source_inputs) = collect_sources(
        base,
        &manifest.source_path.value,
        &manifest.extensions,
        &root_key,
        SourceEncoding::Utf8,
        &mut snapshots,
    )?;
    info!(manifest = %path.display(), dependency_count = manifest.dependencies.len(), "loading root and declaration dependencies");
    let mut dependencies = Vec::new();
    let mut bundles = BTreeMap::new();
    for (index, specification) in manifest.dependencies.iter().enumerate() {
        if specification.kind == DependencyKind::Pex && !manifest.experimental_pex_dependencies {
            return Err(LoadError::ExperimentalDependency {
                name: specification.name.value.clone(),
            });
        }
        let declared = &specification.path.value;
        let canonical_path = if specification.kind == DependencyKind::Repo {
            home.locate(declared, &mut snapshots)?
        } else {
            let path = base.join(declared);
            let canonical_path = canonical_file(&path)?;
            snapshots.push(InputSnapshot::Resolution {
                path,
                canonical_path: canonical_path.clone(),
            });
            canonical_path
        };
        let source_key = format!("dependency:{index}");
        // The relative host path may traverse parents; provenance remains a portable label.
        let source_label = format!("dependency/{index}");
        let (bundle, pex_paths) = match specification.kind {
            DependencyKind::Psc => {
                let (_, inputs) = collect_sources(
                    &canonical_path,
                    "",
                    &["psc".into()],
                    &source_key,
                    specification.encoding,
                    &mut snapshots,
                )?;
                if inputs.is_empty() {
                    return Err(LoadError::InvalidPath {
                        path: canonical_path,
                        reason: "PSC dependency contains no scripts",
                    });
                }
                let bundle = psc_declarations(&source_label, &inputs).map_err(|cause| {
                    LoadError::SourceDeclarations {
                        path: canonical_path.clone(),
                        reason: cause.to_string(),
                    }
                })?;
                (bundle, None)
            }
            DependencyKind::Pex => {
                let files = scan::collect_files(&canonical_path, &["pex".into()], &mut snapshots)?;
                let mut binary_inputs = Vec::new();
                for (relative, path) in files {
                    binary_inputs.push((relative, read_bytes(&path, &mut snapshots)?));
                }
                let inputs = binary_inputs
                    .iter()
                    .map(|(path, bytes)| PexInput { path, bytes })
                    .collect::<Vec<_>>();
                let extracted = extract_pex(&source_label, &inputs).map_err(|reason| {
                    LoadError::PexDeclarations {
                        path: canonical_path.clone(),
                        reason,
                    }
                })?;
                (extracted.bundle, Some(extracted.paths))
            }
            DependencyKind::Decl | DependencyKind::Repo => {
                if !canonical_path.is_file() {
                    return Err(LoadError::InvalidPath {
                        path: canonical_path,
                        reason: "declaration dependency is not a file",
                    });
                }
                let bytes = read_bytes(&canonical_path, &mut snapshots)?;
                let bundle = decode(&bytes).map_err(|cause| LoadError::Declaration {
                    path: canonical_path.clone(),
                    cause,
                })?;
                (bundle, None)
            }
        };
        let carrier = if specification.kind == DependencyKind::Repo {
            format!("repo:{declared}")
        } else {
            declared.clone()
        };
        let scripts = bundle
            .scripts
            .iter()
            .enumerate()
            .map(|(index, script)| DeclaredScript {
                name: script.name.clone(),
                location: DeclarationLocation {
                    carrier_path: pex_paths.as_ref().map_or_else(
                        || carrier.clone(),
                        |paths| format!("{carrier}/{}", paths[index]),
                    ),
                    script_name: script.name.to_ascii_lowercase(),
                    source_path: script.source.as_ref().map(|source| source.path.clone()),
                    line: script.source.as_ref().map(|source| source.line),
                    column: script.source.as_ref().map(|source| source.column),
                },
            })
            .collect();
        let source_id = SourceId::Dependency {
            index,
            kind: specification.kind,
            path: declared.clone(),
            digest: semantic_digest(&bundle),
        };
        debug!(name = %specification.name.value, kind = ?specification.kind, script_count = bundle.scripts.len(), "normalized dependency declarations");
        dependencies.push(LoadedDependency {
            source_key: source_key.clone(),
            source_id,
            kind: specification.kind,
            name: specification.name.value.clone(),
            declared_path: declared.clone(),
            canonical_path,
            declaration: specification.path.span.clone(),
            profile: bundle.profile.clone(),
            scripts,
        });
        bundles.insert(source_key, bundle);
    }
    let root = LoadedRoot {
        source_key: root_key.clone(),
        source_id: SourceId::Project,
        manifest,
        source_files,
    };
    verify_snapshots(&snapshots)?;
    Ok(LoadedProject {
        root_key,
        root,
        dependencies,
        declaration_bundles: bundles,
        source_inputs,
        input_snapshots: snapshots,
        watch_plan,
        folio_home: home.clone(),
    })
}

/// Shared PSC directory loading for direct dependencies and generation commands.
pub fn read_psc_sources(
    root: &Path,
    encoding: SourceEncoding,
) -> Result<Vec<LoadedSourceInput>, LoadError> {
    let mut snapshots = Vec::new();
    let (_, inputs) = collect_sources(
        root,
        "",
        &["psc".into()],
        "generation",
        encoding,
        &mut snapshots,
    )?;
    if inputs.is_empty() {
        return Err(LoadError::InvalidPath {
            path: root.to_owned(),
            reason: "PSC source directory contains no scripts",
        });
    }
    verify_snapshots(&snapshots)?;
    Ok(inputs)
}

pub fn generate_psc_directory(
    root: &Path,
    source: &str,
    encoding: SourceEncoding,
) -> Result<DeclarationBundle, LoadError> {
    let mut snapshots = Vec::new();
    let (_, inputs) = collect_sources(
        root,
        "",
        &["psc".into()],
        "generation",
        encoding,
        &mut snapshots,
    )?;
    if inputs.is_empty() {
        return Err(LoadError::InvalidPath {
            path: root.to_owned(),
            reason: "PSC source directory contains no scripts",
        });
    }
    let bundle =
        psc_declarations(source, &inputs).map_err(|cause| LoadError::SourceDeclarations {
            path: root.to_owned(),
            reason: cause.to_string(),
        })?;
    verify_snapshots(&snapshots)?;
    Ok(bundle)
}

fn psc_declarations(
    source: &str,
    inputs: &[LoadedSourceInput],
) -> Result<DeclarationBundle, folio_declaration_tools::GenerationError> {
    let sources = inputs
        .iter()
        .map(|input| SourceInput {
            path: &input.display_path,
            text: &input.text,
        })
        .collect::<Vec<_>>();
    generate(GenerationOptions { source }, &sources)
}

/// Root source and output directories never traverse links or non-directory components.
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

pub fn load_and_resolve(
    start: &Path,
    explicit: Option<&Path>,
) -> Result<(LoadedProject, folio_project_model::Metadata), LoadError> {
    load_and_resolve_with_home(start, explicit, &FolioHome::from_env()?)
}

pub fn load_and_resolve_with_home(
    start: &Path,
    explicit: Option<&Path>,
    home: &FolioHome,
) -> Result<(LoadedProject, folio_project_model::Metadata), LoadError> {
    let loaded = load_with_home(start, explicit, home)?;
    let metadata = graph::resolve(&loaded.root, &loaded.dependencies).map_err(|cause| {
        debug!(phase = "resolve", cause = %cause, "project resolution failed");
        LoadError::Resolve(Box::new(cause))
    })?;
    Ok((loaded, metadata))
}
