import * as vscode from 'vscode';
import { LanguageClient } from 'vscode-languageclient/node';
import {
  declarationText, isLocation, isNavigationUri, isPosition, isRange, ProtocolPosition, ProtocolRange,
} from './editorOptions';

function navigationUri(value: unknown): vscode.Uri {
  if (!isNavigationUri(value)) { throw new Error('Folio navigation requires a source or declaration URI.'); }
  const uri = vscode.Uri.parse(value, true);
  if (!['file', 'folio-declaration'].includes(uri.scheme.toLowerCase()) || !uri.path) {
    throw new Error('Folio navigation received an invalid document URI.');
  }
  return uri;
}

function position(value: ProtocolPosition): vscode.Position {
  return new vscode.Position(value.line, value.character);
}

function range(value: ProtocolRange): vscode.Range {
  return new vscode.Range(position(value.start), position(value.end));
}

/** Fetches declaration snapshots on demand; buffers belong exclusively to the server. */
export class DeclarationDocuments implements vscode.TextDocumentContentProvider, vscode.Disposable {
  private readonly changes = new vscode.EventEmitter<vscode.Uri>();
  readonly onDidChange = this.changes.event;

  constructor(private readonly activeClient: () => LanguageClient | undefined) {}

  async provideTextDocumentContent(uri: vscode.Uri, token: vscode.CancellationToken): Promise<string> {
    const client = this.activeClient();
    if (!client) { throw new Error('The Folio language server is not connected.'); }
    const result = await client.sendRequest<unknown>(
      'folio/declarationContent', { uri: uri.toString() }, token,
    );
    return declarationText(result);
  }

  /** Requery only open declaration buffers when the server's project inputs change. */
  refresh(): void {
    for (const document of vscode.workspace.textDocuments) {
      if (document.uri.scheme === 'folio-declaration') { this.changes.fire(document.uri); }
    }
  }

  dispose(): void { this.changes.dispose(); }
}

/** Registers a small command allowlist used by server-generated navigation links. */
export function registerNavigation(context: vscode.ExtensionContext, output: vscode.LogOutputChannel): void {
  const guarded = (name: string, action: (...args: unknown[]) => Promise<unknown>): void => {
    context.subscriptions.push(vscode.commands.registerCommand(name, async (...args: unknown[]) => {
      try {
        return await action(...args);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        output.warn(`${name}: ${message}`);
        void vscode.window.showErrorMessage(message);
        return undefined;
      }
    }));
  };
  const open = async (target: unknown, selection?: unknown): Promise<void> => {
    const uri = navigationUri(target);
    if (selection !== undefined && !isRange(selection)) {
      throw new Error('Folio navigation received an invalid source range.');
    }
    let document = await vscode.workspace.openTextDocument(uri);
    if (uri.scheme === 'folio-declaration' && document.languageId !== 'papyrus') {
      document = await vscode.languages.setTextDocumentLanguage(document, 'papyrus');
    }
    await vscode.window.showTextDocument(document, {
      preview: true,
      selection: selection === undefined ? undefined : range(selection as ProtocolRange),
    });
  };
  guarded('folio.openLocation', open);
  guarded('folio.showSource', (target) => open(target));
  const showLocations = async (target: unknown, anchor: unknown, targets: unknown): Promise<void> => {
    const uri = navigationUri(target);
    if (!isPosition(anchor) || !Array.isArray(targets) || !targets.every(isLocation)) {
      throw new Error('Folio navigation received invalid symbol locations.');
    }
    const locations = targets.map((location) => new vscode.Location(navigationUri(location.uri), range(location.range)));
    // showReferences is the built-in peek UI for both references and implementations.
    await vscode.commands.executeCommand('editor.action.showReferences', uri, position(anchor), locations);
  };
  guarded('folio.showReferences', showLocations);
  guarded('folio.showImplementations', showLocations);
}
