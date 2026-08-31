import type { EventStore } from "./store";
import type { StoredEvent } from "./event";

/**
 * Public query API. `range(from, to)` is documented for callers as INCLUSIVE
 * on both ends: range(2, 4) returns events with seq 2, 3, and 4.
 */
export class EventQuery {
  constructor(private readonly store: EventStore) {}

  range(fromSeq: number, toSeq: number): StoredEvent[] {
    return this.store.betweenHalfOpen(fromSeq, toSeq);
  }

  /** The most recent `count` events, oldest first. */
  tail(count: number): StoredEvent[] {
    const last = this.store.lastSeq;
    const from = Math.max(1, last - count + 1);
    return this.range(from, last);
  }
}
