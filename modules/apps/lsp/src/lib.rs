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
    let (messages, incoming) = std::sync::mpsc::sync_channel(128);
    std::thread::Builder::new()
        .name("folio-stdin".into())
        .spawn(move || {
            let input = io::stdin();
            let mut reader = io::BufReader::new(input.lock());
            loop {
                let message = read_message(&mut reader);
                let done = !matches!(message, Ok(Some(_)));
                if messages.send(message).is_err() || done {
                    break;
                }
            }
        })?;
    let home = FolioHome::from_env().map_err(|error| LspError::Project(error.to_string()))?;
    let mut server = Server::new(cwd, manifest_path.map(Path::to_path_buf), output, home);
    loop {
        server.poll_background()?;
        match incoming.recv_timeout(std::time::Duration::from_millis(10)) {
            Ok(Ok(Some(message))) => {
                if server.handle(message)? {
                    break;
                }
            }
            Ok(Ok(None)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Err(error)) => return Err(error),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    Ok(())
}
