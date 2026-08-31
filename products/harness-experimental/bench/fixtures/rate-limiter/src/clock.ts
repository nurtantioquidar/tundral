export interface Clock {
  nowMs(): number;
}

export class SystemClock implements Clock {
  nowMs(): number {
    return Date.now();
  }
}

/** Deterministic clock for tests. */
export class ManualClock implements Clock {
  private current = 0;

  nowMs(): number {
    return this.current;
  }

  advance(ms: number): void {
    this.current += ms;
  }
}
