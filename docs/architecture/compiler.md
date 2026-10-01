# Compiler pipeline

Folio receives consistent project inputs containing file identities, source text, dialect, visible declarations, target, and settings. The frontend does not read the filesystem or manifest. Semantic queries return structured facts and diagnostics. The backend consumes analyzed and validated low-level representations.

## Syntax and semantic analysis

The Papyrus frontend combines a handwritten scanner, recursive descent parsing, and Pratt expression parsing. Its Rowan CST preserves tokens, whitespace, comments, and error ranges. AST access allows required nodes to be absent while a document is being edited.

Declaration summaries are independent of function body contents. Resolved names, types, call targets, and conversions enter HIR with source locations. Symbol identity includes the owning script, state, or property accessor so equally named members do not accidentally share scope.

Local body analysis uses one semantic `Scope`. Its implementation is divided between parameter and statement checking in `semantic/scope.rs`, and child modules for expression analysis, name and member resolution, and call checking. These phases share local bindings, HIR facts, diagnostics, and cancellation state; module boundaries do not introduce separate analysis passes.

Salsa manages analysis inputs and queries. The host validates and applies file additions, replacements, and removals in batches, keeping text, revision, CST, and semantic facts consistent within a query view. Unchanged source text and dialect can reuse parsing and declaration summaries with source locations. Unchanged host inputs reuse the same immutable view and completed semantic facts.

External declarations, user flags, and argument filling policy invalidate the view only when their contents change. Existing views retain their original results. Inputs that affect semantics, including targets, visible APIs, and language policy, contribute to query identity. Salsa's internal keys do not cross CLI, LSP, or disk cache boundaries.

External APIs use the shared declaration model; root sources retain their function bodies separately. Each parameter records a required argument, a known literal default, or an unknown default. Analysis handles unknown callable kinds and defaults conservatively rather than inferring facts from provenance labels.

## Diagnostics and generation gates

Analysis preserves usable local HIR and diagnostics after errors so hover, definition queries, and lint can still work. Diagnostics contain stable codes, severity, file locations, and related locations where needed. Terminal and LSP adapters handle presentation.

Generation strictly checks unresolved symbols, error types, and target constraints. The ability to provide editor facts for part of a file does not make that file valid for a build.

## Target lowering and MIR

The current generation target is the Skyrim SE Papyrus ABI. After type and context checks, each feature use is implemented directly, lowered equivalently, or rejected. Visible APIs, runtime requirements, target instruction capabilities, and build settings remain separate concerns.

Lowering converts typed HIR into MIR with source locations. MIR explicitly represents storage, calls, conversions, and labeled control flow. Validation checks storage references, write destinations, labels, termination of reachable paths, native function bodies, and property accessors. Inherited fields enter validation through external storage slots supplied by analysis; unknown names cannot pass as valid fields.

`function.rs` owns the shared `FunctionLowerer` state, lifecycle, and temporary and label allocation. Its child modules handle statements and assignment places, expressions and conversions, and calls with capture optimization. Each phase writes to the same instruction sequence and retains the original source mappings.

The PEX backend does not resolve names or infer types again. Target rejections retain the original location and a specific reason. The compiler does not discard operations or invent values to make unsupported behavior compile.

## Evaluation order

Lowering preserves these language guarantees:

- `&&` and `||` use conditional jumps, so an unselected branch is not evaluated.
- Named arguments are evaluated in source order, then passed in their bound parameter positions.
- Compound assignment evaluates its receiver, array, and index once, preserving the required old value before evaluating the right side.
- A binary expression captures its left result before evaluating a right side that could overwrite the underlying storage.
- Implicit Boolean and string conversions become explicit MIR operations attributed to the original expression.

State methods and array operations are generated as target operations. Ordinary native APIs still require visible declarations. The user-facing language rules are described in [Skyrim Papyrus](../papyrus.md).

## PEX generation and optimization

`folio-backend-pex` maps validated MIR to the PEX model, including instructions, temporary storage, jumps, states, property layout, and debug line mappings. `folio-format-pex` independently reads, writes, validates, and inspects Skyrim PEX. It rejects unsupported game IDs, versions, and opcodes instead of assuming they use the Skyrim layout. Reused source is documented in the [codec provenance record](../../modules/formats/pex/PROVENANCE.md).

Current optimization is limited to generation cleanup justified by semantic checks. A call result capture is removed only when write and side-effect analysis proves it redundant. A trailing void `RETURN` is added only when MIR control flow can still reach the end of the function. These decisions do not depend on debug line mappings and do not change argument or compound assignment evaluation counts. `build.profile` does not currently select an optimization level.

[Builds and artifacts](build-artifacts.md) defines PEX byte caching and output publication.
