---
name: butterfly-ratatui-tui
description: Butterfly's inline TUI runs on ratatui as of 2026-09-17; the alternate screen for a session is still off-limits without sign-off
metadata:
  type: project
---

On **2026-09-17** the user asked for and approved a full migration of `products/butterfly/cli`'s
inline TUI from hand-written crossterm escape sequences to **ratatui 0.30 over crossterm 0.29**.
Ratatui is a dependency, not a vendored fork — no Codex/OpenCode source was copied, so no new
licence obligations.

**Why:** the user evaluated porting OpenAI Codex's TUI wholesale (243k prod lines, 51 internal
crates) and chose the "adopt ratatui, keep our own viewport policy" option instead.

**A resize reflows by owning the scrollback**: `Term::reset_for_reflow` purges screen AND scrollback
(`CSI 2J` + `CSI 3J`) and `settle_resize` replays the transcript at the new width, capped. This is
Codex's `app::resize_reflow` + `transcript_reflow.rs`. Three designs that do NOT work and were each
shipped and rejected: leaving committed rows at their old wrapping (reads as "the UI doesn't
resize"), and two variants of projecting over the terminal's own reflow (entry-counted window, DSR
cursor probe) — both duplicate, because the reflow has already moved rows into scrollback.

The renderer otherwise follows **Codex's history model**: committed output is written into the terminal's own
scrollback exactly once (pre-wrapped hard-terminated rows inside a scroll region above the
composer) and never redrawn; the composer is the only region the renderer owns. Screen-projection
renderers were tried twice and both duplicated content, because the terminal's own reflow pushes
rows into scrollback that a projection then redraws. **Do not reintroduce a projection.**

**No history repaint at all (2026-09-21).** The 2026-09-17 `fill_history` design — repaint the
region above the composer from the transcript on every composer height change, bottom-aligned on
the region it was about to leave — looked right and broke the one invariant that matters:
scrollback + screen = transcript, each row exactly once. Growth clipped rows without scrolling them,
the next commit scrolled the region, and the shrink repainted rows already in scrollback. A
streaming table (live preview changes the composer height on every row) lost rows and duplicated
others. `Term` now keeps a content band and moves it like a shell: commits fill blank rows first,
growth scrolls only what it must (DL blank rows above the band first), shrink pulls the band down
with IL. **Never redraw a row that may already be in scrollback**; the resize reflow is the only
redraw and it purges scrollback first.

Also learned the same day: streaming commits one line at a time, so a lone blank line is always
"trailing" — dropping trailing blanks silently deleted every paragraph break the model wrote. It is
deferred to the next commit now (`pending_blank`).

The user asked for this explicitly after two of my own designs failed: when they say "make it like
Codex", read Codex's source and copy the mechanism rather than inventing an equivalent.

**How to apply:** two constraints from before the migration still stand and still need the user's
explicit sign-off to change — taking over the **alternate screen** for a session (the Ctrl+T
transcript overlay is the one sanctioned use), and anything that alters **cached prefix bytes**
(invariant 9; TUI work stays in `crates/cli`, never `crates/core`). Butterfly deliberately does NOT
use ratatui's `Viewport::Inline`: `tui/term.rs` owns the viewport so a settled resize can
re-project the visible screen from the transcript. See [[terminal-ambiguous-width]] and
[[butterfly-product]]; the full design record is in `products/butterfly/cli/AGENTS.md`.
