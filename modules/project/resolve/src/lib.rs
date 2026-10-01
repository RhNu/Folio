//! Project loading adapters, strict manifest parsing and leaf dependency selection.

pub mod graph;
pub mod io;
pub mod manifest;

pub use graph::resolve;
pub use io::{LoadedProject, discover, load, load_and_resolve};
