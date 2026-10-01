//! Folio process entry point and common error/logging boundary.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::AtomicBool,
};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use folio_build::{
    ProjectAnalysis, ProjectAnalysisView, ProjectionError,
    execute::{self, BuildError},
    output::{self, OutputError},
};
use folio_diagnostics::{Diagnostic, Severity};
use folio_format::format_source;
use folio_lint::{LintConfig, lint_script};
use folio_papyrus::PapyrusDialect;
use folio_project_model::{
    DependencyEdge, ExternalRequirement, Metadata, PackageId, ScriptSelection,
};
use folio_project_resolve::{
    graph::ResolveErrorKind,
    io::{LoadError, load_root_sources},
    load_and_resolve,
    manifest::ManifestErrorKind,
};
use folio_source::SourceSpan;
use serde::Serialize;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
    Json,
    Text,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SourceEncoding {
    Utf8,
    Windows1252,
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    /// Generate or list local API declarations.
    Declarations {
        #[command(subcommand)]
        command: DeclarationsCommand,
    },
    /// Build selected project scripts into managed PEX generations.
    Build {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Create a Folio project in the current directory.
    Init {
        #[arg(long)]
        name: Option<String>,
    },
    /// Create a new Folio project directory.
    New {
        name: String,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Inspect the last verified successful build and artifact provenance.
    Inspect {
        /// Inspect a PEX file without loading a Folio project.
        #[arg(long)]
        pex: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Serve the current project over Language Server Protocol stdio.
    Lsp,
    /// Analyze project sources and target feasibility; final PEX layout is checked by build.
    Check {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Check or explicitly write the root project's Papyrus source formatting.
    Fmt {
        #[arg(long, conflicts_with = "write")]
        check: bool,
        #[arg(long)]
        write: bool,
    },
    /// Report optional style rules over the selected project's source.
    Lint {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Print the versioned resolved project model.
    Metadata {
        #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
        format: OutputFormat,
    },
    /// Explain declared dependencies and whole-script provider decisions.
    Tree {
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum DeclarationOutputFormat {
    Json,
    Binary,
}

impl From<DeclarationOutputFormat> for folio_format_declarations::DeclarationFormat {
    fn from(value: DeclarationOutputFormat) -> Self {
        match value {
            DeclarationOutputFormat::Json => Self::Json,
            DeclarationOutputFormat::Binary => Self::Binary,
        }
    }
}

#[derive(Debug, Subcommand)]
enum DeclarationsCommand {
    /// List declaration files in the local Folio repository.
    List,
    /// Extract PSC API declarations into a portable carrier.
    Generate {
        #[arg(long)]
        source_root: PathBuf,
        #[arg(long)]
        source: String,
        #[arg(long, value_enum, default_value_t = SourceEncoding::Utf8)]
        encoding: SourceEncoding,
        #[arg(long, required_unless_present = "repo", conflicts_with = "repo")]
        output: Option<PathBuf>,
        #[arg(long, required_unless_present = "output", conflicts_with = "output")]
        repo: Option<String>,
        #[arg(long, value_enum, default_value_t = DeclarationOutputFormat::Binary)]
        format: DeclarationOutputFormat,
    },
}

/// Project commands share discovery and one resolver implementation.
#[derive(Debug, Parser)]
#[command(name = "folio", version, about = "Folio project toolchain")]
struct Cli {
    /// Use a specific folio.toml instead of searching parent directories.
    #[arg(long, global = true)]
    manifest_path: Option<PathBuf>,
    /// Logging filter, using tracing-subscriber directive syntax.
    #[arg(long, default_value = "warn", global = true)]
    log_filter: String,
    /// Log event output format on stderr.
    #[arg(long, value_enum, default_value_t = LogFormat::Text, global = true)]
    log_format: LogFormat,
    #[command(subcommand)]
    command: Option<ProjectCommand>,
}

/// Stable process failure classification for command handlers.
#[derive(Debug)]
enum CliError {
    InvalidLogFilter(String),
    LoggingSetup(String),
    Help(std::io::Error),
    CurrentDirectory(std::io::Error),
    Project(LoadError),
    Analysis(ProjectionError),
    Build(BuildError),
    ManagedOutput(OutputError),
    ProjectCreation(String),
    Declarations(String),
    Format(String),
    Lint(String),
    Pex(String),
    Lsp(String),
    Json(serde_json::Error),
    Output(std::io::Error),
}

impl CliError {
    fn code(&self) -> &'static str {
        match self {
            Self::InvalidLogFilter(_) => "CLI001",
            Self::LoggingSetup(_) => "CLI002",
            Self::Help(_) => "CLI003",
            Self::CurrentDirectory(_) => "CLI004",
            Self::Project(error) => project_error_code(error),
            Self::Analysis(_) => "ANALYSIS001",
            Self::Build(_) => "BUILD001",
            Self::ManagedOutput(_) => "OUTPUT001",
            Self::ProjectCreation(_) => "PROJECT002",
            Self::Declarations(_) => "DECL001",
            Self::Format(_) => "FORMAT001",
            Self::Lint(_) => "LINT001",
            Self::Pex(_) => "PEX001",
            Self::Lsp(_) => "LSP001",
            Self::Json(_) => "CLI005",
            Self::Output(_) => "CLI006",
        }
    }

    fn exit_code(&self) -> ExitCode {
        match self {
            Self::InvalidLogFilter(_)
            | Self::Project(
                LoadError::Manifest(_)
                | LoadError::Declaration { .. }
                | LoadError::SourceDeclarations { .. }
                | LoadError::PexDeclarations { .. }
                | LoadError::ExperimentalDependency { .. }
                | LoadError::Resolve(_)
                | LoadError::InvalidPath { .. }
                | LoadError::NotFound(_)
                | LoadError::RepoMissing { .. }
                | LoadError::RepoAmbiguous { .. }
                | LoadError::InputChanged(_),
            ) => ExitCode::from(2),
            Self::Analysis(_) => ExitCode::from(2),
            Self::Build(BuildError::Diagnostics(_) | BuildError::Plan(_))
            | Self::ProjectCreation(_)
            | Self::Format(_)
            | Self::Lint(_) => ExitCode::from(2),
            Self::Pex(_) => ExitCode::from(2),
            Self::Lsp(_) => ExitCode::FAILURE,
            _ => ExitCode::FAILURE,
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLogFilter(reason) => write!(formatter, "invalid log filter: {reason}"),
            Self::LoggingSetup(reason) => {
                write!(formatter, "could not initialize logging: {reason}")
            }
            Self::Help(reason) => write!(formatter, "could not print help: {reason}"),
            Self::CurrentDirectory(reason) => {
                write!(formatter, "could not read working directory: {reason}")
            }
            Self::Project(reason) => write!(formatter, "{reason}"),
            Self::Analysis(reason) => write!(formatter, "{reason}"),
            Self::Build(reason) => write!(formatter, "{reason}"),
            Self::ManagedOutput(reason) => write!(formatter, "{reason}"),
            Self::ProjectCreation(reason) => write!(formatter, "{reason}"),
            Self::Declarations(reason) => write!(formatter, "{reason}"),
            Self::Format(reason) => write!(formatter, "{reason}"),
            Self::Lint(reason) => write!(formatter, "{reason}"),
            Self::Pex(reason) => write!(formatter, "{reason}"),
            Self::Lsp(reason) => write!(formatter, "{reason}"),
            Self::Json(reason) => write!(formatter, "could not encode JSON output: {reason}"),
            Self::Output(reason) => write!(formatter, "could not write output: {reason}"),
        }
    }
}

/// Preserve the category of each user project failure at the process boundary.
fn project_error_code(error: &LoadError) -> &'static str {
    match error {
        LoadError::NotFound(_) => "PROJECT001",
        LoadError::Manifest(manifest) => match manifest.kind {
            ManifestErrorKind::Toml(_) => "MANIFEST001",
            ManifestErrorKind::InvalidValue { .. } => "MANIFEST003",
        },
        LoadError::Declaration { .. } => "DECL001",
        LoadError::SourceDeclarations { .. } => "PSC001",
        LoadError::PexDeclarations { .. } => "PEX001",
        LoadError::ExperimentalDependency { .. } => "PEX002",
        LoadError::Resolve(resolve) => match &resolve.kind {
            ResolveErrorKind::DuplicateDependencyAlias(_) => "RESOLVE002",
            ResolveErrorKind::ProfileMismatch { .. } => "DECL003",
            ResolveErrorKind::ScriptConflict(_) => "RESOLVE004",
            _ => "RESOLVE001",
        },
        LoadError::InvalidPath { .. } => "PATH001",
        LoadError::Io { .. } => "IO001",
        LoadError::RepoMissing { .. } => "REPO001",
        LoadError::RepoAmbiguous { .. } => "REPO002",
        LoadError::InputChanged(_) => "INPUT001",
    }
}

/// Computes every candidate before a write, so invalid syntax never causes partial formatting.
fn run_fmt(
    output: &mut impl Write,
    cwd: &Path,
    manifest: Option<&Path>,
    write: bool,
) -> Result<ExitCode, CliError> {
    let roots = load_root_sources(cwd, manifest).map_err(CliError::Project)?;
    if !roots.language.eq_ignore_ascii_case("papyrus")
        || !roots.dialect.eq_ignore_ascii_case("skyrim")
    {
        return Err(CliError::Format(format!(
            "unsupported formatter language/dialect: {}/{}",
            roots.language, roots.dialect
        )));
    }
    let mut changes = Vec::new();
    let mut invalid = false;
    for source in roots.inputs {
        match format_source(&source.text, PapyrusDialect::Skyrim) {
            Ok(candidate) if candidate != source.text.as_ref() => {
                changes.push((source.canonical_path, source.text, candidate));
            }
            Ok(_) => {}
            Err(folio_format::FormatError::Syntax(errors)) => {
                invalid = true;
                let offset = errors.first().map_or(0, |error| error.range.start);
                writeln!(
                    output,
                    "cannot format {} at byte {offset}: invalid syntax",
                    source.display_path
                )
                .map_err(CliError::Output)?;
            }
            Err(folio_format::FormatError::ChangedTokens) => {
                invalid = true;
                writeln!(output, "cannot safely format {}", source.display_path)
                    .map_err(CliError::Output)?;
            }
        }
    }
    tracing::info!(
        changed = changes.len(),
        invalid,
        write,
        "formatting candidates computed"
    );
    if invalid {
        return Ok(ExitCode::from(2));
    }
    if write {
        // Verify the whole batch against the loaded revision before touching any source.
        for (path, original, _) in &changes {
            let current = fs::read_to_string(path).map_err(CliError::Output)?;
            if current != original.as_ref() {
                return Err(CliError::Format(format!(
                    "source changed during formatting: {}",
                    path.display()
                )));
            }
        }
        for (path, _, candidate) in &changes {
            fs::write(path, candidate).map_err(CliError::Output)?;
            tracing::info!(path = %path.display(), "formatted source written");
            writeln!(output, "formatted {}", path.display()).map_err(CliError::Output)?;
        }
        Ok(ExitCode::SUCCESS)
    } else {
        for (path, _, _) in &changes {
            writeln!(output, "needs formatting: {}", path.display()).map_err(CliError::Output)?;
        }
        Ok(if changes.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(2)
        })
    }
}

/// Delegate source decoding and publication to the shared project I/O boundary.
fn run_declarations_generate(
    source_root: &Path,
    source: &str,
    encoding: SourceEncoding,
    output: Option<&Path>,
    repo: Option<&str>,
    format: DeclarationOutputFormat,
) -> Result<(usize, PathBuf), CliError> {
    use folio_project_resolve::io::{FolioHome, generate_psc_directory, publish_declaration};
    let encoding = match encoding {
        SourceEncoding::Utf8 => folio_project_model::SourceEncoding::Utf8,
        SourceEncoding::Windows1252 => folio_project_model::SourceEncoding::Windows1252,
    };
    let format = format.into();
    let (path, home) = match (output, repo) {
        (Some(path), None) => (path.to_owned(), None),
        (None, Some(key)) => {
            let home = FolioHome::from_env().map_err(CliError::Project)?;
            let path = home.repo_output(key, format).map_err(CliError::Project)?;
            (path, Some(home))
        }
        _ => {
            return Err(CliError::Declarations(
                "choose exactly one of --output or --repo".into(),
            ));
        }
    };
    let bundle =
        generate_psc_directory(source_root, source, encoding).map_err(CliError::Project)?;
    let bytes = folio_format_declarations::encode(&bundle, format)
        .map_err(|error| CliError::Declarations(error.to_string()))?;
    let path = if let (Some(home), Some(key)) = (home, repo) {
        home.publish_repo(key, &bytes, format)
            .map_err(CliError::Project)?
    } else {
        publish_declaration(&path, &bytes).map_err(CliError::Project)?;
        path
    };
    info!(source, scripts = bundle.scripts.len(), bytes = bytes.len(), output = %path.display(), "declarations published");
    Ok((bundle.scripts.len(), path))
}

fn run(cli: Cli) -> Result<ExitCode, CliError> {
    let filter = EnvFilter::try_new(&cli.log_filter)
        .map_err(|error| CliError::InvalidLogFilter(error.to_string()))?;
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    match cli.log_format {
        LogFormat::Text => subscriber
            .try_init()
            .map_err(|error| CliError::LoggingSetup(error.to_string()))?,
        LogFormat::Json => subscriber
            .json()
            .try_init()
            .map_err(|error| CliError::LoggingSetup(error.to_string()))?,
    }
    info!(phase = "cli.start", "CLI invocation started");
    if matches!(cli.command.as_ref(), Some(ProjectCommand::Lsp)) {
        folio_lsp::serve_stdio(cli.manifest_path.as_deref())
            .map_err(|error| CliError::Lsp(error.to_string()))?;
        info!(phase = "cli.complete", "LSP session completed");
        return Ok(ExitCode::SUCCESS);
    }
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let mut result = ExitCode::SUCCESS;
    match cli.command {
        None => {
            Cli::command()
                .write_help(&mut output)
                .map_err(CliError::Help)?;
            writeln!(output).map_err(CliError::Output)?;
        }
        Some(ProjectCommand::Init { name }) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDirectory)?;
            create_project(&cwd, name.as_deref())?;
            writeln!(output, "created {}", cwd.display()).map_err(CliError::Output)?;
        }
        Some(ProjectCommand::New { name, path }) => {
            scaffold(&name)?;
            let cwd = std::env::current_dir().map_err(CliError::CurrentDirectory)?;
            let directory = path.unwrap_or_else(|| cwd.join(&name));
            fs::create_dir(&directory).map_err(|cause| {
                CliError::ProjectCreation(format!("create {}: {cause}", directory.display()))
            })?;
            create_project(&directory, Some(&name))?;
            writeln!(output, "created {}", directory.display()).map_err(CliError::Output)?;
        }
        Some(ProjectCommand::Inspect {
            pex: Some(path),
            format,
        }) => {
            inspect_pex(&mut output, &path, format)?;
        }
        Some(ProjectCommand::Fmt { check: _, write }) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDirectory)?;
            result = run_fmt(&mut output, &cwd, cli.manifest_path.as_deref(), write)?;
        }
        Some(ProjectCommand::Declarations {
            command:
                DeclarationsCommand::Generate {
                    source_root,
                    source,
                    encoding,
                    output: path,
                    repo,
                    format,
                },
        }) => {
            let (count, path) = run_declarations_generate(
                &source_root,
                &source,
                encoding,
                path.as_deref(),
                repo.as_deref(),
                format,
            )?;
            writeln!(
                output,
                "generated {count} script declarations -> {}",
                path.display()
            )
            .map_err(CliError::Output)?;
        }
        Some(ProjectCommand::Declarations {
            command: DeclarationsCommand::List,
        }) => {
            let home =
                folio_project_resolve::io::FolioHome::from_env().map_err(CliError::Project)?;
            for entry in home.list_repo().map_err(CliError::Project)? {
                writeln!(
                    output,
                    "{} [{}; {} scripts; {}]",
                    entry.key, entry.profile, entry.scripts, entry.source
                )
                .map_err(CliError::Output)?;
            }
        }
        Some(command) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDirectory)?;
            let (loaded, metadata) =
                load_and_resolve(&cwd, cli.manifest_path.as_deref()).map_err(CliError::Project)?;
            let root = Path::new(&loaded.root_key).parent().ok_or_else(|| {
                CliError::ProjectCreation("manifest has no parent directory".into())
            })?;
            match command {
                ProjectCommand::Lsp => unreachable!("handled before locking stdout"),
                ProjectCommand::Init { .. } | ProjectCommand::New { .. } => {
                    unreachable!("handled before project loading")
                }
                ProjectCommand::Fmt { .. } => unreachable!("handled before project loading"),
                ProjectCommand::Declarations { .. } => {
                    unreachable!("handled before project loading")
                }
                ProjectCommand::Check { format } => {
                    let mut service = ProjectAnalysis::new();
                    let view = service
                        .sync_project(&loaded, &metadata)
                        .map_err(CliError::Analysis)?;
                    let mut diagnostics = view.diagnostics();
                    if !diagnostics
                        .iter()
                        .any(|item| item.severity == Severity::Error)
                    {
                        match execute::check_target(&loaded, &metadata, &view) {
                            Ok(()) => {}
                            Err(BuildError::Diagnostics(mut issues)) => {
                                diagnostics.append(&mut issues)
                            }
                            Err(cause) => return Err(CliError::Build(cause)),
                        }
                    }
                    let report = CheckReport::new(&metadata.target, &view, &diagnostics);
                    match format {
                        OutputFormat::Json => write_json(&mut output, &report)?,
                        OutputFormat::Text => print_check(&mut output, &report, "check")?,
                    }
                    if diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.severity == Severity::Error)
                    {
                        result = ExitCode::from(2);
                    }
                }
                ProjectCommand::Lint { format } => {
                    let root_manifest = &loaded.root.manifest;
                    let config = LintConfig::from_rules(&root_manifest.lint_rules)
                        .map_err(|error| CliError::Lint(error.to_string()))?;
                    let mut service = ProjectAnalysis::new();
                    let view = service
                        .sync_project(&loaded, &metadata)
                        .map_err(CliError::Analysis)?;
                    let mut diagnostics = Vec::new();
                    for (&file, source) in &view.sources {
                        if source.package_key != loaded.root_key {
                            continue;
                        }
                        if let Some(script) = view.analysis.hir(file) {
                            diagnostics.extend(lint_script(&script, &config));
                        }
                    }
                    let report = CheckReport::new(&metadata.target, &view, &diagnostics);
                    match format {
                        OutputFormat::Json => write_json(&mut output, &report)?,
                        OutputFormat::Text => print_check(&mut output, &report, "lint")?,
                    }
                    if diagnostics
                        .iter()
                        .any(|item| item.severity == Severity::Error)
                    {
                        result = ExitCode::from(2);
                    }
                    info!(diagnostics = diagnostics.len(), "project lint completed");
                }
                ProjectCommand::Build { format } => {
                    let mut service = ProjectAnalysis::new();
                    let view = service
                        .sync_project(&loaded, &metadata)
                        .map_err(CliError::Analysis)?;
                    let exe = std::env::current_exe().map_err(|cause| {
                        CliError::ProjectCreation(format!("locate compiler executable: {cause}"))
                    })?;
                    let executable_bytes = fs::read(&exe).map_err(|cause| {
                        CliError::ProjectCreation(format!(
                            "read compiler executable {}: {cause}",
                            exe.display()
                        ))
                    })?;
                    let compiler_identity = blake3::hash(&executable_bytes).to_hex().to_string();
                    let cancellation = AtomicBool::new(false);
                    match execute::build_project(
                        &loaded,
                        &metadata,
                        &view,
                        root,
                        &compiler_identity,
                        &cancellation,
                    ) {
                        Ok(outcome) => {
                            let warnings = view
                                .diagnostics()
                                .into_iter()
                                .filter(|item| item.severity != Severity::Error)
                                .collect::<Vec<_>>();
                            let report = CheckReport::new(&metadata.target, &view, &warnings);
                            match format {
                                OutputFormat::Json => {
                                    write_build_json(&mut output, &outcome, &report)?
                                }
                                OutputFormat::Text => {
                                    if !warnings.is_empty() {
                                        print_check(&mut output, &report, "build")?;
                                    }
                                    print_build(&mut output, &outcome)?;
                                }
                            }
                        }
                        Err(BuildError::Diagnostics(issues)) => {
                            let report = CheckReport::new(&metadata.target, &view, &issues);
                            match format {
                                OutputFormat::Json => write_json(&mut output, &report)?,
                                OutputFormat::Text => print_check(&mut output, &report, "check")?,
                            }
                            result = ExitCode::from(2);
                        }
                        Err(cause) => return Err(CliError::Build(cause)),
                    }
                }
                ProjectCommand::Inspect { pex: None, format } => {
                    let build_root = output::existing_build_root(root)
                        .map_err(CliError::ManagedOutput)?
                        .ok_or_else(|| {
                            CliError::ProjectCreation("no successful build is recorded".into())
                        })?;
                    let index = output::read_index(&build_root).map_err(CliError::ManagedOutput)?;
                    if index.invalid {
                        return Err(CliError::ProjectCreation(
                            "last successful build index is corrupt or its artifacts are missing"
                                .into(),
                        ));
                    }
                    let Some(success) = index.result else {
                        return Err(CliError::ProjectCreation(
                            "no successful build is recorded".into(),
                        ));
                    };
                    if !output::verify_published(root, &success).map_err(CliError::ManagedOutput)? {
                        return Err(CliError::ProjectCreation(
                            "published output is missing or modified".into(),
                        ));
                    }
                    match format {
                        OutputFormat::Json => write_json(&mut output, &success)?,
                        OutputFormat::Text => print_success(&mut output, &success)?,
                    }
                }
                ProjectCommand::Inspect { pex: Some(_), .. } => {
                    unreachable!("handled without project loading")
                }
                ProjectCommand::Metadata {
                    format: OutputFormat::Json,
                } => write_json(&mut output, &metadata)?,
                ProjectCommand::Metadata {
                    format: OutputFormat::Text,
                } => print_metadata(&mut output, &metadata)?,
                ProjectCommand::Tree {
                    format: OutputFormat::Json,
                } => write_json(&mut output, &TreeView::from(&metadata))?,
                ProjectCommand::Tree {
                    format: OutputFormat::Text,
                } => print_tree(&mut output, &metadata)?,
            }
        }
    }
    output.flush().map_err(CliError::Output)?;
    info!(phase = "cli.complete", "CLI invocation completed");
    Ok(result)
}

fn create_project(directory: &Path, explicit_name: Option<&str>) -> Result<(), CliError> {
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
fn scaffold(name: &str) -> Result<(String, String), CliError> {
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

fn print_build(output: &mut impl Write, outcome: &execute::BuildOutcome) -> Result<(), CliError> {
    print_success(output, &outcome.success)?;
    if outcome.unchanged {
        writeln!(output, "  inputs and published output unchanged").map_err(CliError::Output)?;
    }
    for (unit, decision) in &outcome.cache_decisions {
        writeln!(output, "  cache {unit}: {decision:?}").map_err(CliError::Output)?;
    }
    Ok(())
}

fn write_build_json(
    output: &mut impl Write,
    outcome: &execute::BuildOutcome,
    diagnostics: &CheckReport,
) -> Result<(), CliError> {
    let cache = outcome
        .cache_decisions
        .iter()
        .map(|(unit, decision)| {
            let (status, reason) = match decision {
                output::CacheDecision::Hit => ("hit", None),
                output::CacheDecision::Missing => ("miss", Some("not present")),
                output::CacheDecision::Invalid(reason) => ("miss", Some(*reason)),
            };
            serde_json::json!({"unit_id": unit, "status": status, "reason": reason})
        })
        .collect::<Vec<_>>();
    let report = serde_json::json!({"schema": 4, "success": outcome.success, "cache": cache, "unchanged": outcome.unchanged, "diagnostics": diagnostics.diagnostics});
    write_json(output, &report)
}

fn print_success(output: &mut impl Write, success: &output::SuccessRecord) -> Result<(), CliError> {
    writeln!(output, "{} build unit(s) succeeded", success.units.len())
        .map_err(CliError::Output)?;
    for unit in &success.units {
        writeln!(
            output,
            "  {} {} {} -> {}",
            unit.package, unit.package_version, unit.target, success.output
        )
        .map_err(CliError::Output)?;
        for artifact in &unit.artifacts {
            writeln!(
                output,
                "    {} <- {}:{}",
                artifact.path, artifact.source_package, artifact.source_path
            )
            .map_err(CliError::Output)?;
        }
    }
    for requirement in &success.external_requirements {
        writeln!(
            output,
            "  external {} {}: {}",
            requirement.package,
            requirement.package_version.as_deref().unwrap_or(""),
            requirement.reason
        )
        .map_err(CliError::Output)?;
    }
    Ok(())
}

fn inspect_pex(output: &mut impl Write, path: &Path, format: OutputFormat) -> Result<(), CliError> {
    let bytes = fs::read(path)
        .map_err(|cause| CliError::Pex(format!("read {}: {cause}", path.display())))?;
    let file = folio_format_pex::PexFile::read_from_slice(&bytes)
        .map_err(|cause| CliError::Pex(format!("inspect {}: {cause}", path.display())))?;
    match format {
        OutputFormat::Text => {
            writeln!(output, "{}", file.metadata_dump()).map_err(CliError::Output)?;
        }
        OutputFormat::Json => {
            let header = file.header();
            let objects = file
                .objects
                .iter()
                .map(|object| {
                    serde_json::json!({
                        "name": file.resolve_string(object.name),
                        "parent": file.resolve_string(object.parent_class_name),
                        "variables": object.variables.len(),
                        "properties": object.properties.len(),
                        "states": object.states.len(),
                    })
                })
                .collect::<Vec<_>>();
            let report = serde_json::json!({
                "schema": 1,
                "target": header.target().id(),
                "version": {"major": header.pex_version().major(), "minor": header.pex_version().minor()},
                "source_file_name": header.source_file_name(),
                "strings": file.string_table().len(),
                "user_flags": file.user_flags.len(),
                "debug_functions": file.debug_info.as_ref().map_or(0, |info| info.functions.len()),
                "objects": objects,
            });
            write_json(output, &report)?;
        }
    }
    info!(path = %path.display(), bytes = bytes.len(), objects = file.objects.len(), "inspected PEX");
    Ok(())
}

#[cfg(test)]
mod scaffold_tests {
    use super::scaffold;

    #[test]
    fn names_produce_valid_nonreserved_starter_scripts() {
        let (script, manifest) = scaffold("CON").unwrap();
        assert_eq!(script, "FolioCON");
        assert!(manifest.contains("name = \"CON\""));
        let (script, _) = scaffold("my-mod").unwrap();
        assert_eq!(script, "FolioMyMod");
        assert!(scaffold("---").is_err());
        assert!(scaffold("../other").is_err());
    }
}

#[derive(Serialize)]
struct CheckReport {
    schema: u32,
    target: String,
    diagnostics: Vec<CheckDiagnostic>,
}

#[derive(Serialize)]
struct CheckDiagnostic {
    code: String,
    severity: &'static str,
    message: String,
    primary: Option<CheckLocation>,
    related: Vec<CheckRelatedLocation>,
}

#[derive(Serialize)]
struct CheckLocation {
    path: String,
    start_byte: usize,
    end_byte: usize,
    line: usize,
    byte_column: usize,
    column: usize,
}

#[derive(Serialize)]
struct CheckRelatedLocation {
    message: String,
    location: Option<CheckLocation>,
}

impl CheckReport {
    fn new(target: &str, view: &ProjectAnalysisView, diagnostics: &[Diagnostic]) -> Self {
        let diagnostics = diagnostics
            .iter()
            .map(|diagnostic| CheckDiagnostic {
                code: diagnostic.code.clone(),
                severity: match diagnostic.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                    Severity::Info => "info",
                },
                message: diagnostic.message.clone(),
                primary: diagnostic
                    .primary
                    .and_then(|span| check_location(view, span)),
                related: diagnostic
                    .related
                    .iter()
                    .map(|related| CheckRelatedLocation {
                        message: related.message.clone(),
                        location: check_location(view, related.span),
                    })
                    .collect(),
            })
            .collect();
        Self {
            schema: 1,
            target: target.to_owned(),
            diagnostics,
        }
    }
}

fn check_location(view: &ProjectAnalysisView, span: SourceSpan) -> Option<CheckLocation> {
    let source = view.sources.get(&span.file)?;
    let text = view.analysis.text(span.file)?;
    let (line, byte_column) = view
        .analysis
        .line_index(span.file)?
        .line_col(span.range.start)?;
    let column = text
        .get(span.range.start - byte_column..span.range.start)?
        .chars()
        .count();
    Some(CheckLocation {
        path: source.display_path.clone(),
        start_byte: span.range.start,
        end_byte: span.range.end,
        line,
        byte_column,
        column,
    })
}

fn print_check(
    output: &mut impl Write,
    report: &CheckReport,
    command: &str,
) -> Result<(), CliError> {
    for diagnostic in &report.diagnostics {
        if let Some(location) = &diagnostic.primary {
            writeln!(
                output,
                "{}:{}:{}: {}[{}]: {}",
                location.path,
                location.line + 1,
                location.column + 1,
                diagnostic.severity,
                diagnostic.code,
                diagnostic.message
            )
            .map_err(CliError::Output)?;
        } else {
            writeln!(
                output,
                "{}[{}]: {}",
                diagnostic.severity, diagnostic.code, diagnostic.message
            )
            .map_err(CliError::Output)?;
        }
        for related in &diagnostic.related {
            if let Some(location) = &related.location {
                writeln!(
                    output,
                    "  {}:{}:{}: {}",
                    location.path,
                    location.line + 1,
                    location.column + 1,
                    related.message
                )
                .map_err(CliError::Output)?;
            }
        }
    }
    if report.diagnostics.is_empty() {
        writeln!(output, "{command} passed ({})", report.target).map_err(CliError::Output)?;
    }
    Ok(())
}

fn write_json<T: Serialize>(output: &mut impl Write, value: &T) -> Result<(), CliError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(CliError::Json)?;
    bytes.push(b'\n');
    output.write_all(&bytes).map_err(CliError::Output)?;
    Ok(())
}

fn print_metadata(output: &mut impl Write, metadata: &Metadata) -> Result<(), CliError> {
    writeln!(
        output,
        "{} {} -> {} ({})",
        metadata.root.name,
        metadata.root.version.as_deref().unwrap_or(""),
        metadata.target,
        metadata.profile
    )
    .map_err(CliError::Output)?;
    writeln!(
        output,
        "  fill missing arguments: {}; PEX debug info: {}",
        metadata.fill_missing_arguments, metadata.debug_info
    )
    .map_err(CliError::Output)?;
    writeln!(
        output,
        "{} packages, {} dependencies, {} scripts",
        metadata.packages.len(),
        metadata.dependencies.len(),
        metadata.scripts.len()
    )
    .map_err(CliError::Output)?;
    for package in &metadata.packages {
        writeln!(
            output,
            "  {} {} [{}]",
            package.id.name,
            package.id.version.as_deref().unwrap_or(""),
            if package.id == metadata.root {
                "workspace"
            } else {
                "api"
            }
        )
        .map_err(CliError::Output)?;
    }
    Ok(())
}

fn print_tree(output: &mut impl Write, metadata: &Metadata) -> Result<(), CliError> {
    writeln!(
        output,
        "{} {}",
        metadata.root.name,
        metadata.root.version.as_deref().unwrap_or("")
    )
    .map_err(CliError::Output)?;
    for edge in &metadata.dependencies {
        writeln!(
            output,
            "  {} -> {} {} ({:?})",
            edge.from.name, edge.to.name, edge.declared_path, edge.kind
        )
        .map_err(CliError::Output)?;
    }
    for script in &metadata.scripts {
        if script.providers.len() > 1 {
            let chain = script
                .providers
                .iter()
                .map(|provider| provider.package.name.as_str())
                .collect::<Vec<_>>()
                .join(" -> ");
            writeln!(output, "  script {}: {}", script.script, chain).map_err(CliError::Output)?;
        }
    }
    for requirement in &metadata.external_requirements {
        writeln!(
            output,
            "  external {}: {}",
            requirement.package.name, requirement.reason
        )
        .map_err(CliError::Output)?;
    }
    Ok(())
}

#[derive(Serialize)]
struct TreeView<'a> {
    schema: u32,
    root: &'a PackageId,
    dependencies: &'a [DependencyEdge],
    scripts: &'a [ScriptSelection],
    external_requirements: &'a [ExternalRequirement],
}

impl<'a> From<&'a Metadata> for TreeView<'a> {
    fn from(metadata: &'a Metadata) -> Self {
        Self {
            schema: 3,
            root: &metadata.root,
            dependencies: &metadata.dependencies,
            scripts: &metadata.scripts,
            external_requirements: &metadata.external_requirements,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error[{}]: {error}", error.code());
            error.exit_code()
        }
    }
}

#[cfg(test)]
mod declaration_command_tests {
    use super::*;

    #[test]
    fn generation_requires_exactly_one_publication_destination() {
        let arguments = [
            "folio",
            "declarations",
            "generate",
            "--source-root",
            "sources",
            "--source",
            "local-api",
        ];
        assert!(Cli::try_parse_from(arguments).is_err());
        let mut both = arguments.to_vec();
        both.extend(["--output", "api.fdecl", "--repo", "mod/api"]);
        assert!(Cli::try_parse_from(both).is_err());
        for destination in [["--output", "api.fdecl"], ["--repo", "mod/api"]] {
            let mut valid = arguments.to_vec();
            valid.extend(destination);
            let cli = Cli::try_parse_from(valid).unwrap();
            assert!(matches!(
                cli.command,
                Some(ProjectCommand::Declarations {
                    command: DeclarationsCommand::Generate {
                        format: DeclarationOutputFormat::Binary,
                        ..
                    }
                })
            ));
        }
    }
}
