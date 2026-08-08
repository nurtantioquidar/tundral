# Monet UI Agent Guide

## Purpose

`@monet/ui` is the Monet design-token layer. It exists because three frontends — `apps/hub`,
`products/inver/web`, and `products/regretz/web` — must not each carry their own copy of the brand.

Follow the repository-root `AGENTS.md` first. `CLAUDE.md` must remain a relative symlink to this
file.

## Ownership boundary

This package owns:

- `tokens.css`: the semantic token layer for colour, type, scale, space, motion, geometry, and
  z-order, declared on `:root`.

This package does not own:

- React components, layout classes, or component-level styling;
- application CSS resets or base element styles;
- anything runtime — this package ships CSS only, and has no JavaScript entry point.

It is a token layer, not a component library. A component moves here only when at least two
frontends genuinely render it, and that promotion requires adding a real JavaScript entry point and
revisiting this file.

## Change rules

- Tokens are consumed by three independently deployed frontends. A rename or removal is a breaking
  change across all of them; verify every consumer in the same change.
- Prefer adding a token over redefining an existing one. Never change a token's meaning in place.
- Keep every value semantic. `--color-accent` is correct; `--color-amber` is not, because it
  prevents a theme rotation.
- Contrast pairings are contractual. The header comment records the verified WCAG AA ratios; if you
  change a colour token, re-verify the affected pairings and update that record.
- The accent and the error colour must stay clearly separable. A warm accent makes them easy to
  confuse, which would make destructive controls read as brand.

## Consumption

Frontends import the layer at the top of their own global stylesheet:

```css
@import "@monet/ui/tokens.css";
```

Consumers must not copy token values into their own CSS, and must not hardcode raw colour, spacing,
radius, or duration values alongside them.

## Verification

This package has no tests of its own; it is verified through its consumers. After any token change:

```bash
bun run check
```

Then build and visually inspect every frontend that renders the changed token, at mobile and
desktop widths, in the states the token affects.
