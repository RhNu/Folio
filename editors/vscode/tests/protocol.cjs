// Isolated client adapter/provider probe. VS Code API objects are minimal shims;
// this measures the installed language-client providers and converters, not UI.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const Module = require('node:module');
const path = require('node:path');
const { performance } = require('node:perf_hooks');

const root = path.resolve(__dirname, '../../..');
const payload = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const folder = { uri: { fsPath: path.join(root, 'fixtures/editor-project') } };
class Position {
  constructor(line, character) { Object.assign(this, { line, character }); }
}
class Range {
  constructor(a, b, c, d) {
    this.start = new Position(a, b);
    this.end = new Position(c, d);
  }
}
class DocumentSymbol {
  constructor(name, detail, kind, range, selectionRange) {
    Object.assign(this, { name, detail, kind, range, selectionRange, children: [] });
  }
}
class SemanticTokens {
  constructor(data, resultId) { Object.assign(this, { data, resultId }); }
}
class EventEmitter { event = () => ({ dispose() {} }); dispose() {} }
const api = {
  Position, Range, DocumentSymbol, SemanticTokens, EventEmitter,
  CancellationError: class extends Error {},
  Disposable: class { constructor(callback) { this.dispose = callback; } },
  CodeActionKind: {},
  SemanticTokensLegend: class {},
  ExtensionMode: { Development: 2 },
  RelativePattern: class { constructor(base, pattern) { Object.assign(this, { base, pattern }); } },
  workspace: {
    workspaceFolders: [folder],
    getConfiguration: () => ({ get: (name, fallback) => name === 'path' ? path.join(root, 'target/release/folio') : fallback }),
    createFileSystemWatcher: () => ({ dispose() {} }),
    onDidChangeConfiguration: () => ({ dispose() {} }),
  },
  window: { createOutputChannel: () => ({ appendLine() {}, dispose() {} }) },
  commands: { registerCommand: () => ({ dispose() {} }) },
  languages: {
    registerDocumentSymbolProvider: () => ({ dispose() {} }),
    registerDocumentSemanticTokensProvider: () => ({ dispose() {} }),
  },
};
for (const name of ['CompletionItem', 'CodeLens', 'DocumentLink', 'CodeAction', 'Diagnostic',
  'CallHierarchyItem', 'TypeHierarchyItem', 'SymbolInformation', 'InlayHint']) {
  api[name] = class {};
}
class LaunchClient {
  constructor(_id, _name, server) { assert.equal(server.args.at(-1), 'lsp'); }
  async start() {}
  async dispose() {}
}
const originalLoad = Module._load;
Module._load = function (name, parent, main) {
  if (name === 'vscode') return api;
  if (name === 'vscode-languageclient/node') return { LanguageClient: LaunchClient, RevealOutputChannelOn: { Error: 4 } };
  return originalLoad.call(this, name, parent, main);
};
const library = path.resolve(path.dirname(require.resolve('vscode-languageclient/node')), '../..');
const converter = require(path.join(library, 'lib/common/protocolConverter.js')).createConverter();
const { DocumentSymbolFeature } = require(path.join(library, 'lib/common/documentSymbol.js'));
const { SemanticTokensFeature } = require(path.join(library, 'lib/common/semanticTokens.js'));
const { CancellationToken } = require('vscode-languageserver-protocol/node');
const client = {
  middleware: {},
  code2ProtocolConverter: {
    asDocumentSymbolParams: (document) => ({ textDocument: { uri: document.uri } }),
    asTextDocumentIdentifier: (document) => ({ uri: document.uri }),
  },
  protocol2CodeConverter: converter,
  sendRequest: (type) => Promise.resolve(payload[type.method === 'textDocument/documentSymbol' ? 0 : 1]),
  handleFailedRequest: (_type, _token, error) => { throw error; },
};
const [, symbols] = new DocumentSymbolFeature(client).registerLanguageProvider({ documentSelector: ['papyrus'] });
const [, tokens] = new SemanticTokensFeature(client).registerLanguageProvider({
  documentSelector: ['papyrus'], full: true, legend: { tokenTypes: [], tokenModifiers: [] },
});
async function main() {
  const extension = require('../out/extension.js');
  const started = performance.now();
  await extension.activate({ subscriptions: [], extensionPath: path.resolve(__dirname, '..'), extensionMode: 2 });
  const activation = performance.now() - started;
  await extension.deactivate();
  const timings = [];
  for (let i = 0; i < 100; i++) {
    const start = performance.now();
    const outline = await symbols.provideDocumentSymbols({ uri: 'file:///probe.psc' }, CancellationToken.None);
    const semantic = await tokens.full.provideDocumentSemanticTokens({ uri: 'file:///probe.psc' }, CancellationToken.None);
    timings.push(performance.now() - start);
    assert.equal(outline[0].name, 'Bench000');
    assert.equal(outline[0].children.length, payload[0][0].children.length);
    assert.deepEqual(Array.from(semantic.data), payload[1].data);
  }
  timings.sort((a, b) => a - b);
  console.log(JSON.stringify({ api: 'shim', startup_adapter_ms: activation,
    providers_and_conversion_median_ms: timings[50], samples: timings.length,
    child_symbols: payload[0][0].children.length, semantic_tokens: payload[1].data.length / 5 }));
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
