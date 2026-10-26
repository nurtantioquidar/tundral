---
name: herdr-isolated-test-session
description: "How to dogfood the Butterfly TUI in a herdr session of my own (not the user's `default`) from a non-TTY agent shell"
metadata: 
  node_type: memory
  type: reference
  originSessionId: cba3b530-ab1a-4d70-b738-4b5e30d1abb6
  modified: 2026-09-19T15:32:58.572Z
---

Butterfly's AGENTS.md requires TUI testing in a herdr session of your own, never the running
`default` one. The herdr client needs a TTY, so from an agent shell: spawn it inside a Python
`pty.fork()` at a chosen size (`TIOCSWINSZ`, e.g. 160x45 gives a ~132-column pane), keep the
wrapper draining output in the background, then drive the session with
`HERDR_SESSION=<name> herdr pane {read,send-text,send-keys} w1:p1`.

- `herdr pane read w1:p1 --source visible|recent --format text|ansi` dumps the screen; ansi
  shows dim/bold, which is how to tell hidden-reasoning leaks from answer text.
- `send-keys` knows `enter`, `esc`, `down`, `ctrl+u`, `ctrl+t`, `q` but NOT Shift+Tab; send it
  raw: `herdr pane send-text w1:p1 $'\e[Z'`.
- Killing and relaunching the pty wrapper at a new size resizes the pane and is a real
  SIGWINCH reflow test (do not use `script` on macOS: it drops resizes).
- Stop a session with `HERDR_SESSION=<name> herdr server stop`, then delete its dir under
  `~/.config/herdr/sessions/`. Never run `herdr --session --help`: it creates a session
  literally named `--help`.
- A cheap live turn (DeepSeek flash, sub-cent) is the only way to see the activity row, tool
  rows, approval card and turn receipt; the approval card needs a command off the allowlist,
  e.g. `python3 -c 'print(42)'`.

Related: [[butterfly-ratatui-tui]], [[terminal-ambiguous-width]].
