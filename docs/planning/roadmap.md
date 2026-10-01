# Roadmap

This is the workspace's record of outstanding work and unresolved verification. The [README](../../README.md) and domain guides describe the current implementation. Remove completed items from this page and retain their lasting behavior in the relevant guide.

## Distribution and compatibility

- Establish executable and VS Code client distribution channels, installation guidance, and version policy when release channels are selected.
- Establish compatibility evidence with identified external environments and inputs before declaring supported game and tool combinations.

## Tooling and editor services

- Verify missing repository carrier recovery, ambiguous dual carriers, path alias retargeting, and PSC navigation across drives in a real editor.
- Verify LSP session cache reuse, buffer close recovery, dynamic watch registration, stale and cancelled navigation responses, and stdio message framing in a real editor. Compilation and in-memory Rust tests do not verify these session behaviors.
- Add `folio explain` for stable diagnostic codes, separating general explanations from rejection reasons in a specific project.
- Implement completion, references, and safe rename through the shared symbol and source model, with corresponding editor protocol handlers.
- Add range formatting, further lint rules, and fixes with revision checks as concrete needs arise.

## Additional targets

- Select a specific game target and SDK, then define its language differences, target constraints, PEX encoding, and verifiable lowering behavior.
- Design manifests and diagnostic presentation for multiple targets when a real project requires them.
