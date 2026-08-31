import { decode, encode, type StoredEvent } from "./event";

/** Append-only in-memory event log with sequence-indexed reads. */
export class EventStore {
  private lines: string[] = [];
  private nextSeq = 1;

  append(kind: string, payload: unknown): StoredEvent {
    const event: StoredEvent = { seq: this.nextSeq++, kind, payload };
    this.lines.push(encode(event));
    return event;
  }

  /** Events with seq in the half-open interval [fromSeq, toSeq). */
  betweenHalfOpen(fromSeq: number, toSeq: number): StoredEvent[] {
    return this.lines
      .map(decode)
      .filter((event) => event.seq >= fromSeq && event.seq < toSeq);
  }

  get lastSeq(): number {
    return this.nextSeq - 1;
  }
}
