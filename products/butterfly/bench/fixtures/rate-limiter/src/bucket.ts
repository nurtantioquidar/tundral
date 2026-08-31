import type { Clock } from "./clock";
import { refillAmount } from "./refill";

/** Token bucket: capacity `burst`, refilled continuously at `ratePerSecond`. */
export class TokenBucket {
  private tokens: number;
  private lastRefillMs: number;
  private carryMs = 0;

  constructor(
    private readonly clock: Clock,
    private readonly ratePerSecond: number,
    private readonly burst: number,
  ) {
    this.tokens = burst;
    this.lastRefillMs = clock.nowMs();
  }

  private refill(): void {
    const now = this.clock.nowMs();
    const elapsed = now - this.lastRefillMs + this.carryMs;
    const { tokens, leftoverMs } = refillAmount(elapsed, this.ratePerSecond);
    this.tokens = Math.min(this.burst, this.tokens + tokens);
    this.carryMs = leftoverMs;
    this.lastRefillMs = now;
  }

  tryTake(count = 1): boolean {
    this.refill();
    if (this.tokens >= count) {
      this.tokens -= count;
      return true;
    }
    return false;
  }

  available(): number {
    this.refill();
    return this.tokens;
  }
}
