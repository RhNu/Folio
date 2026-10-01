import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  RevealOutputChannelOn,
  ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;
let fileWatcher: vscode.FileSystemWatcher | undefined;
let output: vscode.LogOutputChannel;
let pendingRestart: Promise<void> = Promise.resolve();

interface ExecutableResolution {
  command: string;
  source: string;
  configuredPathMissing?: string;
}

const executableName = process.platform === 'win32' ? 'folio.exe' : 'folio';

/** Checks an executable candidate without treating a directory as a launchable file. */
function isExecutableFile(candidate: string): boolean {
  try {
    if (!fs.statSync(candidate).isFile()) {
      return false;
    }
    fs.accessSync(candidate, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

/** Resolves an explicit file or directory, then PATH, then the development build. */
function resolveExecutable(configuredPath: string, context: vscode.ExtensionContext): ExecutableResolution {
  let configuredPathMissing: string | undefined;
  if (configuredPath) {
    if (!path.isAbsolute(configuredPath)) {
      throw new Error('folio.server.path must be an absolute executable or directory path.');
    }
    let candidate = configuredPath;
    try {
      if (fs.statSync(configuredPath).isDirectory()) {
        candidate = path.join(configuredPath, executableName);
      }
    } catch {
      // An unavailable configured path does not prevent trying PATH.
    }
    if (isExecutableFile(candidate)) {
      return { command: candidate, source: 'folio.server.path' };
    }
    configuredPathMissing = candidate;
  }

  const environmentPath = process.env.PATH ?? process.env.Path ?? '';
  for (const entry of environmentPath.split(path.delimiter)) {
    const directory = entry.trim().replace(/^"(.*)"$/, '$1');
    if (!directory) {
      continue;
    }
    const candidate = path.resolve(directory, executableName);
    if (isExecutableFile(candidate)) {
      return { command: candidate, source: 'PATH', configuredPathMissing };
    }
  }

  if (context.extensionMode === vscode.ExtensionMode.Development) {
    const candidate = path.resolve(context.extensionPath, '..', '..', 'target', 'debug', executableName);
    if (isExecutableFile(candidate)) {
      return { command: candidate, source: 'target/debug (extension development)', configuredPathMissing };
    }
  }

  const configuredHint = configuredPathMissing ? ` Configured path not found: ${configuredPathMissing}.` : '';
  const debugHint = context.extensionMode === vscode.ExtensionMode.Development
    ? ' Build folio-cli for the development fallback.'
    : '';
  throw new Error(`Folio executable not found in folio.server.path or PATH.${configuredHint}${debugHint}`);
}

/** Selects the project owning the active Papyrus document, or the only open folder. */
function projectFolder(): vscode.WorkspaceFolder {
  const document = vscode.window.activeTextEditor?.document;
  if (document?.languageId === 'papyrus' && document.uri.scheme === 'file') {
    const folder = vscode.workspace.getWorkspaceFolder(document.uri);
    if (folder) {
      return folder;
    }
  }
  const folders = vscode.workspace.workspaceFolders;
  if (folders?.length === 1) {
    return folders[0];
  }
  throw new Error('Open one Folio project folder, then open a .psc file.');
}

/** Resolves launch inputs without parsing the Folio manifest in TypeScript. */
function serverLaunch(context: vscode.ExtensionContext, folder: vscode.WorkspaceFolder): {
  command: string;
  args: string[];
  source: string;
  configuredPathMissing?: string;
} {
  const settings = vscode.workspace.getConfiguration('folio.server', folder.uri);
  const configuredPath = settings.get<string>('path', '').trim();
  const executable = resolveExecutable(configuredPath, context);

  const configuredManifest = settings.get<string>('manifestPath', '').trim();
  const manifest = configuredManifest
    ? path.resolve(folder.uri.fsPath, configuredManifest)
    : path.join(folder.uri.fsPath, 'folio.toml');
  if (!fs.existsSync(manifest) || !fs.statSync(manifest).isFile()) {
    throw new Error(`Folio manifest not found: ${manifest}. Open its project folder or set folio.server.manifestPath.`);
  }

  const logFilter = settings.get<string>('logFilter', 'info').trim() || 'info';
  const args = ['--log-filter', logFilter];
  if (configuredManifest) {
    args.push('--manifest-path', manifest);
  }
  args.push('lsp');
  return { ...executable, args };
}

/** Starts one stdio LSP process in the selected Folio project folder. */
async function startServer(context: vscode.ExtensionContext): Promise<void> {
  const folder = projectFolder();
  const launch = serverLaunch(context, folder);
  const pattern = new vscode.RelativePattern(folder, '**/{folio.toml,*.psc,*.pex,*.json,*.fdecl}');
  const watcher = vscode.workspace.createFileSystemWatcher(pattern);
  const serverOptions: ServerOptions = {
    command: launch.command,
    args: launch.args,
    options: { cwd: folder.uri.fsPath },
  };
  // The client package's protocol type omits VS Code's URI-based RelativePattern shape.
  const documentSelector: vscode.DocumentSelector = [
    { scheme: 'file', language: 'papyrus', pattern: new vscode.RelativePattern(folder, '**/*.psc') },
  ];
  const clientOptions: LanguageClientOptions = {
    documentSelector: documentSelector as LanguageClientOptions['documentSelector'],
    workspaceFolder: folder,
    synchronize: { fileEvents: watcher },
    outputChannel: output,
    revealOutputChannelOn: RevealOutputChannelOn.Error,
  };
  const nextClient = new LanguageClient('folio', 'Folio Language Server', serverOptions, clientOptions);
  if (launch.configuredPathMissing) {
    output.appendLine(`Configured Folio executable not found: ${launch.configuredPathMissing}; using ${launch.source}.`);
  }
  output.appendLine(`Starting Folio LSP from ${launch.source}: ${launch.command}`);
  output.appendLine(`Project folder: ${folder.uri.fsPath}`);
  try {
    await nextClient.start();
    client = nextClient;
    fileWatcher = watcher;
    output.appendLine('Folio LSP connected.');
  } catch (error) {
    watcher.dispose();
    await nextClient.dispose();
    throw error;
  }
}

/** Stops the client and its file watcher before a restart or deactivation. */
async function stopServer(): Promise<void> {
  const activeClient = client;
  const activeWatcher = fileWatcher;
  client = undefined;
  fileWatcher = undefined;
  try {
    if (activeClient) {
      output.appendLine('Stopping Folio LSP.');
      await activeClient.dispose();
    }
  } finally {
    activeWatcher?.dispose();
  }
}

/** Serializes restarts so a configuration change cannot leave two servers running. */
function restartServer(context: vscode.ExtensionContext): Promise<void> {
  pendingRestart = pendingRestart.then(async () => {
    await stopServer();
    await startServer(context);
  }).catch((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error);
    output.appendLine(`Folio LSP startup failed: ${message}`);
    void vscode.window.showErrorMessage(`Folio LSP: ${message}`, 'Show Output').then((choice) => {
      if (choice === 'Show Output') {
        output.show(true);
      }
    });
  });
  return pendingRestart;
}

/** Registers the Papyrus LSP client and its development controls. */
export async function activate(context: vscode.ExtensionContext): Promise<void> {
  output = vscode.window.createOutputChannel('Folio Language Server', { log: true });
  context.subscriptions.push(output);
  context.subscriptions.push(vscode.commands.registerCommand('folio.restartServer', () => restartServer(context)));
  context.subscriptions.push(vscode.workspace.onDidChangeConfiguration((event) => {
    if (event.affectsConfiguration('folio.server')) {
      void restartServer(context);
    }
  }));
  await restartServer(context);
}

/** Releases the LSP process when VS Code unloads the extension. */
export async function deactivate(): Promise<void> {
  await pendingRestart;
  await stopServer();
}
