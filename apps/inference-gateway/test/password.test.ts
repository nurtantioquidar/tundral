import { describe, expect, test } from "bun:test";
import { hashPassword, validatePassword, verifyPassword } from "../src/accounts/password";

describe("customer passwords", () => {
  test("hashes and verifies without retaining plaintext", async () => {
    const password = "correct horse battery staple";
    const hash = await hashPassword(password);
    expect(hash).not.toContain(password);
    expect(await verifyPassword(password, hash)).toBe(true);
    expect(await verifyPassword("incorrect password", hash)).toBe(false);
  });

  test("enforces the account password boundary", () => {
    expect(() => validatePassword("too-short")).toThrow();
    expect(() => validatePassword("x".repeat(129))).toThrow();
  });
});
