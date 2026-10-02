# Architecture overview

Folio exposes project builds. The project layer loads manifests, source files, and dependencies; the frontend and analysis layer produce semantic facts with source locations; lowering validates target behavior and produces MIR. The PEX backend and independent codec turn that representation into artifacts. CLI and LSP adapt these shared services.

```text
folio.toml / local inputs / editor buffers
  -> project resolution and consistent inputs
  -> lossless Papyrus CST
  -> declarations, binding, types, and HIR
  -> target lowering and MIR validation
  -> PEX backend -> PEX codec
  -> build cache and project output
```

Formatting uses CST. Lint and editor queries share the analysis view. `check` stops after semantic and target feasibility checks; `build` continues through encoding and publication. Files and functions can be internal units of computation without becoming public build entry points.

## Module responsibilities

All paths below are relative to `modules`.

| Area | Responsibility |
| --- | --- |
| `foundation` | File identity, source locations, structured diagnostics, and target profiles |
| `project/model`, `project/resolve` | Manifest model, local input loading, dependency resolution, and script selection |
| `languages/papyrus` | Lexing, lossless CST, AST access, and declaration extraction |
| `compiler/hir`, `compiler/analysis` | Semantic facts, name and type analysis, and consistent query views |
| `compiler/lowering`, `compiler/mir` | Target lowering, control flow, and low-level validation |
| `backends/pex`, `formats/pex` | MIR-to-PEX mapping; independent PEX reading, writing, and validation |
| `formats/declarations`, `tooling/declarations` | Declaration formats and API extraction from PSC and PEX |
| `tooling/format`, `tooling/lint`, `tooling/ide` | Formatting, suggestions, and editor queries |
| `project/build` | Input projection, build planning, caching, execution, and output publication |
| `apps/cli`, `apps/lsp` | Commands, presentation, protocol, and process boundaries |
| `apps/xtask` | Repository maintenance outside the Papyrus build workflow |

Crates use `modules/<domain>/<short-name>` paths and `folio-<responsibility>` package names. Domain directories are not nested workspaces. Add a crate only for a distinct responsibility with a defined API and dependency boundary.

The syntax core depends on neither Salsa nor LSP nor manifests. Analysis consumes explicit inputs without reading the filesystem. Backends consume checked representations without reinterpreting CST, names, or types. The PEX codec does not depend on the compiler. Applications compose these lower-level services.

## Inputs and source locations

The project layer converts disk contents, dependency selection, declaration APIs, targets, and settings into explicit analysis inputs. Editor buffers override disk text. Each update produces a consistent view; stale results must not be published as current diagnostics.

File identity and half-open byte ranges connect CST, HIR, MIR, and diagnostics. Generated nodes retain their original source locations. LSP position encoding conversion occurs at the protocol boundary. Analysis can retain useful local facts after errors, while artifact generation rejects unresolved errors and invalid target operations.

CLI and LSP share resolution, semantic rules, diagnostics, and fix data. Semantic source content, declaration APIs, and relevant configuration contribute to query or build identity. Caches are disposable derived state.

## Target and runtime boundaries

API visibility and runtime capability are separate inputs. A visible declaration does not establish that its implementation is installed in the game. Each feature use must be implemented directly, lowered equivalently, or rejected with a reason; the compiler must not change semantics through silent approximation.

The [Papyrus reference](../papyrus.md) separates source-language requirements, [dialect differences](../papyrus/dialects.md), and [engine behavior](../papyrus/runtime.md) from Folio's implementation status. Documenting a game dialect does not register it as an implemented frontend or generation target. Current target parameters are defined by `TargetProfile` in `modules/foundation/profiles`; they cover a subset of the requirements, rather than establishing full language or runtime conformance.

## Engineering conventions

The root workspace manages the Rust toolchain, dependencies, and lockfile. Libraries emit structured `tracing` events, and process entry points initialize subscribers. Loading, provider selection, analysis, lowering, cache decisions, and publication need enough context to diagnose failures without logging source bodies or sensitive data. CLI logs use stderr; LSP stdout carries protocol messages only.

The domain contracts are documented in [Projects and dependencies](project-model.md), [Papyrus language reference](../papyrus.md), [Compiler pipeline](compiler.md), [Builds and artifacts](build-artifacts.md), and [Tools and editor services](tooling.md). Outstanding work belongs in the [roadmap](../planning/roadmap.md).
