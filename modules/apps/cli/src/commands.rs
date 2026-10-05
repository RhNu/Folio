//! Command coordination over shared project services.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::AtomicBool,
};

use clap::CommandFactory;
use folio_build::{
    ProjectAnalysis,
    execute::{self, BuildError},
    output,
};
use folio_diagnostics::Severity;
use folio_format::format_source;
use folio_lint::{LintConfig, lint_script};
use folio_papyrus::PapyrusDialect;
use folio_project_model::Metadata;
use folio_project_resolve::{LoadedProject, io::load_root_sources, load_and_resolve};
use tracing::info;
use tracing_subscriber::EnvFilter;

use super::{
    Cli, CliError, DeclarationOutputFormat, DeclarationsCommand, LogFormat, OutputFormat,
    ProjectCommand, SourceEncoding,
    reporting::{
        CheckReport, TreeView, print_build, print_check, print_metadata, print_success, print_tree,
        write_build_json, write_json,
    },
    scaffold::{create_project, scaffold},
};

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
            },
            Ok(_) => {},
            Err(folio_format::FormatError::Syntax(errors)) => {
                invalid = true;
                let offset = errors.first().map_or(0, |error| error.range.start);
                writeln!(
                    output,
                    "cannot format {} at byte {offset}: invalid syntax",
                    source.display_path
                )
                .map_err(CliError::Output)?;
            },
            Err(folio_format::FormatError::ChangedTokens) => {
                invalid = true;
                writeln!(output, "cannot safely format {}", source.display_path)
                    .map_err(CliError::Output)?;
            },
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
        for (path, ..) in &changes {
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
        },
        _ => {
            return Err(CliError::Declarations(
                "choose exactly one of --output or --repo".into(),
            ));
        },
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

pub(super) fn run(cli: Cli) -> Result<ExitCode, CliError> {
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
    let result = run_command(&mut output, cli.command, cli.manifest_path.as_deref())?;
    output.flush().map_err(CliError::Output)?;
    info!(phase = "cli.complete", "CLI invocation completed");
    Ok(result)
}

fn inspect_pex(output: &mut impl Write, path: &Path, format: OutputFormat) -> Result<(), CliError> {
    let bytes = fs::read(path)
        .map_err(|cause| CliError::Pex(format!("read {}: {cause}", path.display())))?;
    let file = folio_format_pex::PexFile::read_from_slice(&bytes)
        .map_err(|cause| CliError::Pex(format!("inspect {}: {cause}", path.display())))?;
    match format {
        OutputFormat::Text => {
            writeln!(output, "{}", file.metadata_dump()).map_err(CliError::Output)?;
        },
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
        },
    }
    info!(path = %path.display(), bytes = bytes.len(), objects = file.objects.len(), "inspected PEX");
    Ok(())
}

/// Dispatch commands that can be handled without loading a Papyrus project.
fn run_command(
    output: &mut impl Write,
    command: Option<ProjectCommand>,
    manifest: Option<&Path>,
) -> Result<ExitCode, CliError> {
    let mut result = ExitCode::SUCCESS;
    match command {
        None => {
            Cli::command().write_help(output).map_err(CliError::Help)?;
            writeln!(output).map_err(CliError::Output)?;
        },
        Some(ProjectCommand::Init { name }) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDirectory)?;
            create_project(&cwd, name.as_deref())?;
            writeln!(output, "created {}", cwd.display()).map_err(CliError::Output)?;
        },
        Some(ProjectCommand::New { name, path }) => {
            scaffold(&name)?;
            let cwd = std::env::current_dir().map_err(CliError::CurrentDirectory)?;
            let directory = path.unwrap_or_else(|| cwd.join(&name));
            fs::create_dir(&directory).map_err(|cause| {
                CliError::ProjectCreation(format!("create {}: {cause}", directory.display()))
            })?;
            create_project(&directory, Some(&name))?;
            writeln!(output, "created {}", directory.display()).map_err(CliError::Output)?;
        },
        Some(ProjectCommand::Inspect {
            pex: Some(path),
            format,
        }) => {
            inspect_pex(output, &path, format)?;
        },
        Some(ProjectCommand::Fmt { check: _, write }) => {
            let cwd = std::env::current_dir().map_err(CliError::CurrentDirectory)?;
            result = run_fmt(output, &cwd, manifest, write)?;
        },
        Some(ProjectCommand::Declarations { command }) => run_declarations(output, command)?,
        Some(command) => {
            result = run_project(output, &command, manifest)?;
        },
    }
    Ok(result)
}

/// Generate declarations or enumerate the selected local repository.
fn run_declarations(output: &mut impl Write, command: DeclarationsCommand) -> Result<(), CliError> {
    match command {
        DeclarationsCommand::Generate {
            source_root,
            source,
            encoding,
            output: path,
            repo,
            format,
        } => {
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
        },
        DeclarationsCommand::List => {
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
        },
    }
    Ok(())
}

/// Load project context once before dispatching shared-service operations.
fn run_project(
    output: &mut impl Write,
    command: &ProjectCommand,
    manifest: Option<&Path>,
) -> Result<ExitCode, CliError> {
    let mut result = ExitCode::SUCCESS;
    let cwd = std::env::current_dir().map_err(CliError::CurrentDirectory)?;
    let (loaded, metadata) = load_and_resolve(&cwd, manifest).map_err(CliError::Project)?;
    let root = Path::new(&loaded.root_key)
        .parent()
        .ok_or_else(|| CliError::ProjectCreation("manifest has no parent directory".into()))?;
    match command {
        ProjectCommand::Lsp => unreachable!("handled before locking stdout"),
        ProjectCommand::Init { .. }
        | ProjectCommand::New { .. }
        | ProjectCommand::Declarations { .. }
        | ProjectCommand::Fmt { .. } => {
            unreachable!("handled before project loading")
        },
        ProjectCommand::Check { format } => {
            result = check_project(output, &loaded, &metadata, *format)?;
        },
        ProjectCommand::Lint { format } => {
            result = lint_project(output, &loaded, &metadata, *format)?;
        },
        ProjectCommand::Build { format } => {
            result = build_project(output, &loaded, &metadata, *format, root)?;
        },
        ProjectCommand::Inspect { pex: None, format } => {
            result = inspect_project(output, root, *format)?;
        },
        ProjectCommand::Inspect { pex: Some(_), .. } => {
            unreachable!("handled without project loading")
        },
        ProjectCommand::Metadata {
            format: OutputFormat::Json,
        } => write_json(output, &metadata)?,
        ProjectCommand::Metadata {
            format: OutputFormat::Text,
        } => print_metadata(output, &metadata)?,
        ProjectCommand::Tree {
            format: OutputFormat::Json,
        } => write_json(output, &TreeView::from(&metadata))?,
        ProjectCommand::Tree {
            format: OutputFormat::Text,
        } => print_tree(output, &metadata)?,
    }
    Ok(result)
}

/// Execute the check operation using resolved project inputs.
fn check_project(
    output: &mut impl Write,
    loaded: &LoadedProject,
    metadata: &Metadata,
    format: OutputFormat,
) -> Result<ExitCode, CliError> {
    let mut result = ExitCode::SUCCESS;
    let mut service = ProjectAnalysis::new();
    let view = service
        .sync_project(loaded, metadata)
        .map_err(CliError::Analysis)?;
    let mut diagnostics = view.diagnostics();
    if !diagnostics
        .iter()
        .any(|item| item.severity == Severity::Error)
    {
        match execute::check_target(loaded, metadata, &view) {
            Ok(()) => {},
            Err(BuildError::Diagnostics(mut issues)) => {
                diagnostics.append(&mut issues);
            },
            Err(cause) => return Err(CliError::Build(cause)),
        }
    }
    let report = CheckReport::new(&metadata.target, &view, &diagnostics);
    match format {
        OutputFormat::Json => write_json(output, &report)?,
        OutputFormat::Text => print_check(output, &report, "check")?,
    }
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        result = ExitCode::from(2);
    }
    Ok(result)
}

/// Execute the lint operation using resolved project inputs.
fn lint_project(
    output: &mut impl Write,
    loaded: &LoadedProject,
    metadata: &Metadata,
    format: OutputFormat,
) -> Result<ExitCode, CliError> {
    let mut result = ExitCode::SUCCESS;
    let root_manifest = &loaded.root.manifest;
    let config = LintConfig::from_rules(&root_manifest.lint_rules)
        .map_err(|error| CliError::Lint(error.to_string()))?;
    let mut service = ProjectAnalysis::new();
    let view = service
        .sync_project(loaded, metadata)
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
        OutputFormat::Json => write_json(output, &report)?,
        OutputFormat::Text => print_check(output, &report, "lint")?,
    }
    if diagnostics
        .iter()
        .any(|item| item.severity == Severity::Error)
    {
        result = ExitCode::from(2);
    }
    info!(diagnostics = diagnostics.len(), "project lint completed");
    Ok(result)
}

/// Execute the build operation using resolved project inputs.
fn build_project(
    output: &mut impl Write,
    loaded: &LoadedProject,
    metadata: &Metadata,
    format: OutputFormat,
    root: &Path,
) -> Result<ExitCode, CliError> {
    let mut result = ExitCode::SUCCESS;
    let mut service = ProjectAnalysis::new();
    let view = service
        .sync_project(loaded, metadata)
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
        loaded,
        metadata,
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
                    write_build_json(output, &outcome, &report)?;
                },
                OutputFormat::Text => {
                    if !warnings.is_empty() {
                        print_check(output, &report, "build")?;
                    }
                    print_build(output, &outcome)?;
                },
            }
        },
        Err(BuildError::Diagnostics(issues)) => {
            let report = CheckReport::new(&metadata.target, &view, &issues);
            match format {
                OutputFormat::Json => write_json(output, &report)?,
                OutputFormat::Text => print_check(output, &report, "check")?,
            }
            result = ExitCode::from(2);
        },
        Err(cause) => return Err(CliError::Build(cause)),
    }
    Ok(result)
}

/// Execute the inspect operation using resolved project inputs.
fn inspect_project(
    output: &mut impl Write,
    root: &Path,
    format: OutputFormat,
) -> Result<ExitCode, CliError> {
    let result = ExitCode::SUCCESS;
    let build_root = output::existing_build_root(root)
        .map_err(CliError::ManagedOutput)?
        .ok_or_else(|| CliError::ProjectCreation("no successful build is recorded".into()))?;
    let index = output::read_index(&build_root).map_err(CliError::ManagedOutput)?;
    if index.invalid {
        return Err(CliError::ProjectCreation(
            "last successful build index is corrupt or its artifacts are missing".into(),
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
        OutputFormat::Json => write_json(output, &success)?,
        OutputFormat::Text => print_success(output, &success)?,
    }
    Ok(result)
}
