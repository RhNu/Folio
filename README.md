# Folio

[English](README.md) | [Simplified Chinese](README.zh.md)

Folio is a toolchain for Papyrus projects. It reads `folio.toml`, loads local sources and declaration dependencies, checks Skyrim Papyrus, and generates PEX files for the root project. Checking, building, formatting, linting, and editor services share the same project context.

## Quick start

With `folio` on your `PATH`, run:

```powershell
folio new MyMod
cd MyMod
folio check
folio build
```

`folio new` creates a manifest and a starter script under `Source/Scripts`. To initialize an existing directory, run `folio init --name MyMod` there. Project commands find the nearest `folio.toml` by searching upward from the current directory; use `--manifest-path <path>` to select one explicitly.

By default, Folio reads `.psc` files from `Source/Scripts`, writes the root project's `.pex` files to `Scripts`, and keeps disposable caches and build records in `.folio`. Dependencies supply declarations for analysis. Folio does not deploy files to the game directory.

## Commands

| Command                      | Purpose                                                    |
| ---------------------------- | ---------------------------------------------------------- |
| `folio check`                | Check semantics and target feasibility without writing PEX |
| `folio build`                | Generate and publish PEX for the root project              |
| `folio fmt`                  | Check formatting; use `folio fmt --write` to apply changes |
| `folio lint`                 | Report configurable code suggestions                       |
| `folio tree`                 | Show dependencies and script provider selection            |
| `folio metadata`             | Describe the resolved project; JSON is the default         |
| `folio inspect`              | Verify the latest successful build and its outputs         |
| `folio inspect --pex <path>` | Read a PEX file independently of a project                 |
| `folio declarations list`    | List declarations in the local repository                  |
| `folio lsp`                  | Start the language server                                  |

`check` does not perform final PEX encoding, so `build` can report additional layout limits. `inspect --pex` reads an artifact; it does not build a source file. See each command's help for options and output formats.

## Project configuration

The manifest uses the current release's fields and defaults without a schema version. This example shows the default source and output directories; the `[paths]` section is optional.

```toml
[package]
name = "my-mod"
version = "0.1.0"

[paths]
source = "Source/Scripts"
output = "Scripts"

[languages.papyrus]
dialect = "skyrim"
extensions = ["psc"]

[build]
target = "skyrim-se"
profile = "dev"
emit = ["pex"]
```

Dependencies can be PSC directories (`psc`), experimental PEX directories (`pex`), declaration files (`decl`), or entries in the user-level declaration repository (`repo`). All four supply API declarations. Later dependencies override earlier scripts as whole units; root project sources take precedence over every dependency. The project remains responsible for the required runtime implementations.

A mod's PSC directory can be used directly without adding a manifest to it:

```toml
[[dependencies]]
name = "other-mod"
kind = "psc"
path = "../OtherMod/Source/Scripts"
```

`decl` accepts JSON and binary `.fdecl` files. `repo` resolves declarations under the user's `.folio/repo`; an absolute `FOLIO_HOME` changes that location. Generate CK and SKSE declarations from your own local sources and select them explicitly. PEX directory dependencies require `[experimental] pex-dependencies = true` and expose only API facts recoverable from the binary. See [Projects and dependencies](docs/architecture/project-model.md) for configuration, generation commands, and limitations.

## Language and editor support

[Papyrus language reference](docs/papyrus.md) documents the Skyrim baseline, [game dialect differences](docs/papyrus/dialects.md), and [engine semantics](docs/papyrus/runtime.md), with sources and unresolved specification questions. Folio implements the `skyrim` dialect and `skyrim-se` generation target. Source and dependency declarations allow defaults before required parameters and share validation for literal defaults, property forms, and declaration flags; analysis checks lexical scopes, conversions, accessor permissions, and inherited contracts. [Compiler pipeline](docs/architecture/compiler.md) describes the implementation. Folio extensions and unresolved CK/engine verification remain explicit in the reference and [conformance roadmap](docs/planning/roadmap.md#papyrus-conformance). [Tools and editor services](docs/architecture/tooling.md) covers formatting, lint, diagnostics, navigation, and the language server's project model.

The VS Code client provides `.psc` file icons, syntax and semantic highlighting, declaration hover with documentation and source links, offline Skyrim language hover for keywords, built-in types, literals, and operators, completion, signature help, references, inheritance navigation, CodeLens, parameter hints, verified project renames, symbol search, and formatting. Language hover includes literal values, examples, and Creation Kit reference links in source and read-only API documents. Dependencies without PSC source open as read-only API declarations. It uses a configured Folio executable or directory, then searches `PATH`. To package the client, run `npm ci` and `npm run package` in `editors/vscode`. The VSIX does not include the Folio executable. See the [extension README](editors/vscode/README.md) for installation, settings, debugging, and publishing.

Project loading and analysis run in the background. The VS Code status bar reports loading phases and progress; semantic highlighting, CodeLens, and parameter hints refresh after the project is ready. Session caches reuse unchanged dependency declarations and share analysis results across editor queries.

## Development

Rust crates live under `modules/<domain>/<crate>`; the VS Code client lives in `editors/vscode`. Start with the [architecture overview](docs/architecture/overview.md), then read the relevant domain guide:

- [Projects and dependencies](docs/architecture/project-model.md): manifests, declarations, and provider selection.
- [Compiler pipeline](docs/architecture/compiler.md): analysis, lowering, and PEX generation.
- [Builds and artifacts](docs/architecture/build-artifacts.md): fingerprints, caches, and output publication.
- [Tools and editor services](docs/architecture/tooling.md): CLI, LSP, formatting, lint, and repository maintenance.

[AGENTS.md](AGENTS.md) contains project constraints. Keep both README language versions and the affected domain guide aligned with public configuration or behavior changes. Outstanding work and verification gaps belong in the [roadmap](docs/planning/roadmap.md).

Run `cargo xtask check-lines` to count Rust code lines across the workspace. It excludes comments, blank lines, and generated directories, warns above 650 lines per file, and fails above 1,200. Use `--all` to list every file or `--manifest-path <Cargo.toml>` to select a workspace. See [Repository maintenance](docs/architecture/tooling.md#repository-maintenance) for counting and exit status rules.

## License and provenance

Folio uses [GNU GPL version 3](LICENSE). [Declaration generation](docs/architecture/project-model.md#generating-declarations) explains local API sources and attribution. Third-party SDKs, game scripts, and other assets remain subject to their own use and distribution terms.
