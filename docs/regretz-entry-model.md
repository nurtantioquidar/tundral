# Regretz entry model

Status: decided. Supersedes the placeholder framing that described Regretz as a record of things
that were built and did not work.

## What a Regretz entry is

An investment decision **not** taken — a pass on a listed Indonesian instrument — scored
counterfactually against the IHSG over the window between the decision and its resolution.

This is a narrower and more falsifiable product than the earlier "anti-portfolio of things that were
built" framing. A build post-mortem has no benchmark and no clock; a pass on a ticker has both, so
it can be scored rather than merely narrated.

## Fields

| Field | Meaning |
| --- | --- |
| `id` | Serial number. Appears on the resolution stamp, so it is part of the record. |
| `ticker` | IDX ticker the decision concerns. |
| `decidedOn` | The day the pass was made. |
| `createdAt` | When the row was written. Backdating guard, see below. |
| `thesis` | One-line claim, set in italic. |
| `note` | The reasoning in the author's own words. The product. |
| `resolvedOn` | `null` while open. Open rows are visible and uncounted. |
| `priceAtDecision` | Close on `decidedOn`, restated so a row reads without the price book. |

## Why the counterfactual is benchmark-relative

Absolute return answers the wrong question. Passing on a stock that rose 40 % in a market that rose
45 % was correct, and a ledger that shows the 40 % alone converts hindsight into self-reproach
without evidence. Alpha forgone is the only figure the register displays.

The output unit is percentage points of IHSG-relative return. Positive means the pass forwent
return — regret. Exactly zero means the market was matched, which counts as a correct pass: matching
the index by doing nothing is the null result, not an achievement.

## Integrity rules

These are properties of the data, enforced at decode time or in the scoring layer, never in the UI.

- **Backdating.** `decidedOn` may not precede `createdAt`. Retroactive genius is the failure mode
  the product exists to prevent, so it is rejected as a decode error rather than flagged.
- **Density.** More than `DECISIONS_PER_WEEK_CAP` decisions in one ISO week refuses to load. Logging
  passes faster than they can be researched is how the ledger gets farmed for one vindication.
- **Open rows are permanent.** An open decision cannot be hidden or deleted, only resolved. A ledger
  you can edit is not a ledger.
- **Failures are values.** Missing prices, benchmark gaps and out-of-range dates are typed errors.
  Nothing falls back to zero, because a silent zero reads as "no alpha forgone" when it means
  "unknown".

## Sourcing

Steps 1 and 2 of the build order run entirely on local `entries.json` and `prices.json`. Prices are
matched on exact calendar days: a date inside a series' range with no close is a gap, a date outside
it is out of range, and the two are distinct errors. Nearest-prior interpolation was rejected — it
would silently invent a price on a day the exchange was closed, and the ledger's claim to being
auditable rests on never doing that.

Live IDX price fetching is step 3 and is not built.

## Open questions

- Whether the footer's total should sum percentage points across positions at all. It is what the
  register shows today, labelled as such, but summing pp across decisions of different sizes and
  windows is arithmetically loose and will not survive position sizing.
- What resolves an entry. Today resolution is a date in the seed file; there is no rule for when a
  pass is "over", and a decision left open indefinitely never scores.
