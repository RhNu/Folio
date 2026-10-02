/** Project readiness is independent of the transport connection state. */
export interface ServerStatus {
  state: 'loading' | 'ready' | 'error';
  phase: string;
  message: string;
  completed?: number;
  total?: number;
  generation: number;
}

/** Reject obsolete project updates, but allow generations to restart on reconnect. */
export class ServerStatusTracker {
  current: ServerStatus | undefined;

  reset(): void { this.current = undefined; }

  accept(value: unknown): boolean {
    if (typeof value !== 'object' || value === null) { return false; }
    const update = value as ServerStatus;
    if (!['loading', 'ready', 'error'].includes(update.state)
      || typeof update.phase !== 'string' || typeof update.message !== 'string'
      || !Number.isSafeInteger(update.generation) || update.generation < 0
      || (this.current && update.generation < this.current.generation)) {
      return false;
    }
    for (const count of [update.completed, update.total]) {
      if (count !== undefined && (!Number.isSafeInteger(count) || count < 0)) { return false; }
    }
    this.current = update;
    return true;
  }

  get loading(): boolean { return this.current?.state === 'loading'; }

  get presentation(): { label: string; tooltip: string } | undefined {
    const current = this.current;
    if (!current) { return undefined; }
    const count = current.completed !== undefined && current.total !== undefined
      ? ` (${current.completed}/${current.total})` : '';
    const icon = current.state === 'loading' ? 'sync~spin'
      : current.state === 'ready' ? 'check' : 'warning';
    return {
      label: `$(${icon}) Folio${current.state === 'loading' ? `: ${current.message}${count}` : ''}`,
      tooltip: `${current.message}${count}\nClick to show the language server output.`,
    };
  }
}
