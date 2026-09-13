/* The four cases the scoring function exists to get right, plus the failures it must not swallow.
 * Every assertion goes through the Either so a test can never pass by accident on a thrown error. */

import { Effect, Either } from "effect";
import { describe, expect, it } from "vitest";
import type { ScoringError } from "./errors";
import type { AlphaForgone } from "./score";
import { scoreEntry } from "./score";
import type { Entry, PriceSeries } from "./schema";

const day = (iso: string): Date => new Date(`${iso}T00:00:00.000Z`);

const entry = (over: Partial<Entry> = {}): Entry => ({
  id: "R-001",
  ticker: "BREN",
  decidedOn: day("2025-02-04"),
  createdAt: day("2025-02-04"),
  thesis: "Valuasi tidak masuk akal di harga ini.",
  note: "Market cap-nya sudah melampaui gabungan tiga emiten energi terbesar.",
  resolvedOn: day("2025-08-01"),
  priceAtDecision: 7_500,
  ...over,
});

const series = (ticker: string, points: ReadonlyArray<[string, number]>): PriceSeries => ({
  ticker,
  points: points.map(([iso, close]) => ({ date: day(iso), close })),
});

const run = (e: Effect.Effect<AlphaForgone, ScoringError>) => Effect.runSync(Effect.either(e));

const expectRight = (result: Either.Either<AlphaForgone, ScoringError>): AlphaForgone => {
  if (Either.isLeft(result)) throw new Error(`expected success, got ${result.left._tag}`);
  return result.right;
};

const expectLeftTag = (result: Either.Either<AlphaForgone, ScoringError>, tag: ScoringError["_tag"]): void => {
  if (Either.isRight(result)) throw new Error(`expected ${tag}, got a score`);
  expect(result.left._tag).toBe(tag);
};

describe("scoreEntry", () => {
  it("scores a correct pass when the instrument underperformed IHSG", () => {
    // BREN fell 30 % while IHSG rose 3.3 %. Not buying was right.
    const instrument = series("BREN", [
      ["2025-02-04", 7_500],
      ["2025-08-01", 5_200],
    ]);
    const benchmark = series("IHSG", [
      ["2025-02-04", 695_000],
      ["2025-08-01", 718_000],
    ]);

    const alpha = expectRight(run(scoreEntry(entry(), instrument, benchmark)));

    expect(alpha.alphaPp).toBeCloseTo(-33.976, 3);
    expect(alpha.isRegret).toBe(false);
    expect(alpha.daysHeld).toBe(178);
  });

  it("scores a regret when the instrument outperformed IHSG", () => {
    // PTRO nearly tripled against a market that gained 24.5 %.
    const instrument = series("PTRO", [
      ["2025-03-11", 2_150],
      ["2026-01-15", 5_900],
    ]);
    const benchmark = series("IHSG", [
      ["2025-03-11", 652_000],
      ["2026-01-15", 812_000],
    ]);
    const passed = entry({
      ticker: "PTRO",
      decidedOn: day("2025-03-11"),
      createdAt: day("2025-03-11"),
      resolvedOn: day("2026-01-15"),
      priceAtDecision: 2_150,
    });

    const alpha = expectRight(run(scoreEntry(passed, instrument, benchmark)));

    expect(alpha.alphaPp).toBeCloseTo(149.879, 3);
    expect(alpha.isRegret).toBe(true);
    expect(alpha.instrumentReturnPct).toBeCloseTo(174.42, 2);
    expect(alpha.benchmarkReturnPct).toBeCloseTo(24.54, 2);
  });

  it("beating the market in absolute terms is still a correct pass when IHSG beat it by more", () => {
    // The whole thesis of the product: +40 % into a +45 % market is not a win.
    const instrument = series("ICBP", [
      ["2025-02-04", 10_000],
      ["2025-08-01", 14_000],
    ]);
    const benchmark = series("IHSG", [
      ["2025-02-04", 100_000],
      ["2025-08-01", 145_000],
    ]);
    const passed = entry({ ticker: "ICBP", priceAtDecision: 10_000 });

    const alpha = expectRight(run(scoreEntry(passed, instrument, benchmark)));

    expect(alpha.instrumentReturnPct).toBeCloseTo(40, 6);
    expect(alpha.alphaPp).toBeCloseTo(-5, 6);
    expect(alpha.isRegret).toBe(false);
  });

  it("fails with UnresolvedEntry rather than reporting zero alpha", () => {
    const instrument = series("ARTO", [["2026-05-12", 2_400]]);
    const benchmark = series("IHSG", [["2026-05-12", 821_000]]);
    const open = entry({
      ticker: "ARTO",
      decidedOn: day("2026-05-12"),
      createdAt: day("2026-05-12"),
      resolvedOn: null,
      priceAtDecision: 2_400,
    });

    expectLeftTag(run(scoreEntry(open, instrument, benchmark)), "UnresolvedEntry");
  });

  it("fails with BenchmarkGap when IHSG has no close on a needed day", () => {
    const instrument = series("ANTM", [
      ["2025-06-10", 1_610],
      ["2026-05-12", 3_980],
    ]);
    // The range covers both days; the resolution day itself is missing.
    const benchmark = series("IHSG", [
      ["2025-06-10", 639_000],
      ["2026-06-22", 834_000],
    ]);
    const passed = entry({
      ticker: "ANTM",
      decidedOn: day("2025-06-10"),
      createdAt: day("2025-06-10"),
      resolvedOn: day("2026-05-12"),
      priceAtDecision: 1_610,
    });

    expectLeftTag(run(scoreEntry(passed, instrument, benchmark)), "BenchmarkGap");
  });

  it("fails with MissingPriceData when the instrument has a hole inside its range", () => {
    const instrument = series("MDKA", [
      ["2025-09-15", 1_885],
      ["2026-06-22", 2_140],
    ]);
    const benchmark = series("IHSG", [
      ["2025-09-15", 762_000],
      ["2026-01-15", 812_000],
      ["2026-06-22", 834_000],
    ]);
    const passed = entry({
      ticker: "MDKA",
      decidedOn: day("2025-09-15"),
      createdAt: day("2025-09-15"),
      resolvedOn: day("2026-01-15"),
      priceAtDecision: 1_885,
    });

    expectLeftTag(run(scoreEntry(passed, instrument, benchmark)), "MissingPriceData");
  });

  it("fails with DateOutsideSeries when the decision predates the series", () => {
    const instrument = series("GOTO", [
      ["2025-06-10", 62],
      ["2026-01-15", 71],
    ]);
    const benchmark = series("IHSG", [
      ["2025-01-06", 708_000],
      ["2026-01-15", 812_000],
    ]);
    const passed = entry({
      ticker: "GOTO",
      decidedOn: day("2025-01-06"),
      createdAt: day("2025-01-06"),
      resolvedOn: day("2026-01-15"),
      priceAtDecision: 62,
    });

    expectLeftTag(run(scoreEntry(passed, instrument, benchmark)), "DateOutsideSeries");
  });

  it("treats matching the index exactly as a correct pass, not a regret", () => {
    const instrument = series("UNVR", [
      ["2025-02-04", 1_000],
      ["2025-08-01", 1_100],
    ]);
    const benchmark = series("IHSG", [
      ["2025-02-04", 100_000],
      ["2025-08-01", 110_000],
    ]);

    const alpha = expectRight(run(scoreEntry(entry({ ticker: "UNVR" }), instrument, benchmark)));

    expect(alpha.alphaPp).toBeCloseTo(0, 9);
    expect(alpha.isRegret).toBe(false);
  });
});
