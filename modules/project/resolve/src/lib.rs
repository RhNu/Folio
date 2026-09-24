//! Project loading adapters, strict manifest parsing and pure graph resolution.

pub mod graph;
pub mod io;
pub mod manifest;
mod pex_declarations;

pub use graph::resolve;
pub use io::{LoadedProject, discover, load, load_and_resolve};
