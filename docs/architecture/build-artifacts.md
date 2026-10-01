# Builds and artifacts

`folio check` performs semantic and target feasibility checks on a resolved project without writing PEX. `folio build` also generates the final PEX layout, encodes it, and publishes root project artifacts to `paths.output`. Some string table, instruction count, and debug line limits can only be determined during final encoding. Target planning includes MIR validation before encoding.

## Build flow

1. Load the manifest, local dependencies, declarations, and root sources; select script providers.
2. Project the loaded text into a consistent analysis view and check names, types, and target requirements.
3. Generate and validate MIR for selected root scripts, then encode PEX.
4. Recheck input snapshots and stage complete artifacts in an internal generation.
5. Check output ownership and digests, publish files, and update the latest successful build index.

The workspace, target, and profile identify a build instance. Files, declarations, and functions may be internal units of computation, but every build has project context. Dependencies contribute API declarations without producing deployable code. Build tasks run in a deterministic order.

## Semantic fingerprints and caches

Salsa reuses queries within a process; the disk cache reuses artifacts across processes. These caches are independent.

Disk fingerprints cover compiler identity, root source text, normalized declaration APIs, dependency entry order and provider selection, target rules, user flags, and settings that affect output. Parameter defaults and unknown API facts are part of the declaration API digest.

Provenance labels, historical source locations, carrier encoding, semantically irrelevant script or member ordering, and absolute host paths are excluded from the API digest. Invalidation remains conservative: one changed semantic input may rebuild the entire build instance. Corrupt cache entries or mismatched digests are discarded and rebuilt.

## Physical snapshots and publication checks

Physical input snapshots separately record raw file bytes, directory membership, link targets for logical paths, and the existence of repository candidates. Before publication, Folio rechecks those snapshots, reloads inputs using the operation's fixed Folio home, and compares the semantic fingerprint.

Rewritten, added, or deleted files, retargeted links, or a newly ambiguous repository entry prevent publication of stale results. A reusable API digest does not prove that the original files are unchanged.

## Reproducible output

PEX timestamps are zero, user and machine names are empty, and source paths are relative to the owning package. Generation IDs do not enter PEX bytes. `build.debug-info` controls function and line mappings without changing runtime instructions.

Build reports retain diagnostics and cache decisions, including argument filling warnings when the build uses cached output.

## Output ownership and recovery

The default output layout is flat: `Scripts/<script>.pex`. The project-local `.folio` directory stores caches, staged generations, the concurrent publication guard, and the latest successful build index.

Folio does not rewrite unchanged artifacts when both inputs and published files are unchanged. Missing outputs can be restored from a valid cache. It replaces a file only when the previous successful index records Folio ownership and the current digest still matches. A foreign file with the same name, or a modified old artifact, causes publication to fail.

A successful build removes previously owned artifacts that are no longer part of the result. Ordinary I/O failures trigger an attempt to restore old files. Replacing several PEX files is not atomic across a process or machine crash. `folio inspect` uses the successful index and content digests to detect inconsistent output.

Builds write within the project's managed locations and never deploy automatically to a game directory. `folio inspect --pex <path>` uses the independent codec to read an arbitrary PEX file without project context; it does not compile source files.
