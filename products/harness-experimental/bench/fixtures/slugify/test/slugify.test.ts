import { describe, expect, test } from "bun:test";
import { slugify } from "../src/slugify";

describe("slugify", () => {
  test("lowercases and hyphenates", () => {
    expect(slugify("Hello World")).toBe("hello-world");
  });
  test("collapses non-alphanumeric runs", () => {
    expect(slugify("Rust & Bun -- Fast!!")).toBe("rust-bun-fast");
  });
  test("trims leading and trailing separators", () => {
    expect(slugify("  --Already Trimmed--  ")).toBe("already-trimmed");
  });
  test("keeps digits", () => {
    expect(slugify("Top 10 CLIs of 2026")).toBe("top-10-clis-of-2026");
  });
  test("empty input stays empty", () => {
    expect(slugify("")).toBe("");
  });
});
