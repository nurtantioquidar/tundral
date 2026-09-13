/* Set-level rules and the one entry point the UI calls.
 *
 * Two things live here rather than in `score.ts` because they are properties of the ledger as a
 * whole, not of a single pass: the decision-density cap, and the total. Everything the register
 * renders comes out of `loadLedger` already resolved — React never sees an Effect. */

import { Data, Effect } from "effect";
import type { ParseResult } from "effect";
import { BenchmarkGap, MissingPriceData } from "./errors";
import type { ScoringError } from "./errors";
import { scoreEntry } from "./score";
import type { AlphaForgone } from "./score";
import { decodeEntries, decodePrices } from "./schema";
import type { Entry, PriceSeries } from "./schema";

const MS_PER_DAY = 86_400_000;

export const BENCHMARK_TICKER = "IHSG";

/* Logging fifty passes a week to farm one vindication is the obvious exploit, so the ledger refuses
 * to load when it sees that shape. The number is a judgement call, not a derived constant — it is
 * low enough that a genuine week of research fits and a farming run does not. */
export const DECISIONS_PER_WEEK_CAP = 3;

export class DecisionDensityExceeded extends Data.TaggedError("DecisionDensityExceeded")<{
  readonly isoWeek: string;
  readonly count: number;
  readonly cap: number;
}> {}

export type LedgerRow =
  | { readonly _tag: "Resolved"; readonly entry: Entry; readonly alpha: AlphaForgone }
  | { readonly _tag: "Open"; readonly entry: Entry; readonly daysHeld: number };

export type Ledger = {
  readonly rows: ReadonlyArray<LedgerRow>;
  /** Alpha forgone across resolved rows, in percentage points. Open rows contribute nothing —
   * they have no answer yet, and counting them as zero would flatter the author. */
  readonly totalAlphaPp: number;
  readonly resolvedCount: number;
  readonly openCount: number;
};

/** ISO-8601 week key, computed in UTC. Weeks are the density window because that is the cadence a
 * person actually researches at. */
const isoWeekKey = (date: Date): string => {
  const cursor = new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate()));
  const weekday = cursor.getUTCDay() === 0 ? 7 : cursor.getUTCDay();
  cursor.setUTCDate(cursor.getUTCDate() + 4 - weekday);
  const yearStart = Date.UTC(cursor.getUTCFullYear(), 0, 1);
  const week = Math.ceil(((cursor.getTime() - yearStart) / MS_PER_DAY + 1) / 7);
  return `${cursor.getUTCFullYear()}-W${String(week).padStart(2, "0")}`;
};

export const checkDecisionDensity = (
  entries: ReadonlyArray<Entry>,
  cap: number = DECISIONS_PER_WEEK_CAP,
): Effect.Effect<void, DecisionDensityExceeded> => {
  const perWeek = new Map<string, number>();
  for (const entry of entries) {
    const key = isoWeekKey(entry.decidedOn);
    perWeek.set(key, (perWeek.get(key) ?? 0) + 1);
  }
  for (const [isoWeek, count] of perWeek) {
    if (count > cap) return Effect.fail(new DecisionDensityExceeded({ isoWeek, count, cap }));
  }
  return Effect.void;
};

export const buildLedger = (
  entries: ReadonlyArray<Entry>,
  prices: ReadonlyArray<PriceSeries>,
  asOf: Date,
): Effect.Effect<Ledger, ScoringError | DecisionDensityExceeded> =>
  Effect.gen(function* () {
    yield* checkDecisionDensity(entries);

    const book = new Map(prices.map((series) => [series.ticker, series]));
    const benchmark = book.get(BENCHMARK_TICKER);
    if (benchmark === undefined) return yield* Effect.fail(new BenchmarkGap({ date: asOf }));

    /* Newest decision first. A register is written in append order, but the row a person came to
     * check is almost always the most recent one. */
    const ordered = [...entries].sort((a, b) => b.decidedOn.getTime() - a.decidedOn.getTime());

    const rows = yield* Effect.forEach(ordered, (entry): Effect.Effect<LedgerRow, ScoringError> => {
      if (entry.resolvedOn === null) {
        return Effect.succeed({
          _tag: "Open",
          entry,
          daysHeld: Math.round((asOf.getTime() - entry.decidedOn.getTime()) / MS_PER_DAY),
        });
      }
      const instrument = book.get(entry.ticker);
      if (instrument === undefined) {
        return Effect.fail(new MissingPriceData({ ticker: entry.ticker, date: entry.decidedOn }));
      }
      return scoreEntry(entry, instrument, benchmark).pipe(
        Effect.map((alpha): LedgerRow => ({ _tag: "Resolved", entry, alpha })),
      );
    });

    const resolved = rows.filter((row) => row._tag === "Resolved");
    return {
      rows,
      totalAlphaPp: resolved.reduce((sum, row) => sum + row.alpha.alphaPp, 0),
      resolvedCount: resolved.length,
      openCount: rows.length - resolved.length,
    };
  });

/** Decode both files and score everything. The only function the UI layer calls. */
export const loadLedger = (
  rawEntries: unknown,
  rawPrices: unknown,
  asOf: Date,
): Effect.Effect<Ledger, ScoringError | DecisionDensityExceeded | ParseResult.ParseError> =>
  Effect.gen(function* () {
    const entries = yield* decodeEntries(rawEntries);
    const prices = yield* decodePrices(rawPrices);
    return yield* buildLedger(entries, prices, asOf);
  });
