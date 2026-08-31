export class RateLimitedError extends Error {
  constructor(readonly key: string) {
    super(`rate limited: ${key}`);
    this.name = "RateLimitedError";
  }
}
