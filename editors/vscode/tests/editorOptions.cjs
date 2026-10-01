const assert = require('node:assert/strict');
const { test } = require('node:test');
const { readEditorOptions, isNavigationUri, isPosition, isRange, isLocation, declarationText } = require('../out/editorOptions.js');

test('editor settings retain explicit false values alongside defaults', () => {
  const values = new Map([
    ['hover.documentation', false], ['codeLens.references', false], ['inlayHints.parameterNames', false],
  ]);
  const options = readEditorOptions((key, fallback) => values.get(key) ?? fallback);
  assert.equal(options.hover.documentation, false);
  assert.equal(options.hover.details, true);
  assert.equal(options.codeLens.enabled, true);
  assert.equal(options.codeLens.references, false);
  assert.equal(options.codeLens.implementations, true);
  assert.equal(options.codeLens.source, true);
  assert.equal(options.inlayHints.parameterNames, false);
});

test('navigation permits source and declaration documents and rejects executable schemes', () => {
  assert.equal(isNavigationUri('file:///C:/Mod/Source/Test.psc'), true);
  assert.equal(isNavigationUri('folio-declaration:/ObjectReference.psc'), true);
  for (const uri of ['command:workbench.action.openSettings', 'https://example.com', 'untitled:Test', 'file:/a\n', null]) {
    assert.equal(isNavigationUri(uri), false);
  }
});

test('navigation positions require nonnegative safe integer protocol coordinates', () => {
  assert.equal(isPosition({ line: 0, character: 0 }), true);
  for (const value of [null, {}, { line: -1, character: 0 }, { line: 1, character: 0.5 },
    { line: Number.MAX_SAFE_INTEGER + 1, character: 0 }, { line: 1, character: '0' }]) {
    assert.equal(isPosition(value), false);
  }
});

test('ranges reject reversed selections and locations validate both target and range', () => {
  const selection = { start: { line: 1, character: 2 }, end: { line: 1, character: 8 } };
  assert.equal(isRange(selection), true);
  assert.equal(isRange({ start: selection.end, end: selection.start }), false);
  assert.equal(isRange({ start: { line: 2, character: 0 }, end: { line: 1, character: 50 } }), false);
  assert.equal(isLocation({ uri: 'folio-declaration:/Script.psc', range: selection }), true);
  assert.equal(isLocation({ uri: 'command:arbitrary', range: selection }), false);
  assert.equal(isLocation({ uri: 'file:///test.psc' }), false);
});

test('declaration responses preserve exact source text and replace unavailable snapshots', () => {
  const source = '; API\r\nScriptName Demo\r\n';
  assert.equal(declarationText({ text: source, languageId: 'papyrus' }), source);
  assert.match(declarationText(null), /^; .*unavailable.*\n$/);
  for (const value of [undefined, '', {}, { text: 1, languageId: 'papyrus' }, { text: source, languageId: 'html' }]) {
    assert.throws(() => declarationText(value));
  }
});
