/* The scoring function. Given a pass and the two series it should be judged against, it answers one
 * question: how much IHSG-relative return did not acting cost?
 *
 * The output is always benchmark-adjusted. Absolute return is carried alongside so a row can show
 * its working, but `alphaPp` is the figure — a stock that gained 40 % in a market that gained 45 %
 * is a correct pass, and any surface that shows the 40 % on its own is lying about the decision. */

import { Effect } from "effect";
import { BenchmarkGap, DateOutsideSeries, MissingPriceData, UnresolvedEntry } from "./errors";
import type { ScoringError } from "./errors";
import type { Entry, PriceSeries } from "./schema";

const MS_PER_DAY = 86_400_000;

export type AlphaForgone = {
  readonly entryId: string;
  readonly ticker: string;
  /** Instrument return minus benchmark return, in percentage points. The figure. */
  readonly alphaPp: number;
  /** Shown only next to `alphaPp`, never on its own. */
  readonly instrumentReturnPct: number;
  readonly benchmarkReturnPct: number;
  /** Positive alpha means the pass forwent return: regret. Exactly zero is a correct pass — the
   * market was matched, and matching the index by doing nothing is the null result. */
  readonly isRegret: boolean;
  readonly daysHeld: number;
  readonly openClose: number;
  readonly finalClose: number;
};

/** Close on an exact day, or a typed reason why not. `onGap` distinguishes an instrument hole from
 * a benchmark hole; a range miss is the same failure whichever series it happens in. */
const closeOn = (
  series: PriceSeries,
  date: Date,
  onGap: () => MissingPriceData | BenchmarkGap,
): Effect.Effect<number, ScoringError> => {
  const first = series.points[0];
  const last = series.points[series.points.length - 1];
  if (first === undefined || last === undefined) return Effect.fail(onGap());

  const at = date.getTime();
  if (at < first.date.getTime() || at > last.date.getTime()) {
    return Effect.fail(
      new DateOutsideSeries({
        ticker: series.ticker,
        date,
        seriesStart: first.date,
        seriesEnd: last.date,
      }),
    );
  }

  const hit = series.points.find((point) => point.date.getTime() === at);
  return hit === undefined ? Effect.fail(onGap()) : Effect.succeed(hit.close);
};

/** Simple return over the window, as a ratio. Both closes are integers in the same unit, so the
 * unit cancels and the result is dimensionless. */
const returnPct = (open: number, final: number): number => (final / open - 1) * 100;

export const scoreEntry = (
  entry: Entry,
  instrument: PriceSeries,
  benchmark: PriceSeries,
): Effect.Effect<AlphaForgone, ScoringError> =>
  Effect.gen(function* () {
    const resolvedOn = entry.resolvedOn;
    if (resolvedOn === null) {
      return yield* Effect.fail(
        new UnresolvedEntry({ entryId: entry.id, ticker: entry.ticker, decidedOn: entry.decidedOn }),
      );
    }

    const openClose = yield* closeOn(instrument, entry.decidedOn, () => new MissingPriceData({ ticker: instrument.ticker, date: entry.decidedOn }));
    const finalClose = yield* closeOn(instrument, resolvedOn, () => new MissingPriceData({ ticker: instrument.ticker, date: resolvedOn }));
    const openBenchmark = yield* closeOn(benchmark, entry.decidedOn, () => new BenchmarkGap({ date: entry.decidedOn }));
    const finalBenchmark = yield* closeOn(benchmark, resolvedOn, () => new BenchmarkGap({ date: resolvedOn }));

    const instrumentReturnPct = returnPct(openClose, finalClose);
    const benchmarkReturnPct = returnPct(openBenchmark, finalBenchmark);
    const alphaPp = instrumentReturnPct - benchmarkReturnPct;

    return {
      entryId: entry.id,
      ticker: entry.ticker,
      alphaPp,
      instrumentReturnPct,
      benchmarkReturnPct,
      isRegret: alphaPp > 0,
      daysHeld: Math.round((resolvedOn.getTime() - entry.decidedOn.getTime()) / MS_PER_DAY),
      openClose,
      finalClose,
    };
  });
