const MINIMUM_PASSWORD_LENGTH = 12;
const MAXIMUM_PASSWORD_LENGTH = 128;

export function validatePassword(password: string): void {
  if (password.length < MINIMUM_PASSWORD_LENGTH || password.length > MAXIMUM_PASSWORD_LENGTH) {
    throw new Error(
      `Password must be between ${MINIMUM_PASSWORD_LENGTH} and ${MAXIMUM_PASSWORD_LENGTH} characters`,
    );
  }
}

export async function hashPassword(password: string): Promise<string> {
  validatePassword(password);
  return Bun.password.hash(password, {
    algorithm: "argon2id",
    memoryCost: 65_536,
    timeCost: 3,
  });
}

export async function verifyPassword(password: string, passwordHash: string): Promise<boolean> {
  if (!password) return false;
  return Bun.password.verify(password, passwordHash).catch(() => false);
}
