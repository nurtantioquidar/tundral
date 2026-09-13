/* The rules that only make sense across the whole ledger: backdating, decision density, and the
 * fact that an open row is visible but contributes nothing to the total. */

import { Effect, Either } from "effect";
import { describe, expect, it } from "vitest";
import { buildLedger, checkDecisionDensity, loadLedger } from "./ledger";
import { decodeEntries } from "./schema";
import type { Entry, PriceSeries } from "./schema";

const day = (iso: string): Date => new Date(`${iso}T00:00:00.000Z`);
const AS_OF = day("2026-08-08");

const rawEntry = (over: Record<string, unknown> = {}) => ({
  id: "R-001",
  ticker: "BREN",
  decidedOn: "2025-02-04",
  createdAt: "2025-02-04",
  thesis: "Valuasi tidak masuk akal di harga ini.",
  note: "Kapitalisasi pasarnya sudah melampaui gabungan tiga emiten energi terbesar.",
  resolvedOn: "2025-08-01",
  priceAtDecision: 7_500,
  ...over,
});

const rawPrices = [
  {
    ticker: "IHSG",
    points: [
      { date: "2025-02-04", close: 695_000 },
      { date: "2025-08-01", close: 718_000 },
    ],
  },
  {
    ticker: "BREN",
    points: [
      { date: "2025-02-04", close: 7_500 },
      { date: "2025-08-01", close: 5_200 },
    ],
  },
];

const decode = (raw: unknown) => Effect.runSync(Effect.either(decodeEntries(raw)));

const entryAt = (iso: string, id: string): Entry => ({
  id,
  ticker: "BBCA",
  decidedOn: day(iso),
  createdAt: day(iso),
  thesis: "t",
  note: "n",
  resolvedOn: null,
  priceAtDecision: 9_800,
});

describe("backdating", () => {
  it("rejects a decision dated before the row was created", () => {
    const result = decode([rawEntry({ decidedOn: "2025-01-02", createdAt: "2025-02-04" })]);
    expect(Either.isLeft(result)).toBe(true);
  });

  it("accepts a decision logged on the day it was made", () => {
    const result = decode([rawEntry()]);
    expect(Either.isRight(result)).toBe(true);
  });

  it("rejects a resolution dated before its own decision", () => {
    const result = decode([rawEntry({ resolvedOn: "2024-12-31" })]);
    expect(Either.isLeft(result)).toBe(true);
  });
});

describe("decision density", () => {
  it("allows a week at the cap", () => {
    // 2026-02-02 is a Monday; these three land in the same ISO week.
    const entries = [entryAt("2026-02-02", "R-1"), entryAt("2026-02-04", "R-2"), entryAt("2026-02-06", "R-3")];
    expect(Either.isRight(Effect.runSync(Effect.either(checkDecisionDensity(entries))))).toBe(true);
  });

  it("refuses to load a week over the cap", () => {
    const entries = [
      entryAt("2026-02-02", "R-1"),
      entryAt("2026-02-03", "R-2"),
      entryAt("2026-02-04", "R-3"),
      entryAt("2026-02-05", "R-4"),
    ];
    const result = Effect.runSync(Effect.either(checkDecisionDensity(entries)));
    if (Either.isRight(result)) throw new Error("expected the density cap to trip");
    expect(result.left._tag).toBe("DecisionDensityExceeded");
    expect(result.left.count).toBe(4);
  });

  it("does not merge adjacent weeks", () => {
    // Friday and the following Monday are different ISO weeks.
    const entries = [
      entryAt("2026-02-04", "R-1"),
      entryAt("2026-02-05", "R-2"),
      entryAt("2026-02-06", "R-3"),
      entryAt("2026-02-09", "R-4"),
    ];
    expect(Either.isRight(Effect.runSync(Effect.either(checkDecisionDensity(entries))))).toBe(true);
  });
});

describe("buildLedger", () => {
  const benchmarkOnly: ReadonlyArray<PriceSeries> = [
    { ticker: "IHSG", points: [{ date: day("2026-05-12"), close: 821_000 }] },
  ];

  it("keeps an open decision visible and out of the total", () => {
    const open = entryAt("2026-05-12", "R-9");
    const ledger = Effect.runSync(buildLedger([open], benchmarkOnly, AS_OF));

    expect(ledger.rows).toHaveLength(1);
    expect(ledger.rows[0]?._tag).toBe("Open");
    expect(ledger.openCount).toBe(1);
    expect(ledger.totalAlphaPp).toBe(0);
  });

  it("counts days held for an open decision up to the as-of date", () => {
    const ledger = Effect.runSync(buildLedger([entryAt("2026-05-12", "R-9")], benchmarkOnly, AS_OF));
    const row = ledger.rows[0];
    if (row?._tag !== "Open") throw new Error("expected an open row");
    expect(row.daysHeld).toBe(88);
  });

  it("totals alpha across resolved rows only", () => {
    const entries = Effect.runSync(decodeEntries([rawEntry(), rawEntry({ id: "R-002", resolvedOn: null })]));
    const ledger = Effect.runSync(buildLedger(entries, decodedPrices(), AS_OF));

    expect(ledger.resolvedCount).toBe(1);
    expect(ledger.openCount).toBe(1);
    expect(ledger.totalAlphaPp).toBeCloseTo(-33.976, 3);
  });
});

describe("loadLedger", () => {
  it("decodes and scores the seed shape end to end", () => {
    const result = Effect.runSync(Effect.either(loadLedger([rawEntry()], rawPrices, AS_OF)));
    if (Either.isLeft(result)) throw new Error(`expected a ledger, got ${result.left._tag}`);
    expect(result.right.resolvedCount).toBe(1);
    expect(result.right.totalAlphaPp).toBeCloseTo(-33.976, 3);
  });

  it("surfaces a missing benchmark rather than scoring against nothing", () => {
    const withoutIhsg = rawPrices.filter((series) => series.ticker !== "IHSG");
    const result = Effect.runSync(Effect.either(loadLedger([rawEntry()], withoutIhsg, AS_OF)));
    if (Either.isRight(result)) throw new Error("expected a failure");
    expect(result.left._tag).toBe("BenchmarkGap");
  });
});

function decodedPrices(): ReadonlyArray<PriceSeries> {
  return [
    {
      ticker: "IHSG",
      points: [
        { date: day("2025-02-04"), close: 695_000 },
        { date: day("2025-08-01"), close: 718_000 },
      ],
    },
    {
      ticker: "BREN",
      points: [
        { date: day("2025-02-04"), close: 7_500 },
        { date: day("2025-08-01"), close: 5_200 },
      ],
    },
  ];
}
