//! Workspace maintenance commands for Folio contributors.

use std::{io, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use folio_xtask::workspace::check_lines;
use tracing::error;

#[derive(Parser)]
#[command(about = "Folio workspace maintenance tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Lint Rust source sizes; warn above 650 code lines and fail above 1200.
    CheckLines {
        /// Show every Rust file, including those below the warning threshold.
        #[arg(long)]
        all: bool,
        /// Cargo manifest used to locate the workspace (defaults to Folio).
        #[arg(long)]
        manifest_path: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let result = match Cli::parse().command {
        Command::CheckLines { all, manifest_path } => check_lines(manifest_path.as_deref(), all),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(error) => {
            error!(%error, "source line check failed");
            eprintln!("xtask check-lines: {error}");
            ExitCode::FAILURE
        }
    }
}
