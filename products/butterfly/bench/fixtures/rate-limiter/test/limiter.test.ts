import { describe, expect, test } from "bun:test";
import { ManualClock, RateLimitedError, RateLimiter } from "../src/index";

// Customer report: "with ratePerSecond=2 our clients wait 500ms between
// retries but stay rate-limited forever." Waiting any amount of time must
// eventually earn tokens back.

describe("RateLimiter", () => {
  test("burst is honored then exhausted", () => {
    const clock = new ManualClock();
    const limiter = new RateLimiter(clock, { ratePerSecond: 2, burst: 2 });
    limiter.check("k");
    limiter.check("k");
    expect(() => limiter.check("k")).toThrow(RateLimitedError);
  });

  test("tokens accumulate across sub-second waits", () => {
    const clock = new ManualClock();
    const limiter = new RateLimiter(clock, { ratePerSecond: 2, burst: 2 });
    limiter.check("k");
    limiter.check("k");
    // Four 500ms waits at 2 tokens/sec = 4 earned tokens (capped at burst 2).
    for (let i = 0; i < 4; i++) {
      clock.advance(500);
    }
    expect(limiter.availableFor("k")).toBe(2);
    limiter.check("k");
    limiter.check("k");
    expect(() => limiter.check("k")).toThrow(RateLimitedError);
  });

  test("a single 750ms wait at 4/sec earns 3 tokens", () => {
    const clock = new ManualClock();
    const limiter = new RateLimiter(clock, { ratePerSecond: 4, burst: 10 });
    for (let i = 0; i < 10; i++) {
      limiter.check("k");
    }
    clock.advance(750);
    expect(limiter.availableFor("k")).toBe(3);
  });

  test("fractional leftovers carry forward", () => {
    const clock = new ManualClock();
    const limiter = new RateLimiter(clock, { ratePerSecond: 1, burst: 5 });
    for (let i = 0; i < 5; i++) {
      limiter.check("k");
    }
    clock.advance(900);
    expect(limiter.availableFor("k")).toBe(0);
    clock.advance(100);
    expect(limiter.availableFor("k")).toBe(1);
  });

  test("keys are isolated", () => {
    const clock = new ManualClock();
    const limiter = new RateLimiter(clock, { ratePerSecond: 1, burst: 1 });
    limiter.check("a");
    expect(() => limiter.check("a")).toThrow(RateLimitedError);
    limiter.check("b");
  });
});
