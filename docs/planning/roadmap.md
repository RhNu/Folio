# Roadmap

This is the workspace's record of outstanding work and unresolved verification. The [README](../../README.md) and domain guides describe the current implementation. Remove completed items from this page and retain their lasting behavior in the relevant guide.

## Distribution and compatibility

- Establish executable and VS Code client distribution channels, installation guidance, and version policy when release channels are selected.
- Establish compatibility evidence with identified external environments and inputs before declaring supported game and tool combinations.

## Tooling and editor services

- Verify loading progress, deferred semantic highlighting, refresh after readiness, reconnect, shutdown, and sustained editing with Cadence in real VS Code. Stdio probes do not establish UI behavior or long-session CPU/memory stability. Investigate the reported interruption separately; retained logs do not establish a language-server crash.
- Verify missing repository carrier recovery, ambiguous dual carriers, path alias retargeting, and PSC navigation across drives in a real editor.
- Verify LSP session cache reuse, buffer close recovery, dynamic watch registration, stale and cancelled navigation responses, and stdio message framing in a real editor. Compilation and in-memory Rust tests do not verify these session behaviors.
- Add `folio explain` for stable diagnostic codes, separating general explanations from rejection reasons in a specific project.
- Verify themed declaration hover and spacing above code blocks, documentation sections and links, CodeLens/parameter hint refresh, completion resolution, versioned rename edits, unsaved PSC dependency API/navigation updates, and read-only declaration navigation/reconnect recovery in VS Code. Pure logic tests and compilation do not establish these editor behaviors.
- Verify language hover rendering for Skyrim keywords, flags, types, literals, and operators in VS Code, including selected PSC dependencies, read-only API views, Creation Kit link opening, theme-colored examples, and live documentation/details settings. Pure logic coverage does not establish the editor presentation or browser-link behavior.
- Add range formatting, further lint rules, and fixes with revision checks as concrete needs arise.

## Additional targets

- Select a specific game target and SDK, then define its language differences, target constraints, PEX encoding, and verifiable lowering behavior.
- Design manifests and diagnostic presentation for multiple targets when a real project requires them.

## Papyrus conformance

The current scope is the Skyrim language and `skyrim-se` generation target. CK/SKSE API catalogs and additional game implementations are separate work. Pure syntax, semantic, MIR/PEX, and editor-model unit tests establish compiler behavior for their inputs; they do not establish original CK or engine compatibility.

### Coverage and unresolved decisions

- Verify nontrailing parameter defaults against original CK 1.6.1170.0 PSC and its compiler, including explicit positional calls, named omission of an earlier default, and rejection of omitted required arguments. Local refreshed declaration data contains this pattern in `CarriageSystemScript.Travel`, `FerrySystemScript.Travel`, and `WIDeadBody01.ReorderAliasesBasedOnDistance`. Folio accepts it while preserving required slots; derived declarations and pure logic tests do not establish original compiler or game compatibility.
- Obtain an identified original CK/compiler baseline for the complete mixed Bool/numeric/string operator and comparison matrix, standalone None casts, redundant array casts, all namespace collision pairs, and mixed positional/named argument ordering. Do not derive a complete matrix from isolated wiki examples.
- Verify or narrow explicit Folio extensions: event documentation, unary plus, repeated unary/cast expressions, the `\r` escape, and reopening same-named source states. Declaration extraction currently merges reopened states; authoritative acceptance/rejection evidence remains missing. Source identifiers currently use ASCII letters/digits/underscore.
- Confirm constant-context conversion rules beyond matching types, Int-to-Float widening, and None references. Runtime String conversions do not imply a compile-time formatter for declaration constants.
- Resolve conflicting evidence for Conditional AutoReadOnly and typed fallthrough. Current Folio policy rejects both rather than emitting uncertain behavior.
- Verify explicitly minus-prefixed high-bit hexadecimal literals and original compiler Int-to-Float handling. Unsigned full-width hex is accepted as signed 32-bit bits, supported by local SKSE PSC/PEX evidence. Direct signed literals retain signed-magnitude limits; `-0xFFFFFFFF` remains rejected. Obtain a compiled caller confirming the omitted `GameData.GetAllWeapons.weaponTypes` default: callee PEX does not retain parameter defaults.
- Verify inherited private-field visibility and namespace interactions independently of owner-qualified storage identity. Local state capacity is 128 including the empty state; inherited counting needs versioned evidence.
- Select a `.flg` import workflow only when required. Manifest definitions now carry explicit bits/scopes, but this does not establish equivalence to original compiler flag-file parsing.
- Improve the location of remaining downstream-only rejections, including unsupported Float remainder and some global-instance cases. No approximation is emitted for these operations.

### External compatibility verification

Before declaring runtime compatibility, identify CK/compiler and game versions and verify:

- Inherited auto-state selection and state/ancestor fallback; Parent calls across an ancestor without a local implementation; empty lifecycle callbacks, populated callbacks, and nested transitions. Expect Parent dispatch and OnEndState → state update → OnBeginState ordering as described in the runtime reference.
- Conditional property visibility in CK and its generated storage metadata. Change an AutoReadOnly literal after saving, then load with the rebuilt script; confirm the current script value is observed rather than persisted mutable backing storage.
- Array None/search/bounds behavior, Float epsilon, string caching/casing, loop-local lifetime, instance-lock reentry, initialization/reset, and saved stacks against the documented contracts.
- Editor diagnostics, lexical completion/rename, intrinsic named parameters, and numeric hover after live source/dependency changes. In-memory model tests do not establish protocol, refresh, or UI behavior.

Retain disagreements as unresolved evidence rather than reproducing historical compiler bugs without a language requirement. Fallout 4 is a future target; Fallout 76 and Starfield implementation review remains deferred.

### Additional reference verification

- Resolve conflicting or incomplete Skyrim evidence for Conditional property kinds, typed-function fallthrough, state/ancestor dispatch, `Parent` call context, string comparison/caching, floating-point epsilon, identifier character sets, unary operators, and inherited state-count limits. Keep historical CK compiler bugs distinct from semantic requirements.
- Establish versioned CK/compiler and engine evidence for built-in state methods and callbacks, array operations and failures, local-variable lifetime, instance-lock reentrancy, initialization/reset, and saved-stack compatibility. Pure logic tests do not establish these external behaviors.
- Validate the [Fallout and Starfield dialect reference](../papyrus/dialects.md) against identified official compiler/SDK versions before implementing targets. Preserve separate evidence for Fallout 4, Fallout 76, and Starfield; do not infer compatibility from shared syntax or third-party tool support.
- Resolve Fallout 4 source conflicts about native/const storage, struct unboxing from `Var`, array allocation bounds and invalid mutation/search arguments, release/final call argument effects, and custom-event unregistration timing. Verify state-event parameter changes, initialization deadlocks, repeatable quest resets, and const saved-value/alias behavior against the identified engine.
- Obtain Starfield compiler/SDK evidence for `TryLockGuard` and alternative branches, guard requirements and access modifiers, `GetMatchingStructs`, special parameter types, and actual flags. The pinned reconstructed lexer, parser, and walkers disagree; a token inventory or decompiled temporary name is not a definitive source production. Establish Fallout 76 language/runtime evidence independently where public references remain unavailable.
