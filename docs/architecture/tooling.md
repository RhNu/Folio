# Tools and editor services

CLI and LSP use the same project resolution and analysis view. Tooling does not duplicate Papyrus name resolution or type rules. Libraries return structured results; process entry points handle logging, output formats, and protocol messages.

## CLI and machine output

The CLI keeps command arguments and error classification in `main.rs`. Internal modules separate command coordination (`commands.rs`), text and JSON reports (`reporting.rs`), and project creation (`scaffold.rs`). They use the same shared project services and process logging boundary.

`folio` accepts global `--manifest-path`, `--log-filter`, and `--log-format` options. Commands include `init`, `new`, `metadata`, `tree`, `check`, `build`, `fmt`, `lint`, `inspect`, `lsp`, and `declarations`. Consult `folio --help` and each subcommand's help for its arguments.

`metadata` defaults to JSON. `tree`, `check`, `build`, `lint`, and `inspect` default to text and can select JSON. Project metadata, dependency trees, and build results have versioned schemas. Stdout carries the requested data and stderr carries logs. For `folio lsp`, stdout carries LSP messages only.

Exit status is 0 on success, 2 for project or source errors, and 1 for I/O or process errors.

## Formatting

`folio fmt` and `folio fmt --check` check root project Papyrus sources, list differing files, and return status 2 when formatting differs. Only `folio fmt --write` writes changes.

The formatter uses lossless CST and does not require complete dependency APIs or a successful semantic build. Its current style uses four-space indentation, basic token spacing, and a final newline. It preserves existing blank lines, each line's existing newline style, comments, string contents, and keyword and identifier spelling.

Each candidate is parsed again and checked against the original non-whitespace tokens. Invalid syntax, changed tokens, or an inability to establish safety prevents modification of that file. Writes recheck the input content. LSP document formatting uses the same formatter and rejects stale results if the buffer version or project generation changes.

## Lint

`folio lint` and LSP share rules over typed HIR. The current rule, `papyrus.prefer-truthy-none-check`, defaults to warning. In `If`, `ElseIf`, and `While` conditions, it suggests replacing `value != None` with `value` when `value` resolves to a script reference. The reversed comparison is also recognized. Arrays, unresolved types, and other types do not trigger the suggestion.

The rule reports diagnostics without modifying source. Configure its severity as `off`, `info`, `warning`, or `error` under `[lint.rules]`. Warnings do not fail the command; errors return status 2. Disabling lint does not disable compilation diagnostics in `check` or `build`.

## Language server capabilities

`folio lsp` serves project diagnostics, rich hover, completion and resolution, definition and declaration navigation, references, implementations, document highlights, signature help, document and workspace symbols, CodeLens and resolution, parameter-name inlay hints, verified rename, semantic tokens, and whole-document formatting over stdio.

The LSP adapter separates its public entry point in `lib.rs` and message framing, URI conversion, and position encoding in `protocol.rs`. A bounded stdin channel feeds the session event loop, which continues processing edits, cancellation, and shutdown while one project worker loads and analyzes inputs. Two query workers serve immutable ready snapshots through a queue limited to 64 waiting requests; excess requests receive cancellation errors. Completion, hover, signatures, and immediate document navigation take priority, with at most four consecutive interactive selections before a waiting background query runs. Identical waiting semantic-token, CodeLens, and inlay-hint requests coalesce; different hint ranges remain distinct. Cancellation and generation changes remove waiting work immediately, returning cancellation responses. Formatting remains synchronous. All handlers share project generation and guarded response publication. Completion and CodeLens resolution reject obsolete data.

Hover starts with the owning script, state where applicable, and selected provider's package, kind, and source. Source context and the declaration share one header group with normal paragraph spacing, avoiding a horizontal rule directly above the code block. A Papyrus code block preserves the complete declaration, default literals, and flags; long parameter lists wrap at token boundaries. Documentation, known limitations, and navigation actions occupy separate Markdown sections. Clients without Markdown receive plain text. User-authored documentation is escaped before being combined with trusted navigation actions.

Language hover uses the file's Papyrus dialect and lossless token/CST context, independently of successful name resolution. The current Skyrim catalog covers keywords, standard `Hidden`/`Conditional` flags in declaration positions, primitive and primitive-array types, literals, operators, and delimiters. Ordinary identifiers retain semantic declaration hover. `Self` and `Parent` combine their explanations with analyzed types and navigation; array `Length` keeps the checker's intrinsic signature. Token ranges exclude neighboring whitespace and comments. Numeric previews include a directly enclosing unary minus, distinguish subtraction from negation, and decode signed decimal integers and full-width 32-bit hexadecimal bit patterns. Hex previews show the signed value, so `0xFFFFFFFF` displays -1; directly minus-prefixed literals retain signed-magnitude limits. Integer and string previews share Papyrus literal decoding with validation and lowering; HIR re-exports that API. String previews display escaped control characters and describe source values rather than runtime string-cache casing. Invalid or out-of-range values are identified without inventing a value. New hover specifies an integer literal size. Hover does not evaluate arbitrary expressions or establish target acceptance.

Completion respects lexical block visibility. Local rename distinguishes same-named sibling declarations, remaps declaration identities after edits, and verifies every original binding through reanalysis to prevent capture. Intrinsic completion, signature help, and named-call analysis use `akElement`/`aiStartIndex` for Find/RFind and `asNewState` for GotoState.

Selected dependency PSC snapshots and generated read-only API documents use the same pure language-help query with the project's implemented dialect. The server performs no online lookup. Language descriptions remain prose, examples use Papyrus code fences, and catalog reference URLs render as Markdown links or plaintext URLs. `hover.documentation` controls descriptions, examples, and reference links; `hover.details` controls values and other facts. Other games' keywords or constraints are not added to the Skyrim catalog.

This catalog is an editor aid for the implemented Skyrim dialect, rather than the complete Papyrus specification. The [language](../papyrus.md), [dialect](../papyrus/dialects.md), and [engine](../papyrus/runtime.md) references provide broader rules and source conflicts. Their Fallout and Starfield sections do not extend the current hover catalog. Known catalog/compiler discrepancies and unresolved extension policies are recorded in the [roadmap](../planning/roadmap.md#papyrus-conformance); the catalog is not a guarantee that every described expression can be emitted correctly.

The catalog contains original summaries and small original examples, without vendored wiki text or assets. Its Skyrim references were checked through the CK UESP MediaWiki API; these revision IDs identify the evidence for the descriptions and are separate from game/runtime compatibility verification:

| Creation Kit reference | Revision |
| --- | --- |
| [Keywords](https://ck.uesp.net/wiki/Keyword_Reference) | 12664 |
| [Literals](https://ck.uesp.net/wiki/Literals_Reference) | 25539 |
| [Default values](https://ck.uesp.net/wiki/Default_Value_Reference) | 9273 |
| [Operators](https://ck.uesp.net/wiki/Operator_Reference) | 25698 |
| [Flags](https://ck.uesp.net/wiki/Flag_Reference) | 10064 |
| [Properties](https://ck.uesp.net/wiki/Property_Reference) | 25738 |
| [States](https://ck.uesp.net/wiki/State_Reference) | 26035 |
| [Arrays](https://ck.uesp.net/wiki/Array_Reference) | 24877 |
| [Functions](https://ck.uesp.net/wiki/Function_Reference) | 25052 |
| [Events](https://ck.uesp.net/wiki/Events_Reference) | 25007 |
| [Script structure](https://ck.uesp.net/wiki/Script_File_Structure) | 25811 |
| [Statements](https://ck.uesp.net/wiki/Statement_Reference) | 26036 |
| [Casts](https://ck.uesp.net/wiki/Cast_Reference) | 24921 |

Completion shares the checker's scope, inheritance, instance/global, and import lookup rules. Initial candidates carry inexpensive type details and replacement edits; resolution renders the selected candidate's full signature and documentation without enumerating candidates again. A bounded session cache retains eight candidate lists and rejects expired items. External declaration lookups use the analysis index, and HIR readers share immutable results. Built-in state and array operations use the checker's authoritative signature facts. Signature help uses resolved calls and declaration parameter order, including defaults and available documentation. Parameter hints use the bound argument order and omit explicit named arguments and arguments already spelling the parameter name. Document symbols include scripts, states, and members; workspace symbols search root project declarations.

Completion selects matching prefixes from the checker's ordered name maps before allocating candidates. ASCII semantic candidates and keywords are merged in order without sorting the complete result again; empty-prefix completion still returns the complete candidate set. Expensive completion work cooperatively observes cancellation, including response construction.

Each ready project owns an immutable `IdeSnapshot` with shared lazy declaration, per-file occurrence, reference, and ancestry indices. Hover, navigation, CodeLens, and semantic tokens reuse those facts; implementation queries restrict member inspection to eligible descendants and cache results by semantic symbol identity. Local declaration identity remains part of reference keys. Request cancellation belongs to an individual snapshot handle; it cannot poison shared indices or publish an unfinished index. New project inputs receive new indices, and final response publication still rejects obsolete requests.

References use bound symbols, type references, imports, and named-argument labels. Counts cover analyzable root project code, including unsaved root buffers; runtime calls and dependency bodies are outside that count. Implementation navigation includes known derived scripts and compatible callable overrides from selected sources and declaration dependencies. CodeLens above scripts and members exposes source context, parent navigation, references, derived scripts, and overrides.

Rename supports local variables, parameters, and verifiable root members. It requires complete analysis without errors, rejects scripts, state members, accessors, intrinsics, native functions, events, and inherited/overridden APIs, checks identifier and visibility conflicts, and reanalyzes all proposed edits before returning a versioned workspace edit. Every original binding must preserve its selected declaration. It cannot prove that runtime strings or consumers outside the project refer to a renamed member.

Open unsaved `.psc` buffers override disk text; closing a document restores disk state. Edits to selected direct PSC dependencies regenerate their declaration API from the open buffers, so hover, signatures, and source locations agree with the edited declaration without analyzing dependency bodies. Only new unsaved scripts within the root project's source directory enter the project graph. Stale results are not published as current diagnostics. Positions default to UTF-16 and can negotiate UTF-8 when the client supports it.

Root sources and direct PSC dependencies support navigation to real source locations. Dependency navigation parses the PSC snapshot loaded for the operation without adding dependency bodies to semantic analysis. Declaration, repository, and PEX inputs show verifiable aliases or carrier provenance. The VS Code client negotiates read-only `folio-declaration:` documents, including precise generated member locations and explicit unknown PEX facts; historical generation paths never become file navigation targets.

The Folio client negotiates the custom `folio/declarationContent` request and `folio/projectChanged` notification through initialization options. The request returns the selected API's text and language ID or null when it is no longer available. Clients without that support receive source navigation only. Folio command links are negotiated separately; ordinary LSP clients receive file links where possible. Editor preferences arrive through initialization options and `workspace/didChangeConfiguration` without changing semantic settings or restarting the server. CodeLens and inlay refresh requests respect client capabilities.

Clients can also negotiate `initializationOptions.folio.status`. The `folio/status` notification reports `loading`, `ready`, or `error`, a generation, phase, readable message, and optional completed/total counts. Counts describe the reported phase, not an estimated overall percentage. While loading, automatic semantic-token, CodeLens, and inlay queries return empty results, while item resolution returns a content-modified error. A separate bounded queue retains up to 64 interactive and outline requests until the matching generation is ready; cancellation and further edits discard obsolete requests. Ready publication refreshes semantic tokens, CodeLens, and inlay hints when supported. The VS Code client suppresses these automatic queries during loading and shows progress in the status bar; lightweight TextMate coloring remains available.

## Session inputs and invalidation

The project worker retains disk inputs, dependency projections, and the analysis host for the session. Editor requests share the prepared loaded project instead of copying its declarations. Opening or submitting identical ready text reuses semantic results and updates the diagnostic version. Actual edits immediately invalidate old requests and coalesce for 40 ms before projection, preserving all open overlays. At most one load runs and one latest update waits; cancellation never loses a pending disk refresh. Generation changes and cancellation are serialized with final query publication, and obsolete project results are discarded.

Closing a document refreshes disk inputs to restore discarded buffers and account for new or deleted files. Save and relevant watched-file events coalesce into disk refreshes. Broad client events for unrelated files are ignored. Refreshes reread inputs and verify exact bytes, directory membership, and path resolution before reusing decoded dependency declarations; they do not rely solely on timestamps. Manifest order and root provider precedence remain authoritative. A load failure clears current results but retains input monitoring so the same session can recover after repair.

Analysis and dependency caches stay in memory; LSP semantic data is not persisted in `.folio`. Source edits reuse unchanged dependency signatures, declaration indexes, and source-independent checks. Source names and ancestry participate in reuse of external world validation. Root body analysis still runs project-wide when semantic inputs change; this is not a per-function incremental semantic engine.

## File monitoring

Repository watches cover both candidate carriers for references without a suffix. Missing paths are watched through their nearest existing parent directory so later creation can be detected.

Clients supporting dynamic relative-path watches receive registrations for resolved manifests, source directories, and dependency carriers, including dependencies outside the workspace folder. Other clients must forward those changes themselves. Registrations are updated when dependency configuration changes.

## Protocol positions and diagnostics

Document outlines, semantic tokens, and diagnostics share a text-derived line index for protocol positions, preserving UTF-8, UTF-16, and CRLF boundaries.

Debug logs split project preparation into input loading, projection, semantic analysis, and diagnostic/lint collection. Editor query `elapsed_us` starts when the protocol handler receives the request and includes deferred loading and queue waits; separate fields report those waits, context preparation, query work, publication-lock waiting, and response handling. Completion also reports candidate creation and response construction separately. Protocol trace events report response bytes, serialization, output-lock waiting, and writing. Project spans and timings do not contain source bodies or complete metadata objects.

The `modules/apps/lsp/tests/performance.py` tool can use a release binary to report loading, open, close, and edit timings. `--project <directory> --focus <relative.psc>` exercises an existing project through unsaved buffers, including empty/prefixed completion, selected-item resolution, hover, and save refresh. Reports and logs stay under Folio's `.folio/lsp-perf`; project files are not edited. The probe waits for negotiated project readiness rather than treating a transport response as a loading barrier. It does not negotiate the VS Code command links or reproduce concurrent automatic editor requests, so its hover timings do not cover that complete client path. `--compare` checks stable response digests against another report. This opt-in external probe is separate from pure logic unit tests and does not establish real editor or game compatibility.

## VS Code client

The client in `editors/vscode` registers `.psc`, provides a default Papyrus file icon and TextMate highlighting, starts `folio lsp`, and forwards file events. The same Papyrus grammar colors hover code blocks using the active theme. Semantic highlighting uses standard VS Code token types and modifiers for resolved functions, events, types, properties, parameters, and variables, with contributed TextMate scope mappings.

Navigation commands validate document schemes and source positions before opening or peeking locations. Trusted Markdown enables only the Folio navigation command allowlist. Read-only declaration buffers refresh on project changes and reconnects. The status item distinguishes transport connection from project readiness and shows loading phases/counts; clicking it opens output. Source context, CodeLens categories, hover documentation/details, and parameter hints have live settings.

The icon is packaged in the VSIX. VS Code uses it when the active file icon theme allows language icons and has no specific icon for `.psc` or Papyrus.

Executable lookup checks the extension setting, then `PATH`, then the repository's `target/debug` build in extension development mode only. One client session serves one Folio project folder. Packaging, installation, publishing, debugging, and settings are documented in the [extension README](../../editors/vscode/README.md). The VSIX contains no Folio executable.

## Repository maintenance

[Rusteward](https://github.com/RhNu/Rusteward) provides Rust formatting, Clippy, source-layout policies, and effective code-line checks across the Cargo workspace. These contributor commands are separate from Folio's Papyrus `fmt` and `lint` operations. The repository's `rust-toolchain.toml` selects Rust and the rustfmt/Clippy components.

Install Rusteward once with `cargo install --git https://github.com/RhNu/Rusteward.git --locked rusteward`. Run these commands from the workspace or any member directory:

```powershell
cargo dev format --locked
cargo dev format --check --diff --locked
cargo dev lint --locked
cargo dev check --locked
cargo dev config show --locked
```

`format` applies rustfmt followed by declaration spacing. `format --check --diff` shows the final differences without modifying source. `lint` runs source policies and the managed Clippy profile; `check` verifies formatting and collects lint findings. Add `--manifest-path <Cargo.toml>` to select another workspace, or `--json` for structured diagnostics. `--locked` preserves Cargo.lock during workspace discovery and Clippy.

`rusteward.toml` specifies only the inherited line policy: warn above 650 effective code lines and error above 1,200, with nonfatal line warnings. Formatting and lint use Rusteward defaults, including the managed `all`/`pedantic` Clippy profile, declaration spacing, `mod-rs`, and external unit-test modules. Keep this configuration free of project-specific formatting and lint overrides. User-level Rusteward configuration participates in normal layering; use `config show` to confirm that a local profile has not overridden these defaults.

Effective code lines contain Rust token content: blank lines, comment-only lines, BOM, and shebang are excluded; nonblank contents of multiline literals count. The same thresholds apply to production code, tests, and shared test helpers. Split files that exceed the hard limit by responsibility. Rusteward scans authored workspace Rust sources, including tests and inactive feature code, and skips symlinks, Cargo output, generated files, and its default excluded directories. See the [Rusteward reference](https://github.com/RhNu/Rusteward/blob/main/docs/features.md) for scan boundaries and the authoritative default profiles.

Rusteward returns status 0 for passing checks, 1 for formatting or lint failures, and 2 for operational errors. Clippy warnings fail the check; custom rule warnings remain nonfatal under the default policy.

The Rust quality workflow installs the repository-selected Rust toolchain and Rusteward through `RhNu/Rusteward@main`, then runs `cargo dev check --locked` on Ubuntu and Windows. Hosted installation and cache verification remain tracked in the roadmap until verified on GitHub runners.

## Rust test organization

Unit tests of individual functions, module behavior, or private implementation details use a separate child module file named `tests.rs`. The parent declares `#[cfg(test)] mod tests;`; alternative entry names and inline unit test modules are not used. Crate-root units use `src/tests.rs`. Units for another module use `<module>/tests.rs`, such as `src/manifest/tests.rs` for `src/manifest.rs`.

When a unit suite grows too large, keep `tests.rs` as its entry and declare responsibility-based child modules in the corresponding `tests/` directory. For example, `src/manifest/tests.rs` can declare `mod parsing;` to load `src/manifest/tests/parsing.rs`.

Comprehensive tests that cover a crate's complete public workflow belong in its `tests/` directory beside `src/`. Each entry is a Cargo test target named for its responsibility and imports the crate's public API. Public parser and codec contracts, semantic analysis, HIR-to-MIR lowering, PEX emission, and project resolution and planning are tested this way with in-memory inputs. Private function and boundary units remain inside their owning source modules. Mixed suites are split according to what each case exercises, without dropping assertions or exposing private production APIs solely for testing.

Shared fixtures and independent test evaluators live under the test trees, normally `tests/common/support.rs`. An internal unit suite can reference a shared fixture through a test-only helper module. Helpers are not standalone Cargo test entries or production APIs. Unit and comprehensive suites can both test pure logic; their placement alone does not imply real filesystem, process, protocol, editor, or game coverage. External verification still requires the separately authorized workflow and any unresolved results remain in the roadmap.
