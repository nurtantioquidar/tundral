---
name: codex-is-the-tui-reference
description: The user judges Butterfly's TUI against Codex side by side and wants a design proposal in chat before implementation
metadata:
  type: feedback
---

For Butterfly TUI work the user's bar is "as tidy as Codex", judged on the real thing: run the
same prompts in `codex` and in `butterfly`, same repo, same herdr pane, and compare the captures
line by line (2026-09-21: `explain me @AGENTS.md eli5`, then `give me table`). They rejected the
right-aligned tool receipts and the light-grid table on sight and approved the Codex shapes.

**Why:** they have Codex open all day and notice every deviation; "make it better" without a
reference produced designs they then reversed. The side-by-side also surfaced two real bugs
(scrollback rows lost/duplicated, paragraph breaks dropped) that no unit test had caught.

**How to apply:** when they say "propose first", write the design in chat with before/after
mockups and alternatives, and stop; implement only after they pick. Before claiming a rendering
fix works, capture Butterfly's full scrollback (`herdr pane read --source recent --lines 600`) and
check it against the model's raw reply from the session log. See [[herdr-isolated-test-session]]
and [[butterfly-ratatui-tui]].
