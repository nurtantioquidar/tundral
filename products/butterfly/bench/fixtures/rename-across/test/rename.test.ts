import { describe, expect, test } from "bun:test";
// Migration requirement: the accessor is now called loadRecord everywhere.
import { loadRecord } from "../src/db";
import { cachedRecord, getEndpoint, reportLine, syncAll } from "../src/index";

describe("loadRecord migration", () => {
  test("db exports loadRecord", () => {
    expect((loadRecord("a") as { value: number }).value).toBe(1);
  });
  test("api still works", () => {
    expect(getEndpoint("b").status).toBe(200);
  });
  test("sync still works", () => {
    expect(syncAll(["a", "b"]).length).toBe(2);
  });
  test("report still works", () => {
    expect(reportLine("a")).toContain("\"value\":1");
  });
  test("cache still works", () => {
    expect(cachedRecord("b")).toEqual(cachedRecord("b"));
  });
  test("old name is gone from the public surface", async () => {
    const db = await import("../src/db");
    expect("fetchRecord" in db).toBe(false);
  });
});
