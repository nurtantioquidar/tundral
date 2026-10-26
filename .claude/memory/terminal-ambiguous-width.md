---
name: terminal-ambiguous-width
description: User runs herdr 0.8.0 (not zellij); ambiguous glyphs render 2 columns while unicode_width, pyte and ratatui all count 1
metadata:
  type: project
---

The user's terminal workspace manager is **herdr 0.8.0** (`~/.nix-profile/bin/herdr`), NOT zellij —
corrected 2026-09-17 when they said so directly. zellij is installed but is not what they use, so
never verify terminal behaviour there and call it done.
Ghostty renders East Asian **Ambiguous** characters (`·` `—` `…` `●` `∴` `•` `─` `↑↓→`) as **2
columns**. `unicode_width::width()`, pyte, **and ratatui** (`Line::width`, `Buffer` cell placement)
all count them as **1**.

**Why:** on 2026-09-16 this cost several rounds of "still broken". Every pyte-based replay harness
reported zero overflowing rows while the user's screen sheared every line mid-word, because the
harness and the product shared the same wrong assumption. Measured: 221-246 overflowing rows under
wide-ambiguous accounting vs 0 under narrow.

**How to apply:** verify in herdr — `herdr pane read --source recent|visible` dumps scrollback and
viewport, `pane send-text`/`send-keys` drive it, and an outer PTY resize makes it reflow; always in
a session of your own, never the running `default` one. Measure rows under BOTH width
conventions — a pyte/xterm-only check proves nothing about what they see. Since the 2026-09-17
ratatui migration this also means layout must never measure with ratatui's own `Line::width`;
Butterfly budgets with `theme::line_cols` / `cell_width` (`width_cjk`). See [[butterfly-product]]
and [[butterfly-ratatui-tui]]; the glyph rule itself lives in `products/butterfly/cli/AGENTS.md`.
