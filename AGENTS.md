# Folio project guide

## Working context

- Read `README.md` and the relevant domain guide before changing behavior. Keep public configuration and examples aligned with the implementation.
- Write workspace documentation in English. Maintain `README.zh.md` as the corresponding Chinese copy of the root `README.md`; other documents have no translated copies.
- Keep outstanding work and unresolved verification in `docs/planning/roadmap.md`. Domain guides describe the current implementation.

## Product and module boundaries

- Every public build operation requires a project context. Internal file or function queries do not imply a single-file build interface.
- Place Rust crates under `modules/<domain>/<short-name>`. Use `folio-<responsibility>` for package names and normally `folio_<responsibility>` for library names. Domain directories are not nested workspaces.
- Keep the syntax core independent of Salsa, LSP, and manifests. Semantic queries must not read files, access the network, or render diagnostic text.
- The frontend does not generate PEX. The backend consumes analyzed representations without reading CST, resolving names again, or inferring types. The PEX codec remains independent of the compiler.
- Treat CLI and LSP as adapters over shared project loading, analysis, diagnostics, and fixes.
- Before adding a crate, establish its API boundary and dependency direction using `docs/architecture/overview.md`. Do not add empty crates for future features.

## Compiler and project invariants

- Preserve CST comments, whitespace, and error nodes. Analysis may retain partial results after errors; artifact generation requires strict validation.
- Source locations include file identity. Generated nodes retain origin mappings. Convert LSP position encodings only at the protocol boundary.
- Select duplicate scripts as whole units: later manifest dependencies take precedence, and root project sources take precedence over dependencies. Directory enumeration must not affect selection; retain enough provenance to explain it.
- Model SDK/API visibility separately from runtime capabilities. Declaration dependencies do not produce deployable code.
- Give each target feature an explicit outcome: direct implementation, equivalent lowering, or rejection. Never silently approximate program semantics. Maintain one authoritative capability model and record each feature's target requirements and verification basis.
- Build cache keys cover actual semantic inputs. Manage Salsa's in-memory query cache separately from the disk artifact cache.
- Builds write only to Folio-managed output locations and never deploy automatically to game directories.

## Repository conventions

- Put every Rust unit test module in a separate child module file named `tests.rs`; do not define unit test modules inline or use alternative entry names. The parent declares `#[cfg(test)] mod tests;`. Use `src/tests.rs` for crate-root units and `<module>/tests.rs` for units of other modules. Split large suites into responsibility-based child files under the owning `tests/` module directory, with `tests.rs` as their entry.
- Put comprehensive tests of a crate's public workflows in that crate's `tests/` directory beside `src/`, grouped by responsibility. Split mixed suites according to coverage: private implementation units remain in `tests.rs`, while complete public API flows use external Cargo test targets. Keep helpers in the test trees and do not widen production visibility solely for tests. Test placement does not establish or authorize external I/O, process, editor, or game verification.
- Libraries emit structured `tracing` events; process entry points initialize subscribers. Build logs identify the package, target, revision, and stage. Reserve LSP stdout for protocol messages.
- Pure logic tests do not establish real CLI, file watcher, editor, or game compatibility. Keep unresolved external verification in the roadmap.
- Reuse knowledge and minimal behavioral examples from older projects without importing their coupled CLI, workspace, or compiler structure wholesale.
- Before adding third-party source, declarations, or assets, verify use and distribution terms and retain auditable provenance, licensing, and attribution in the owning directory.
- The root workspace owns the Rust toolchain and dependencies. Repository configuration is authoritative for their versions.
