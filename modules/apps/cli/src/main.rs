//! Folio process entry point, command arguments, and error classification.
use clap::{Parser, Subcommand, ValueEnum};
use folio_build::{ProjectionError, execute::BuildError, output::OutputError};
use folio_project_resolve::{graph::ResolveErrorKind, io::LoadError, manifest::ManifestErrorKind};
use std::{path::PathBuf, process::ExitCode};

mod commands;
mod reporting;
mod scaffold;
use commands::run;

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
mod tests;
