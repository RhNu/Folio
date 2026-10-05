import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  RevealOutputChannelOn,
  ServerOptions,
  State,
} from 'vscode-languageclient/node';
import { navigationCommands, readEditorOptions } from './editorOptions';
import { DeclarationDocuments, registerNavigation } from './navigation';
import { ServerStatusTracker } from './serverStatus';

let client: LanguageClient | undefined;
let fileWatcher: vscode.FileSystemWatcher | undefined;
let output: vscode.LogOutputChannel;
let declarations: DeclarationDocuments;
let status: vscode.StatusBarItem;
let clientEvents: vscode.Disposable[] = [];
let pendingRestart: Promise<void> = Promise.resolve();
let shuttingDown = false;

interface ExecutableResolution {
  command: string;
  source: string;
  configuredPathMissing?: string;
}

const executableName = process.platform === 'win32' ? 'folio.exe' : 'folio';

/** Reads only presentation options; changing these does not restart the process. */
function editorSettings(folder: vscode.WorkspaceFolder | undefined) {
  const settings = vscode.workspace.getConfiguration('folio.editor', folder?.uri);
  return { folio: {
    editor: readEditorOptions((key, fallback) => settings.get<boolean>(key, fallback)),
    clientCommands: true,
    declarationDocuments: true,
    status: true,
  } };
}

async function updateEditorSettings(activeClient: LanguageClient): Promise<void> {
  await activeClient.sendNotification('workspace/didChangeConfiguration', {
    settings: editorSettings(activeClient.clientOptions.workspaceFolder),
  });
}

function showStatus(label: string, tooltip: string): void {
  status.text = label;
  status.tooltip = tooltip;
  status.show();
}

/** Measures the full client wait, including server loading and query queues. */
async function timedQuery<T>(method: string, action: () => T | Thenable<T>): Promise<T> {
  const started = performance.now();
  try {
    return await action();
  } finally {
    output.debug(`Folio ${method}: ${(performance.now() - started).toFixed(1)} ms.`);
  }
}

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
  // PSC dependencies can live outside the workspace; the server validates project ownership.
  const documentSelector: vscode.DocumentSelector = [
    { scheme: 'file', language: 'papyrus' },
    { scheme: 'folio-declaration', language: 'papyrus' },
  ];
  const projectStatus = new ServerStatusTracker();
  const clientOptions: LanguageClientOptions = {
    documentSelector: documentSelector as LanguageClientOptions['documentSelector'],
    workspaceFolder: folder,
    synchronize: { fileEvents: watcher },
    outputChannel: output,
    revealOutputChannelOn: RevealOutputChannelOn.Error,
    initializationOptions: () => editorSettings(folder),
    markdown: { isTrusted: { enabledCommands: navigationCommands }, supportHtml: false },
    middleware: {
      provideCompletionItem: (document, position, context, token, next) =>
        timedQuery('completion', () => next(document, position, context, token)),
      resolveCompletionItem: (item, token, next) =>
        timedQuery('completion resolve', () => next(item, token)),
      provideHover: (document, position, token, next) =>
        timedQuery('hover', () => next(document, position, token)),
      provideSignatureHelp: (document, position, context, token, next) =>
        timedQuery('signature help', () => next(document, position, context, token)),
      provideDocumentSemanticTokens: (document, token, next) =>
        projectStatus.loading ? null : next(document, token),
      provideDocumentSemanticTokensEdits: (document, previous, token, next) =>
        projectStatus.loading ? null : next(document, previous, token),
      provideDocumentRangeSemanticTokens: (document, range, token, next) =>
        projectStatus.loading ? null : next(document, range, token),
      provideCodeLenses: (document, token, next) =>
        projectStatus.loading ? [] : next(document, token),
      provideInlayHints: (document, range, token, next) =>
        projectStatus.loading ? [] : next(document, range, token),
    },
  };
  const nextClient = new LanguageClient('folio', 'Folio Language Server', serverOptions, clientOptions);
  if (launch.configuredPathMissing) {
    output.appendLine(`Configured Folio executable not found: ${launch.configuredPathMissing}; using ${launch.source}.`);
  }
  output.appendLine(`Starting Folio LSP from ${launch.source}: ${launch.command}`);
  output.appendLine(`Project folder: ${folder.uri.fsPath}`);
  // Register before initialize: loading may arrive before start resolves.
  const events = [nextClient.onNotification('folio/status', (params: unknown) => {
    if (shuttingDown || !projectStatus.accept(params)) { return; }
    const presentation = projectStatus.presentation!;
    showStatus(presentation.label, presentation.tooltip);
  }), nextClient.onDidChangeState((event) => {
    if (shuttingDown) { return; }
    if (event.newState === State.Running) {
      if (!projectStatus.current) {
        showStatus('$(check) Folio', 'Folio language server connected. Click to show its output.');
      }
      // Running precedes initialized. Refresh only after that handshake finishes.
      void nextClient.start().then(async () => {
        if (shuttingDown || client !== nextClient) { return; }
        await updateEditorSettings(nextClient);
        if (!shuttingDown && client === nextClient) { declarations.refresh(); }
      }).catch((error: unknown) => {
        if (!shuttingDown) { output.error(`Folio reconnect refresh failed: ${String(error)}`); }
      });
    } else if (event.newState === State.Starting) {
      declarations.reset();
      projectStatus.reset();
      showStatus('$(sync~spin) Folio', 'Folio language server is starting.');
    } else {
      declarations.reset();
      projectStatus.reset();
      showStatus('$(warning) Folio', 'Folio language server stopped. Use Folio: Restart Language Server.');
    }
  })];
  try {
    await nextClient.start();
    client = nextClient;
    fileWatcher = watcher;
    clientEvents.push(nextClient.onNotification('folio/projectChanged', (params: { generation: number }) => {
      output.debug(`Folio project changed: generation ${params.generation}.`);
      declarations.refresh(params.generation);
    }));
    clientEvents.push(...events);
    // Configuration can change while initialize is in flight, before client is assigned.
    await updateEditorSettings(nextClient);
    declarations.refresh();
    if (!projectStatus.current) {
      showStatus('$(check) Folio', 'Folio language server connected. Click to show its output.');
    }
    output.appendLine('Folio LSP connected.');
  } catch (error) {
    for (const event of events) { event.dispose(); }
    if (client === nextClient) {
      client = undefined;
      fileWatcher = undefined;
      for (const event of clientEvents) { event.dispose(); }
      clientEvents = [];
    }
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
  declarations.reset();
  fileWatcher = undefined;
  for (const event of clientEvents) { event.dispose(); }
  clientEvents = [];
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
    if (shuttingDown) { return; }
    showStatus('$(sync~spin) Folio', 'Folio language server is starting.');
    await stopServer();
    await startServer(context);
  }).catch((error: unknown) => {
    if (shuttingDown) { return; }
    const message = error instanceof Error ? error.message : String(error);
    output.appendLine(`Folio LSP startup failed: ${message}`);
    showStatus('$(warning) Folio', `Folio language server could not start: ${message}`);
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
  shuttingDown = false;
  output = vscode.window.createOutputChannel('Folio Language Server', { log: true });
  // Keep the logger alive until asynchronous client shutdown has completed.
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left);
  status.command = 'folio.showOutput';
  context.subscriptions.push(status);
  context.subscriptions.push(vscode.commands.registerCommand('folio.showOutput', () => output.show(true)));
  registerNavigation(context, output);
  declarations = new DeclarationDocuments(() => client);
  context.subscriptions.push(declarations);
  context.subscriptions.push(vscode.workspace.registerTextDocumentContentProvider('folio-declaration', declarations));
  context.subscriptions.push(vscode.commands.registerCommand('folio.restartServer', () => restartServer(context)));
  context.subscriptions.push(vscode.workspace.onDidChangeConfiguration((event) => {
    if (event.affectsConfiguration('folio.server')) {
      void restartServer(context);
    } else if (event.affectsConfiguration('folio.editor') && client) {
      const activeClient = client;
      output.info('Updating Folio editor presentation settings.');
      void updateEditorSettings(activeClient)
        .catch((error: unknown) => {
          if (!shuttingDown) { output.error(`Folio editor configuration update failed: ${String(error)}`); }
        });
    }
  }));
  await restartServer(context);
}

/** Releases the LSP process when VS Code unloads the extension. */
export async function deactivate(): Promise<void> {
  shuttingDown = true;
  try {
    await pendingRestart;
    await stopServer();
  } finally {
    output?.dispose();
  }
}
