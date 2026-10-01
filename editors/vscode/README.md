# Folio for VS Code

This extension provides Papyrus `.psc` file icons and syntax highlighting, and connects to `folio lsp` for diagnostics, rich declaration and language hover, completion, signature help, references, inheritance navigation, CodeLens, parameter hints, verified rename, document and workspace symbols, semantic highlighting, and document formatting.

TextMate highlighting uses conventional scopes. The server refines resolved functions, events, types, properties, parameters, and variables with standard VS Code semantic token types. The packaged `icons/papyrus-psc.svg` is the default Papyrus language icon; a theme's specific `.psc` or Papyrus icon takes precedence, and the theme must allow language icons for the default to appear.

## Package and install locally

Use Node.js 22 or later. Run these commands from this directory:

```powershell
npm ci
npm run package
code --install-extension .\folio-vscode-0.2.0.vsix
```

Packaging compiles TypeScript and generates `folio-vscode-<version>.vsix`, with the version taken from `package.json`. Use that filename if the package version differs from the example. VS Code also supports **Install from VSIX** in the Extensions view. Repackage and install a new VSIX after changes.

The VSIX contains only the client. Install the Folio executable separately and add it to `PATH`, or configure `folio.server.path`.

## Open a project

Open a folder containing `folio.toml`. One client session serves `.psc` files in one project folder. The client forwards changes to manifests, source files, JSON declarations, and `.fdecl` declarations within the folder.

The server also registers watches for resolved manifests, source directories, and declaration carriers outside the folder, updating registrations after dependency changes. Watches include local repository files and candidate paths that do not yet exist. Opening or reopening identical text reuses the session's semantic view; saves and watch events refresh disk inputs.

Definition and declaration navigation in root sources and PSC directory dependencies leads to the selected real `.psc` source. Unsaved edits to existing PSC dependency declarations update their API and navigation positions without analyzing dependency bodies. Declaration files, repository entries, and PEX dependencies open a read-only API view with selected provider information. Historical generation paths are descriptive and never treated as files on the local machine. Open declaration views refresh when project inputs change or the server reconnects.

## Editor information and navigation

Hover begins with the owning script and selected package/source, followed by a complete declaration colored by the current theme. Declaration documentation, extra facts, and navigation links have separate sections. Papyrus `{ ... }` documentation is available from local sources, direct PSC dependencies, and newly generated declaration files. Existing PEX documentation strings are also preserved. Old declaration files remain readable; regenerate them to recover documentation from the original sources.

Hover also explains Skyrim keywords, standard declaration flags, built-in types, literals, and operators. It includes brief descriptions, applicable examples, and Creation Kit reference links without network requests. Literal hover shows the source value, including signed numbers, hexadecimal integers, and decoded string escapes; it does not evaluate arbitrary expressions or predict Skyrim's runtime string casing. `Self` and `Parent` retain their analyzed types, and array `Length` retains its intrinsic signature. Language help is available in root sources, selected PSC dependencies, and read-only API documents, including incomplete code. Comments and whitespace do not trigger it. The documentation setting controls descriptions, examples, and reference links; the details setting controls literal values and other extra facts.

CodeLens above script headers shows source context, parent navigation, derived scripts, and project references. Above members it shows references and callable overrides. Clicking a count opens the corresponding locations. Reference counts cover root project code and open root buffers; they do not count runtime calls or dependency bodies.

Completion follows visible local, inherited, imported, and selected dependency declarations. Signature help includes defaults and documentation. Parameter-name hints omit named arguments and obvious labels. Use the normal VS Code definition, references, implementation, symbol search, and rename actions.

Rename handles locals, parameters, and verifiable root members after checking conflicts and reanalyzing the proposed changes. Scripts, state members, native/event APIs, inherited or overridden APIs, dependency declarations, and unresolved projects are rejected. Runtime strings and consumers outside the project still require review.

The Folio status item opens the output panel. **Folio: Show Language Server Output** and **Folio: Restart Language Server** are also available in the Command Palette.

## Settings

| Setting | Purpose |
| --- | --- |
| `folio.server.path` | Absolute path to the Folio executable or its directory; lookup continues to `PATH` if unavailable |
| `folio.server.manifestPath` | Optional manifest path, absolute or relative to the open folder; empty uses the folder's `folio.toml` |
| `folio.server.logFilter` | Server logging filter; defaults to `info` |
| `folio.editor.hover.documentation` | Show declaration/language documentation, language examples, and reference links; defaults to `true` |
| `folio.editor.hover.details` | Show literal values, provider facts, limitations, and navigation links; defaults to `true` |
| `folio.editor.codeLens.enabled` | Enable Folio CodeLens; defaults to `true` |
| `folio.editor.codeLens.references` | Show project reference counts; defaults to `true` |
| `folio.editor.codeLens.implementations` | Show parent, derived-script, and override navigation; defaults to `true` |
| `folio.editor.codeLens.source` | Show script source context; defaults to `true` |
| `folio.editor.inlayHints.parameterNames` | Show parameter-name hints; defaults to `true` |

Changing `folio.server.*` settings restarts the server. Editor settings apply live. VS Code's own `editor.codeLens` and `editor.inlayHints.enabled` settings also control visibility. Lookup checks `folio.server.path` and then `PATH`. Only an extension development session falls back to the repository's `target/debug/folio` or `folio.exe`.

If the configured location is unavailable, the output panel records the source actually used. LSP messages use stdout and logs use stderr. Projects outside the repository can use a Folio executable on `PATH` or an explicit `folio.server.path`.

## Debug from the repository

1. Install JavaScript dependencies in `editors/vscode`, then open the repository in VS Code.
2. Select **Folio: Debug VS Code extension** in Run and Debug, then press F5. The prelaunch task builds the extension and Folio; the development host opens `fixtures/editor-project`.
3. Open `Source/Scripts/FolioArenaController.psc`. The **Folio Language Server** output panel reports connections and errors.
4. To debug the server, select **Folio: Attach to LSP process** in the repository window and choose the Folio process. This launch configuration requires CodeLLDB.

Restart the F5 session after changing extension or Rust code. **Folio: Restart Language Server** in the Command Palette restarts the server using the current binary.

## Publish

`npm run publish` compiles the client and invokes `vsce publish` to upload it to the VS Code Marketplace. The `folio-local` publisher in `package.json` is a local testing placeholder. Before publishing, select a registered publisher ID, update the version, and configure authentication according to the [VS Code publishing guide](https://code.visualstudio.com/api/working-with-extensions/publishing-extension). Publishing credentials are not stored in this repository.

The shared server behavior and protocol boundaries are described in `docs/architecture/tooling.md` in the Folio repository.
