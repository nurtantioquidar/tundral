# tundral.ai design

> The studio was renamed from Monet to Tundral on 2026-10-23. The concept below (Monet's pond at
> Giverny) predates the rename and stays until the hub is redesigned for Tundral; Dijkstral has no
> card or icon yet.

The hub's design law. Every colour, size, and font in `apps/hub` traces back to this file; change it
first, then the code. Product surfaces (Inver) keep `@tundral/ui` and are not governed by this file.
Read `design-notes.md` for what was tried and rejected.

## Brief

monet.sh is the front door of Monet, a small studio named for the painter that builds instruments
for working with AI (Butterfly, Cocoon, Nimble, Inver, Cicada) and, next, art made with AI. Readers
are startup-program reviewers and developers deciding whether the studio is real. The page has one
job: make them write to tio@monet.sh.

## World

Modern pixel art, after a reference the owner supplied ("Quiet Place": pale sage sky, outlined
pixel pines, soft pixel clouds, a pond strip, monospace type, coral pill buttons). The scene is
Monet's pond at Giverny: pines on the outer banks, lily pads, his green footbridge over the gap, and
a small robot at an easel painting a tiny *Impression, Sunrise*. The robot is the AI idea in one
picture — a model holding the brush — and the corner credit says the scene was drawn in code with
Claude.

## Tokens

All in `src/app/tokens.css`. Page colours (`--color-*`):

| Token | Hex | Role |
|---|---|---|
| `--color-sky` | `#D4E3E1` | Hero sky |
| `--color-paper` | `#EEF3F1` | Page below the hero |
| `--color-ink` | `#1D2B2B` | Text |
| `--color-ink-soft` | `#4B5C5B` | Secondary text |
| `--color-cta` | `#C64E3E` | Primary pill (white text, AA) |
| `--color-chip` | `#F5F7F4` | Secondary pill |

The pixel palette (`--px-*`) is shared by the scene and the icons: a dark maroon outline
(`#4A2E2E`), three pine greens, cloud white and shade, bank and water tones, lily pad and pink,
the bridge green, robot greys, coral, and one light/mid/dark trio per icon tone. Spacing is a 4px
scale. Radius: pills only. No shadows.

## Type

JetBrains Mono only: 700 for the title, section titles, tool names, and buttons; 400 for
everything else. Title `--text-title` (two lines that never wrap), h2 `--text-h2`, body 0.9375rem.

## Signature

The pond scene, pinned to the bottom of the hero behind the centred copy. It is drawn at a low
logical resolution and scaled by whole pixels (4× on wide screens, 3× otherwise) with
nearest-neighbour sampling. Trees stay on the outer thirds; the centre is open sky over the bridge
and the painter. Only two things move: the water shimmer and the robot's brush arm, on a slow
two-frame loop; with `prefers-reduced-motion`, both are still. The scene publishes
`--scene-clear` so the copy always sits above the bridge and painter.

## Icons

Each tool's icon is described as simple shapes, rasterised at build time onto a 24-pixel grid, and
shaded like the scene: maroon outline, light top-left, dark bottom-right. One region per icon is
coral and always means "the thing this tool decides or protects": Butterfly's cache hit, Cocoon's
key, Nimble's chosen route, the request passing Inver's bridge arch, Cicada's remaining balance.

## Logo

A pink water lily opening on a green pad, after Monet's *Water Lilies*, with a coral heart. It is
pixel art from the same rasteriser as the icons (`LOGOS.lily` in `src/app/pixel-art.ts`), drawn on
a 16-pixel grid plus outline (18 art pixels) and shown at exact multiples: 36px in the nav, and
`src/app/icon.svg` as the favicon, generated from the same artwork.

## Copy rules

Sentence case. Plain verbs. Buttons say what they do. No hype adjectives, no invented metrics, no
"AI-powered". Product stages come from each product's own README.

## Banned

Everything in the frontend-taste banned list, except where the owner's reference asks for it
(coral pill buttons, centred feature columns). Also, from the owner's reactions: amber or yellow
accents, the terminal-dark look, Hallmark-style stamps, an email address in the top bar, a
footer, and a scene credit line.
