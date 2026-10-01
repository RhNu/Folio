# Folio for VS Code

This extension provides Papyrus `.psc` file icons and syntax highlighting, and connects to `folio lsp` for diagnostics, hover with provider information, definition and declaration navigation, signature help, document symbols, semantic highlighting, and document formatting.

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

Definition and declaration navigation in root sources and PSC directory dependencies leads to the selected real `.psc` source. Declaration files, repository entries, and PEX dependencies expose provider information without treating historical source locations as files on the local machine.

## Settings

| Setting | Purpose |
| --- | --- |
| `folio.server.path` | Absolute path to the Folio executable or its directory; lookup continues to `PATH` if unavailable |
| `folio.server.manifestPath` | Optional manifest path, absolute or relative to the open folder; empty uses the folder's `folio.toml` |
| `folio.server.logFilter` | Server logging filter; defaults to `info` |

Changing these settings restarts the server. Lookup checks `folio.server.path` and then `PATH`. Only an extension development session falls back to the repository's `target/debug/folio` or `folio.exe`.

If the configured location is unavailable, the output panel records the source actually used. LSP messages use stdout and logs use stderr. Projects outside the repository can use a Folio executable on `PATH` or an explicit `folio.server.path`.

## Debug from the repository

1. Install JavaScript dependencies in `editors/vscode`, then open the repository in VS Code.
2. Select **Folio: Debug VS Code extension** in Run and Debug, then press F5. The prelaunch task builds the extension and Folio; the development host opens `fixtures/editor-project`.
3. Open `Source/Scripts/FolioArenaController.psc`. The **Folio Language Server** output panel reports connections and errors.
4. To debug the server, select **Folio: Attach to LSP process** in the repository window and choose the Folio process. This launch configuration requires CodeLLDB.

Restart the F5 session after changing extension or Rust code. **Folio: Restart Language Server** in the Command Palette restarts the server using the current binary.

## Publish

`npm run publish` compiles the client and invokes `vsce publish` to upload it to the VS Code Marketplace. The `folio-local` publisher in `package.json` is a local testing placeholder. Before publishing, select a registered publisher ID, update the version, and configure authentication according to the [VS Code publishing guide](https://code.visualstudio.com/api/working-with-extensions/publishing-extension). Publishing credentials are not stored in this repository.

The shared server behavior and protocol boundaries are described in [Tools and editor services](../../docs/architecture/tooling.md).
