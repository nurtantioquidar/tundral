export interface LimiterConfig {
  /** Sustained tokens per second. */
  ratePerSecond: number;
  /** Maximum burst size (bucket capacity). */
  burst: number;
}

export function validateConfig(config: LimiterConfig): void {
  if (config.ratePerSecond <= 0) {
    throw new Error("ratePerSecond must be positive");
  }
  if (config.burst < 1) {
    throw new Error("burst must be at least 1");
  }
}
