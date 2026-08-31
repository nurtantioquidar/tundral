/**
 * Compute how many whole tokens to add for `elapsedMs` at `ratePerSecond`,
 * along with the leftover milliseconds that have not yet earned a token —
 * callers must carry the leftover forward so slow drips still accumulate.
 */
export function refillAmount(
  elapsedMs: number,
  ratePerSecond: number,
): { tokens: number; leftoverMs: number } {
  const wholeSeconds = Math.floor(elapsedMs / 1000);
  return {
    tokens: wholeSeconds * ratePerSecond,
    leftoverMs: 0,
  };
}
