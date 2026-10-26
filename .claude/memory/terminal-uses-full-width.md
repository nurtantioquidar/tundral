---
name: terminal-uses-full-width
description: The user rejects text-measure caps in terminal UIs — prose must fill the width they sized the window to
metadata:
  type: feedback
---

On **2026-09-17** I capped Butterfly's prose at 96 columns for readability. The user rejected it on
sight, from a screenshot: *"nothing change so much space on the right (black)"*. The cap was
removed the same day.

**Why:** the typographic argument for a short measure — the eye loses its place on the return
sweep, which is why books stop well short of the page — does not transfer to a terminal. A book's
measure is chosen by the typesetter because the reader cannot change it. A terminal's width is
chosen by the reader, every time they size the window, so capping it overrides a decision they have
already made and the unused space is visible on every line. Codex does not cap either: its history
cells wrap to the width they are given.

**How to apply:** render to the full width the surface has. If long measures genuinely need
addressing, do it with vertical rhythm, indentation and grid alignment — never by leaving the
window empty. The same instinct applies to any "tasteful" constraint that shows up as unused
screen. `entries_rewrap_at_the_width_they_are_rendered_at` now asserts prose keeps widening past 96
columns so the cap cannot return by accident. See [[butterfly-ratatui-tui]].
