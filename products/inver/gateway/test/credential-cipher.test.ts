import { describe, expect, test } from "bun:test";
import { CredentialCipher } from "../src/upstreams/credential-cipher";

const key = Buffer.alloc(32, 7).toString("base64");
const otherKey = Buffer.alloc(32, 8).toString("base64");

describe("upstream credential encryption", () => {
  test("round-trips a credential without embedding the plaintext", () => {
    const cipher = CredentialCipher.fromBase64(key);
    const encrypted = cipher.encrypt("provider-1", "sk-provider-secret");

    expect(encrypted).toStartWith("v1.");
    expect(encrypted).not.toContain("sk-provider-secret");
    expect(cipher.decrypt("provider-1", encrypted)).toBe("sk-provider-secret");
  });

  test("binds ciphertext to its provider and deployment key", () => {
    const cipher = CredentialCipher.fromBase64(key);
    const encrypted = cipher.encrypt("provider-1", "secret");

    expect(() => cipher.decrypt("provider-2", encrypted)).toThrow("could not be decrypted");
    expect(() => CredentialCipher.fromBase64(otherKey).decrypt("provider-1", encrypted)).toThrow(
      "could not be decrypted",
    );
  });

  test("requires an exact 32-byte deployment key", () => {
    expect(CredentialCipher.fromEnvironment({})).toBeNull();
    expect(() => CredentialCipher.fromBase64(Buffer.alloc(16).toString("base64"))).toThrow(
      "base64-encoded 32-byte key",
    );
  });
});
