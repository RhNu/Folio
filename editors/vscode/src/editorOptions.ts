/** Live presentation settings shared by initialization and configuration updates. */
export interface EditorOptions {
  hover: { documentation: boolean; details: boolean };
  codeLens: { enabled: boolean; references: boolean; implementations: boolean; source: boolean };
  inlayHints: { parameterNames: boolean };
}

type ReadSetting = (key: string, fallback: boolean) => boolean;

export function readEditorOptions(read: ReadSetting): EditorOptions {
  return {
    hover: { documentation: read('hover.documentation', true), details: read('hover.details', true) },
    codeLens: {
      enabled: read('codeLens.enabled', true),
      references: read('codeLens.references', true),
      implementations: read('codeLens.implementations', true),
      source: read('codeLens.source', true),
    },
    inlayHints: { parameterNames: read('inlayHints.parameterNames', true) },
  };
}

export interface ProtocolPosition { line: number; character: number }
export interface ProtocolRange { start: ProtocolPosition; end: ProtocolPosition }
export interface ProtocolLocation { uri: string; range: ProtocolRange }

/** Commands may originate in server markdown, so treat all arguments as untrusted. */
export function isPosition(value: unknown): value is ProtocolPosition {
  if (!value || typeof value !== 'object') { return false; }
  const position = value as Partial<ProtocolPosition>;
  return Number.isSafeInteger(position.line) && Number.isSafeInteger(position.character)
    && position.line! >= 0 && position.character! >= 0;
}

export function isRange(value: unknown): value is ProtocolRange {
  if (!value || typeof value !== 'object') { return false; }
  const range = value as Partial<ProtocolRange>;
  return isPosition(range.start) && isPosition(range.end)
    && (range.start.line < range.end.line
      || (range.start.line === range.end.line && range.start.character <= range.end.character));
}

/** Restricts navigation to source documents and Folio's read-only declarations. */
export function isNavigationUri(value: unknown): value is string {
  return typeof value === 'string' && /^(?:file|folio-declaration):/i.test(value)
    && !/[\u0000-\u001f\u007f]/u.test(value);
}

export function isLocation(value: unknown): value is ProtocolLocation {
  if (!value || typeof value !== 'object') { return false; }
  const location = value as Partial<ProtocolLocation>;
  return isNavigationUri(location.uri) && isRange(location.range);
}

/** An unavailable provider must replace an open snapshot, rather than leave stale API text. */
export function declarationText(value: unknown): string {
  if (value === null) { return '; Selected declaration is unavailable in the current project.\n'; }
  if (!value || typeof value !== 'object') { throw new Error('The Folio language server returned an invalid declaration document.'); }
  const content = value as { text?: unknown; languageId?: unknown };
  if (typeof content.text !== 'string' || content.languageId !== 'papyrus') {
    throw new Error('The Folio language server returned an invalid declaration document.');
  }
  return content.text;
}

export const navigationCommands = [
  'folio.openLocation', 'folio.showReferences', 'folio.showImplementations', 'folio.showSource',
] as const;
