/** Coalesces project generations without carrying them across server sessions. */
export class DeclarationRefresh {
  private generation: number | undefined;
  private pending = false;

  request(generation?: number): boolean {
    if (generation !== undefined) {
      if (!Number.isSafeInteger(generation) || generation < 0
        || (this.generation !== undefined && generation <= this.generation)) { return false; }
      this.generation = generation;
    }
    this.pending = true;
    return true;
  }

  take(): boolean {
    const pending = this.pending;
    this.pending = false;
    return pending;
  }

  reset(): void {
    this.generation = undefined;
    this.pending = false;
  }
}
