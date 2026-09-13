/* Every way scoring can fail, as data. Nothing in this layer throws, returns null, or falls back to
 * zero: an unscoreable entry is a distinct value the caller has to handle, because a ledger that
 * quietly reports 0 pp where it means "I don't know" is worse than one that reports nothing. */

import { Data } from "effect";

/** The date sits inside the series' range but that day has no close for the instrument. */
export class MissingPriceData extends Data.TaggedError("MissingPriceData")<{
  readonly ticker: string;
  readonly date: Date;
}> {}

/** The same hole, on the benchmark side. Kept separate because a gap in IHSG invalidates every
 * entry scored against that window, not just one row. */
export class BenchmarkGap extends Data.TaggedError("BenchmarkGap")<{
  readonly date: Date;
}> {}

/** The date is before the first close or after the last one — the series cannot speak to it. */
export class DateOutsideSeries extends Data.TaggedError("DateOutsideSeries")<{
  readonly ticker: string;
  readonly date: Date;
  readonly seriesStart: Date;
  readonly seriesEnd: Date;
}> {}

/** The decision is still open. Not an anomaly — most of a healthy ledger is unresolved — but it has
 * no alpha yet, and the register renders it as such rather than as a zero. */
export class UnresolvedEntry extends Data.TaggedError("UnresolvedEntry")<{
  readonly entryId: string;
  readonly ticker: string;
  readonly decidedOn: Date;
}> {}

export type ScoringError = MissingPriceData | BenchmarkGap | DateOutsideSeries | UnresolvedEntry;
