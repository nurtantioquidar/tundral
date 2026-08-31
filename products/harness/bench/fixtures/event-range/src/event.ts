export interface StoredEvent {
  seq: number;
  kind: string;
  payload: unknown;
}

export function encode(event: StoredEvent): string {
  return JSON.stringify([event.seq, event.kind, event.payload]);
}

export function decode(line: string): StoredEvent {
  const [seq, kind, payload] = JSON.parse(line) as [number, string, unknown];
  return { seq, kind, payload };
}
