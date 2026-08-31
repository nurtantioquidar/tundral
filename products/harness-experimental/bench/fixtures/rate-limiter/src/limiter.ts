import { TokenBucket } from "./bucket";
import type { Clock } from "./clock";
import { validateConfig, type LimiterConfig } from "./config";
import { RateLimitedError } from "./errors";

/** Per-key rate limiter used by the API gateway. */
export class RateLimiter {
  private readonly buckets = new Map<string, TokenBucket>();

  constructor(
    private readonly clock: Clock,
    private readonly config: LimiterConfig,
  ) {
    validateConfig(config);
  }

  /** Throws RateLimitedError when the key is over its budget. */
  check(key: string): void {
    let bucket = this.buckets.get(key);
    if (!bucket) {
      bucket = new TokenBucket(this.clock, this.config.ratePerSecond, this.config.burst);
      this.buckets.set(key, bucket);
    }
    if (!bucket.tryTake()) {
      throw new RateLimitedError(key);
    }
  }

  availableFor(key: string): number {
    return this.buckets.get(key)?.available() ?? this.config.burst;
  }
}
