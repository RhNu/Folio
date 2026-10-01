//! LSP 3.17 stdio adapter over the project resolver and shared semantic analysis.
use folio_project_resolve::io::FolioHome;
use protocol::read_message;
use server::Server;
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
};

mod protocol;
mod server;

#[derive(Debug)]
pub enum LspError {
    Io(io::Error),
    Project(String),
    Protocol(String),
}

impl std::fmt::Display for LspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "LSP I/O: {error}"),
            Self::Project(error) => write!(f, "LSP project: {error}"),
            Self::Protocol(error) => write!(f, "LSP protocol: {error}"),
        }
    }
}
impl std::error::Error for LspError {}
impl From<io::Error> for LspError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Runs a single LSP session. The caller initializes a tracing subscriber that writes to stderr.
pub fn serve_stdio(manifest_path: Option<&Path>) -> Result<(), LspError> {
    let cwd = std::env::current_dir()?;
    let output = Arc::new(Mutex::new(io::stdout()));
    let input = io::stdin();
    let mut reader = io::BufReader::new(input.lock());
    let home = FolioHome::from_env().map_err(|error| LspError::Project(error.to_string()))?;
    let mut server = Server::new(cwd, manifest_path.map(Path::to_path_buf), output, home);
    while let Some(message) = read_message(&mut reader)? {
        if server.handle(message)? {
            break;
        }
    }
    Ok(())
}
