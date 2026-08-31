import { beforeEach, describe, expect, test } from "bun:test";
import { EventStore } from "../src/store";
import { EventQuery } from "../src/query";

describe("EventQuery", () => {
  let store: EventStore;
  let query: EventQuery;

  beforeEach(() => {
    store = new EventStore();
    for (let i = 1; i <= 6; i++) {
      store.append("tick", { i });
    }
    query = new EventQuery(store);
  });

  test("range is inclusive on both ends", () => {
    const seqs = query.range(2, 4).map((event) => event.seq);
    expect(seqs).toEqual([2, 3, 4]);
  });

  test("single-element range", () => {
    const seqs = query.range(3, 3).map((event) => event.seq);
    expect(seqs).toEqual([3]);
  });

  test("tail returns the last N oldest-first", () => {
    const seqs = query.tail(3).map((event) => event.seq);
    expect(seqs).toEqual([4, 5, 6]);
  });

  test("tail larger than the log returns everything", () => {
    const seqs = query.tail(100).map((event) => event.seq);
    expect(seqs).toEqual([1, 2, 3, 4, 5, 6]);
  });

  test("store keeps half-open semantics for internal callers", () => {
    const seqs = store.betweenHalfOpen(2, 4).map((event) => event.seq);
    expect(seqs).toEqual([2, 3]);
  });
});
