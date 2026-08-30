import { describe, expect, test } from "bun:test";
import { parseDuration } from "../src/duration";

describe("parseDuration", () => {
  test("single units", () => {
    expect(parseDuration("2h")).toBe(7200);
    expect(parseDuration("30m")).toBe(1800);
    expect(parseDuration("45s")).toBe(45);
  });
  test("combined units", () => {
    expect(parseDuration("1h30m")).toBe(5400);
    expect(parseDuration("1h2m3s")).toBe(3723);
    expect(parseDuration("2m30s")).toBe(150);
  });
  test("rejects invalid input", () => {
    expect(() => parseDuration("")).toThrow();
    expect(() => parseDuration("90x")).toThrow();
    expect(() => parseDuration("h")).toThrow();
    expect(() => parseDuration("30m1h30m")).toThrow();
  });
});
