/* The transport shape of the ledger. Everything that enters the scoring layer is decoded through
 * these schemas first, so a malformed date or an out-of-order series is a decode failure at the
 * boundary rather than a wrong number three functions deeper. */

import { Schema } from "effect";

/** A calendar day, `YYYY-MM-DD`, decoded at UTC midnight. The pattern is not decoration: `new Date`
 * reads `2025-2-4` in local time and `2025-02-04` in UTC, and a ledger that shifts a decision by a
 * day depending on the reader's timezone is not auditable. */
const IsoDay = Schema.String.pipe(
  Schema.pattern(/^\d{4}-\d{2}-\d{2}$/, { message: () => "expected a calendar day as YYYY-MM-DD" }),
  Schema.compose(Schema.Date),
);

/** Closes are integers. IDX quotes whole rupiah, and IHSG is carried as index points × 100 so the
 * benchmark never depends on a float either. Returns are ratios and are genuinely fractional; that
 * is arithmetic on dimensionless numbers, not on money. */
const Close = Schema.Number.pipe(Schema.int(), Schema.positive());

export const PricePoint = Schema.Struct({
  date: IsoDay,
  close: Close,
});

export const PriceSeries = Schema.Struct({
  ticker: Schema.String.pipe(Schema.nonEmptyString()),
  points: Schema.Array(PricePoint).pipe(
    Schema.minItems(1),
    Schema.filter(
      (points) => points.every((p, i) => i === 0 || p.date.getTime() > points[i - 1]!.date.getTime()),
      { message: () => "price points must be strictly ascending by date" },
    ),
  ),
});

export const Entry = Schema.Struct({
  /** Serial number. The stamp carries it, so it is part of the record, not a database detail. */
  id: Schema.String.pipe(Schema.nonEmptyString()),
  ticker: Schema.String.pipe(Schema.nonEmptyString()),
  /** The day the pass was made. */
  decidedOn: IsoDay,
  /** When the row was written. See the backdating filter below. */
  createdAt: IsoDay,
  /** The one-line claim, set in italic. */
  thesis: Schema.String.pipe(Schema.nonEmptyString()),
  /** The reasoning, in the author's own words. This is the product; everything else is a figure. */
  note: Schema.String.pipe(Schema.nonEmptyString()),
  /** `null` while the decision is open. Open rows cannot be hidden or deleted, only resolved. */
  resolvedOn: Schema.NullOr(IsoDay),
  /** The close on `decidedOn`, restated here so a row is readable without the price book. */
  priceAtDecision: Close,
}).pipe(
  /* Backdating is blocked. A decision date that precedes the row's creation timestamp is
   * retroactive genius, which is the exact failure mode this tool exists to prevent. */
  Schema.filter((entry) => entry.decidedOn.getTime() >= entry.createdAt.getTime(), {
    message: () => "decision date cannot precede the entry's creation timestamp",
  }),
  /* A resolution cannot land before the decision it resolves. */
  Schema.filter((entry) => entry.resolvedOn === null || entry.resolvedOn.getTime() >= entry.decidedOn.getTime(), {
    message: () => "resolution date cannot precede the decision date",
  }),
);

export const EntryBook = Schema.Array(Entry);
export const PriceBook = Schema.Array(PriceSeries);

export type Entry = typeof Entry.Type;
export type PricePoint = typeof PricePoint.Type;
export type PriceSeries = typeof PriceSeries.Type;

export const decodeEntries = Schema.decodeUnknown(EntryBook);
export const decodePrices = Schema.decodeUnknown(PriceBook);
