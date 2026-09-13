# Regretz

A ledger of investment decisions not taken, scored counterfactually against the IHSG.

Not a portfolio app. Not a dashboard. A dated, auditable register of judgment calls, some of which
indict the person who wrote them. Every design and copy decision follows from that.

Follow the repository-root `AGENTS.md` first, except where this file overrides it — see
**Divergence from the shared token layer** below.

## Non-negotiables

1. Every figure is IHSG-relative. Absolute return is never displayed alone. A stock that went up
   40 % in a market that went up 45 % is a correct pass. If a number on screen isn't
   benchmark-adjusted, it's a bug.
2. Indonesian number and date formatting throughout. `id-ID` locale. Dot for thousands, comma for
   decimal: `+218,4%`, `Rp1.240`, `9,62 pp`. Dates as `04 Feb 26`. This is not cosmetic — it's the
   difference between a tool built for BEI and a US template with tickers swapped in.
3. Restraint is rewarded with silence. A correct pass renders in plain ink, no celebration, no
   green, no badge. Only regret gets the accent colour. The visual language argues that not acting
   is the default and being right about it is unremarkable.
4. The decision text is the product. Each entry carries the user's own prose about why they passed.
   It is the largest, most readable thing in the row. The moment entries compress into a table of
   tickers and percentages, this becomes a portfolio app with a gimmick.

## Tokens

Cool paper, not warm cream. Violet stamp ink, not terracotta.

```text
--paper        #ECEDE8   page
--paper-2      #E3E5DF   sunken rows, table headers
--ink          #16181A   primary text, figures
--ink-2        #5C6169   labels, metadata
--rule         #C9CCC5   hairlines
--stamp        #4B3A7A   regret / forgone alpha / the seal. ONE accent. Nothing else uses it.
```

Dark mode inverts to `#15171A` paper with `#E8E9E4` ink; `--stamp` lightens to `#9A8AD0`.

### Divergence from the shared token layer

This module does not consume `@monet/ui`. The shared layer is a warm terminal-dark system on Geist
with an amber accent; Regretz is cool paper on Newsreader with a violet stamp. It is a replacement,
not an extension, so importing it would also pull in `--radius-card: 8px`, the `--shadow-*` tokens
and three additional accents — all of which this project's ban list forbids.

Root `AGENTS.md` states that every frontend consumes `packages/ui`. This file overrides that for
Regretz under the instruction-hierarchy rule ("the closest `AGENTS.md` adds to or overrides broader
guidance"). Tokens live in `src/styles/tokens.css`.

## Type — two faces, no third

- `Newsreader` — decision prose only. 15/1.45. Real italics for the thesis line.
- `Commit Mono` — every figure, label, date, ticker. `font-variant-numeric: tabular-nums` is
  mandatory on anything numeric; columns must align on the decimal.

Both are SIL OFL and are vendored as woff2 under `public/fonts/`. Never load them from a CDN.

Labels are mono at 11px with `letter-spacing: 0.06em`, lowercase. Not small-caps, not uppercase.
Authored lowercase, never `text-transform`-ed — the label class is shared with tickers and serials,
and `BRPT` is not `brpt`.

Scale: 11 / 12 / 15 / 17 / 22. Five sizes. Nothing between, nothing outside.

Weights: 400 and 500. Nothing heavier exists in this project. Commit Mono ships 400 and 700 only,
so mono runs are always 400; `font-synthesis: none` is set globally so a stray `500` on a mono run
fails visibly instead of being faked into a heavier weight by the browser.

## The signature element

The resolution stamp. When a decision resolves, a violet stamp lands on the entry — slightly
rotated (1.5–2°), carrying the serial number and the IHSG-adjusted figure, with a faint ink-bleed
edge. It animates once, on resolution, ~400ms.

This is the only animation in the application. Every other state change is instant. Spend the entire
motion budget here; a ledger that fades and slides everywhere reads as generated.

The bleed is an SVG `feTurbulence` + `feDisplacementMap` filter, because the obvious ways to soften
an edge — a blur, a shadow, a gradient — are all banned.

The stamp deliberately breaks the numeric grid: it is rotated and padded, so its figure does not
share a decimal axis with the plain figures in the same column. Decimal alignment is a rule about
numeric cells; the stamp is a mark, and reading as a mark is the point.

## Ban list

Enforce these without being asked. If a request conflicts with one, say so before building.

### Shape and depth

- No `border-radius` above 4px. No `rounded-lg`, `rounded-xl`, `rounded-2xl`, `rounded-full`.
- No `box-shadow` anywhere. Not on cards, not on hover, not `shadow-sm`.
- No cards. Hairline rules and a baseline grid. Rows are separated by `border-bottom`, not wrapped
  in containers.
- No gradients. No `bg-gradient-*`, no gradient text, no mesh, no noise texture.
- No glassmorphism, no `backdrop-blur`, no translucent overlay panels.

### Type

- No Inter, Roboto, Open Sans, or `system-ui` for body text.
- No font-weight above 500. No `font-bold`, no `font-semibold`.
- No `text-transform: uppercase` on anything longer than three characters.
- No Title Case. Sentence case everywhere, including labels and buttons.
- No proportional figures in numeric columns.

### Colour

- No green-up / red-down. Gains and losses are not the axis this product measures.
- No second accent. `--stamp` is the only chromatic colour in the build.
- No coloured badge pills for status. Status is typography and position.

### Layout and chrome

- No centred hero with a subtitle beneath it.
- No emoji. No icon-per-row decoration. Icons only where they replace a word.
- No metric-card grid across the top of a view.
- No `01 / 02 / 03` numbered markers unless the content is genuinely sequential.

### Motion and states

- No skeleton shimmer. Loading states are a mono `loading` label in `--ink-2`.
- No spring physics, no bounce, no hover lift, no scale-on-hover.
- No toast that says "successfully". The saved entry appearing is the confirmation.

### Copy

- Errors state what happened and what to do. No apology, no first person, no "Error:" prefix.
- Empty states are an invitation, not a confession. Not "No entries yet."
- Never "simply", "just", "seamless", "unlock", "leverage", "powerful".

## Build order

Do not reorder this. Each step must run before the next begins.

1. Register view rendering entries from a local `entries.json`. No database, no auth, no API.
2. Scoring function: `(entry, priceSeries, ihsgSeries) => alphaForgone`. Pure, unit-tested.
3. IDX price fetching with retries. Plain `fetch` and a `for` loop.
4. Entry creation form.
5. Persistence.
6. Share card.

The scoring function is the product's only real intellectual content. It should exist before
anything is styled beyond the register.

Steps 1 and 2 are built. Steps 3–6 are not; do not assume a form, a database, or a network layer
exists.

## Anti-gaming rules

Enforced in the scoring layer, never in the UI. A rule a component can skip is not a rule.

- **Alpha-only exports.** Share cards may show forgone alpha. Never absolute portfolio value, never
  total capital.
- **Decision-density cap.** `DECISIONS_PER_WEEK_CAP` in `src/scoring/ledger.ts`, currently 3.
  Logging fifty passes to farm one vindication is the obvious exploit. The number is a judgement
  call, not a derived constant — revisit it against real usage rather than treating it as settled.
- **Backdating is blocked.** A decode-time filter on `Entry` rejects any row whose decision date
  precedes its creation timestamp. Retroactive genius is the failure mode this whole tool exists to
  prevent.
- **Unresolved entries stay visible.** Open decisions cannot be hidden or deleted, only resolved.
  They render a mono `unresolved` label in `--ink-2` and contribute nothing to the total — counting
  an open decision as zero alpha would flatter the author.

## Architecture

Effect stays at the boundary. `src/main.tsx` decodes, validates and scores the ledger once, then
hands plain resolved objects to React. No `Effect.gen` inside a component, no Effect types in props.
The UI stays boring and readable.

- `src/scoring/` — schema, typed errors, `scoreEntry`, and the ledger-level rules. Pure; no DOM.
- `src/register/` — the register, its row, and `id-ID` formatting. Feature-local.
- `src/data/` — seed `entries.json` and `prices.json`.

Money is integer rupiah; the IHSG is carried as index points × 100 so the benchmark is an integer
too. Returns are ratios over those integers and are genuinely fractional — that is arithmetic on
dimensionless numbers, not on money, and the root rule against floating-point money does not reach
it. Regretz holds no balances and no ledger of funds.

Scoring failures are values, not exceptions: `MissingPriceData`, `BenchmarkGap`, `DateOutsideSeries`,
`UnresolvedEntry`, `DecisionDensityExceeded`. Nothing in this module throws, returns `null`, or
falls back to zero.

## Language

UI chrome is English; entry prose is Indonesian; all numbers and dates are `id-ID`. The prose is
written in the voice of a retail investor reasoning about their own decisions, and reads as
Indonesian for that reason. If the chrome is ever translated, translate all of it at once — a
half-Indonesian interface is worse than either.

## Instruction files

- `AGENTS.md` is the canonical project instruction file for this module.
- `CLAUDE.md` must remain a relative symlink to `AGENTS.md`.
- Record durable knowledge only; no task history or temporary status.

## Ownership boundary

Regretz is a separate product from Inver. It shares no accounts, sessions, balance, or data with
Inver, and must never call the Inver gateway. If Regretz ever needs an account system, it gets its
own or consumes a future control plane — it does not borrow Inver's.

Content is Regretz's own. Do not import copy, components, or server code from a sibling application.

## Commands

```bash
bun run dev        # Vite dev server on :3002
bun run test       # Vitest
bun run typecheck  # tsc --noEmit
bun run check      # typecheck + test
bun run build      # production bundle
```

`bun run start` is `vite preview`, which is a local static preview and not a production server.
Deploying this module needs a static host or an adapter; the Caddy block for `regretz.monet.sh`
reverse-proxies :3002 and is framework-agnostic.

## Required verification

```bash
bun run check
bun run build
```

Then check the register at 360px and at desktop, in both colour schemes, and confirm the ban list
still holds — the fastest audit is computed styles, not eyeballing:

```js
[...document.querySelectorAll("*")].filter((el) => {
  const c = getComputedStyle(el);
  return c.boxShadow !== "none" || /gradient/.test(c.backgroundImage) || parseInt(c.fontWeight, 10) > 500;
});
```
