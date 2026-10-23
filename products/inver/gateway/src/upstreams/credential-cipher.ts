import { createCipheriv, createDecipheriv, randomBytes } from "node:crypto";

const FORMAT_VERSION = "v1";
const KEY_BYTES = 32;
const IV_BYTES = 12;

function decodeKey(value: string): Buffer {
  const key = Buffer.from(value.trim(), "base64");
  if (key.length !== KEY_BYTES) {
    throw new Error("UPSTREAM_CREDENTIAL_ENCRYPTION_KEY must be a base64-encoded 32-byte key");
  }
  return key;
}

function context(providerId: string): Buffer {
  // Frozen: stored ciphertexts are bound to this context, so it keeps the pre-Tundral name.
  return Buffer.from(`monet:upstream-provider:${providerId}:${FORMAT_VERSION}`, "utf8");
}

export class CredentialCipher {
  private constructor(private readonly key: Buffer) {}

  static fromBase64(key: string): CredentialCipher {
    return new CredentialCipher(decodeKey(key));
  }

  static fromEnvironment(
    environment: Record<string, string | undefined> = process.env,
  ): CredentialCipher | null {
    const key = environment.UPSTREAM_CREDENTIAL_ENCRYPTION_KEY?.trim();
    return key ? CredentialCipher.fromBase64(key) : null;
  }

  encrypt(providerId: string, plaintext: string): string {
    if (!plaintext) throw new Error("Upstream API key is required");
    const iv = randomBytes(IV_BYTES);
    const cipher = createCipheriv("aes-256-gcm", this.key, iv);
    cipher.setAAD(context(providerId));
    const ciphertext = Buffer.concat([cipher.update(plaintext, "utf8"), cipher.final()]);
    const tag = cipher.getAuthTag();
    return [
      FORMAT_VERSION,
      iv.toString("base64url"),
      tag.toString("base64url"),
      ciphertext.toString("base64url"),
    ].join(".");
  }

  decrypt(providerId: string, sealed: string): string {
    const [version, ivValue, tagValue, ciphertextValue, extra] = sealed.split(".");
    if (
      version !== FORMAT_VERSION ||
      !ivValue ||
      !tagValue ||
      !ciphertextValue ||
      extra !== undefined
    ) {
      throw new Error("Stored upstream credential has an unsupported format");
    }
    try {
      const iv = Buffer.from(ivValue, "base64url");
      const tag = Buffer.from(tagValue, "base64url");
      const ciphertext = Buffer.from(ciphertextValue, "base64url");
      if (iv.length !== IV_BYTES || tag.length !== 16 || ciphertext.length === 0) throw new Error("invalid");
      const decipher = createDecipheriv("aes-256-gcm", this.key, iv);
      decipher.setAAD(context(providerId));
      decipher.setAuthTag(tag);
      return Buffer.concat([decipher.update(ciphertext), decipher.final()]).toString("utf8");
    } catch {
      throw new Error("Stored upstream credential could not be decrypted");
    }
  }
}
