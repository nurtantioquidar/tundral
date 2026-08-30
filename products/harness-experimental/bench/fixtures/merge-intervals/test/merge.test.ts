import { describe, expect, test } from "bun:test";
import { mergeIntervals } from "../src/merge";

describe("mergeIntervals", () => {
  test("merges overlapping intervals", () => {
    expect(mergeIntervals([[1, 4], [2, 6]])).toEqual([[1, 6]]);
  });
  test("merges touching intervals", () => {
    expect(mergeIntervals([[1, 3], [3, 5]])).toEqual([[1, 5]]);
  });
  test("keeps disjoint intervals apart", () => {
    expect(mergeIntervals([[1, 2], [4, 5]])).toEqual([[1, 2], [4, 5]]);
  });
  test("handles containment and unsorted input", () => {
    expect(mergeIntervals([[5, 9], [1, 10], [2, 3]])).toEqual([[1, 10]]);
  });
  test("handles empty input", () => {
    expect(mergeIntervals([])).toEqual([]);
  });
});
