# Butterfly CLI Agent Guide

## Purpose

`butterfly` is the Butterfly product: a fast, model-agnostic coding agent CLI in Rust — a chat REPL
that runs tools inside the user's repository against any OpenAI-compatible Chat Completions
endpoint (DeepSeek is the reference direct model; GLM, Luna and Muse Spark are built in; Nimble is one more
provider). Design pillars: speed (Rust, streaming, lean prompts), a cache-first prompt layout
(opencode-class hit rates), two-stage auto-compaction with a typed working state, deterministic
safety (approval-gated diffs, argv-only commands), and deterministic completion (verification and
classified recovery in autonomous modes). `BUTTERFLY_ARCHITECTURE_REVIEW.md` in the product root
records the audit that set this direction.

It was selected over a parallel implementation after a benchmarked comparison; the evidence and
the losing tree's archive location are recorded in `docs/harness-implementation-comparison.md`.

## Ownership boundaries

This module owns its conversation state, tool execution policy, context management, model and
provider descriptors (capabilities, pricing, limits), provider request semantics per dialect,
verification and failure classification, telemetry, and local session persistence under
`~/.local/state/butterfly/sessions/`. It calls providers directly with user-supplied keys (one
environment variable per provider, e.g. `DEEPSEEK_API_KEY`, `ZAI_API_KEY`, `OPENAI_API_KEY`,
`NIMBLE_API_KEY`); it never imports source from another Monet application. Nimble is optional: a
session never needs it, and `products/nimble` never links Butterfly.

## Layout

```text
crates/core   # butterfly-core: UI-free engine
  domain      # messages, 4-counter usage, integer micro-USD money, versioned price cards + tiers
  model       # ModelDescriptor/Capabilities/ProviderDescriptor registry; built-ins + [[models]] config
  provider    # OpenAI-compatible SSE adapter parameterized by Dialect (DeepSeek/GLM/OpenAI/Nimble),
              # OpenAI Responses-API dialect (tools + reasoning, encrypted-reasoning replay);
              # scripted fake for tests
  tools       # registry + security policy (PolicySettings: confined default, sandboxed relaxation),
              # argv-only commands, sandbox-only `shell`
  context     # prompt layout (gated appendices only), anchor+drift token ledger, compaction
  failure     # closed FailureKind vocabulary + rule-based classifiers
  verify      # acceptance/detected/model-declared checks, git-diff check, evidence rendering
  telemetry   # per-session JSONL events + summary; key-based redaction
  session     # deterministic agent loop, run_task (attempt → verify → classify → evidence → retry),
              # approval gating, typed working state, JSONL persistence/resume
crates/cli    # butterfly: inline ratatui TUI (crossterm editor, @-mention fuzzy completion,
              # footer, arrow-key approvals, paste placeholders, markdown render);
              # plain line loop for non-TTY stdin; -p headless path
  tui/theme   # the one palette (ratatui Styles) + width-aware Line primitives;
              # `theme::ansi` is the ONLY place raw escapes are still written
  tui/chrome  # every composed surface as a pure width -> Vec<Line> function
  tui/term    # all terminal I/O: composer viewport (cell-diffed) + write-once
              # scrollback history, on Codex's insert-history model
  tui/ui      # state, projection/layout, input handling
../bench      # fixture benchmark vs external harnesses + full-window compaction exercise
```

## TUI design

The inline TUI is **ratatui over crossterm, and it follows Codex's TUI in both rendering model and
visual design** (read from `codex-rs/tui`, Apache-2.0; mechanism and shapes reimplemented, no code
copied). It is layered: `theme` owns color, glyphs and the width-aware `Line` primitives, `chrome`
composes each surface as a pure function of data and width, `term` owns every byte written to the
terminal, `ui` owns state and input. Adopted 2026-09-17 at the operator's explicit request.

The look, and what each piece corresponds to in Codex: a rounded box card on open
(`╭─╮ │ ╰─╯`, `>_ butterfly`, `model:` / `directory:` / `mode:` rows, sized to content and never
full width); `› ` as the composer caret and the marker on a user turn; `• ` opening every agent
row — answers, tool rows, notices; tool rows reading `• Read    <path>` / `• Running <cmd>`; a
decision rendered as a bold question, the subject on a `$`/`•` row, then numbered choices with `›`
on the selected one; and a dim status line two columns in under the composer reading
`model · Context N% left · <mode> mode`. There is **no rule above the composer** and **no filled
band on a user turn** — Codex separates with air, and both of those were Butterfly's own devices.
Session start is inline: nothing is flooded or erased, so the shell's own output stays above.

**The screen has exactly two parts, and this is the whole design:**

- **The viewport** — the composer block at the bottom. A ratatui `Buffer`, cell-diffed, and the
  ONLY region the renderer ever redraws.
- **History** — everything committed. Written into the terminal's own scrollback exactly once by
  `Term::insert_history`, and never touched again.

Two properties of that write are what make scrollback behave, both Codex's:

1. **Rows are pre-wrapped and hard-terminated.** Every line handed to `insert_history` already fits
   one screen row, and rows are separated by a real `\r\n`. A later narrowing re-wraps each row on
   its own; it cannot glue consecutive rows into one logical paragraph, which is what turned a
   scrolled-back conversation into run-together text.
2. **The write happens inside a scroll region above the viewport** (`CSI 1;top r`), with the cursor
   on that region's last row. Each `\r\n` scrolls only that region, pushing its top row into native
   scrollback and leaving the composer untouched.

**When the composer changes height it repaints the history region, bottom-aligned on the region it
is ABOUT to leave.** The composer grows and shrinks constantly — the activity row arrives, the live
line comes and goes, a dropdown opens — and each change hands rows to or takes them from the region
above. `Term::fill_history` redraws that region in place from the transcript: at a fixed width those
are exactly the rows already there, so nothing scrolls, nothing reaches scrollback twice, and no
hole is left behind. Two rules make it correct and both were learned the hard way:

- It takes the **new** region as an argument. Reading the current `viewport_top` aligns on the old
  one, and the composer then draws over the newest rows of history or leaves a run of blanks where
  they should be.
- It clears the union of the old region and the new one and nothing else clears them again. The
  composer must NOT separately clear the rows it gives back when it shrinks; that erases what the
  repaint just wrote into them.

**A resize REFLOWS, by owning the scrollback.** This is Codex's strategy
(`app::resize_reflow` + `clear_scrollback_and_visible_screen_ansi`) and it is the only one that
actually works. Once rows are written to scrollback the terminal owns them, and a renderer has
three choices: leave them at their old wrapping (which is what "the UI does not resize" looks
like), re-project over the top (which duplicates, because the terminal's own reflow has already
moved some of them into scrollback), or take the whole scrollback over. Butterfly takes it over.
`Term::reset_for_reflow` emits `CSI r` / `CSI 0m` / `CSI H` / `CSI 2J` / **`CSI 3J`** / `CSI H` —
a shell `clear` plus a scrollback purge — and `settle_resize` then replays the transcript at the new
width, capped at `REFLOW_ROW_CAP` rows so a drag does not get slower the longer the session runs.
The in-memory transcript is the source of truth; the terminal's copy is a rendering of it that goes
stale the instant the width changes.

The cost is explicit and is Codex's too: anything on screen before the session started is purged by
the first resize. The session already takes the screen over.

Both earlier designs are recorded in `docs/butterfly-features.md` because they look reasonable and
are not: an entry-counted window and a cursor-position (DSR) probe each tried to keep a projection
and subtract the overlap, and each only reduced the duplication.

The load-bearing composition rule is `render_entry`'s contract: for every `Entry` variant, one
returned `Line` is exactly one screen row AND it fits the width it was rendered at. `insert_history`
depends on it — a row that soft-wraps into two breaks the scroll arithmetic. Keep entries as DATA
(preview + outcome, parsed table, spans), never pre-rendered rows.

Glyph widths are **measured, not assumed**. `theme::CHROME_GLYPHS` lists the glyphs the chrome
draws itself; each was measured in the reference terminal (a 3/4-width run occupies the same
columns as the same run of ASCII, i.e. one column apiece) and `cell_width` returns 1 for them. That
is what lets the box card line up — a border and its content must agree — and it is what Codex and
ratatui assume throughout. Everything else, above all text the model wrote, keeps the conservative
budget: Unicode marks a large set — `—` `●` `∴` `↑↓→` and more — as East Asian
Ambiguous, and a terminal may render them one column or two. **`unicode_width::width()` assumes one, and so do ratatui's `Line::width` and its
`Buffer` cell placement** — so layout must never measure with `Line::width`; use `theme::line_cols`
/ `theme::cell_width`, which budget with `width_cjk`. A row can then come out a column short on a
narrow-ambiguous terminal, which nobody sees, but never overflows, which shears the grid.
Adding a glyph to the chrome means adding it to `CHROME_GLYPHS` **and measuring it in the reference
terminal first**; `glyphs_have_one_unambiguous_width` and `chrome`'s `check` helper then hold the
line. Use `SEP`/`MARK_*`/`BOX_*`, never the literal.

The layout width is `columns - 1` (`term::usable_width`): writing the last column is the one thing
terminals disagree about, and some multiplexers record the two rows as ONE logical line and re-wrap
them together. A pyte-based replay counts ambiguous glyphs as ONE and is structurally blind to that
whole class of bug — it reported zero overflowing rows where a real terminal had 221. Measure rows
under BOTH conventions.

The reference terminal is **herdr 0.8.0** — not zellij. `herdr pane read --source visible|recent`
dumps the viewport and scrollback, `pane send-text`/`send-keys` drive it, and an outer PTY resize
makes it reflow. Always test in a session of your own, never the running `default` one.

To debug any of this, set `BUTTERFLY_TUI_TRACE=<path>`: every byte the TUI writes is tee'd there by
`term::TracedOut`, which wraps the backend's writer. Do NOT reach for `script` — on macOS it does
not propagate window resizes into the wrapped process, so the recording is silent about the one
thing a resize bug needs.

A new surface belongs in `chrome` with a width-budget test, not inline in `ui`; a new color belongs
in `theme` or nowhere; a new escape sequence belongs in `term` or nowhere. Every marker sits in
column 0 and every piece of content in column 2. Moving to the alternate screen for the session (as
opposed to the Ctrl+T overlay, which already uses it) remains a product decision. Visual changes
stay in `crates/cli`, must not alter cached prefix bytes (invariant 9), and must not change what an
approval prompt gates or how a keypress maps to consent (invariants 4-5).

## Commands

```bash
cargo test --workspace --locked                       # offline test suite
cargo fmt --check && cargo clippy --workspace --locked --all-targets -- -D warnings
bun run --filter @monet/butterfly-cli check             # all of the above
bun run dev:butterfly                                 # REPL via the repo root
butterfly models                                      # registry: built-ins + config
BUTTERFLY_LIVE_TEST=1 DEEPSEEK_API_KEY=... cargo test -p butterfly-core --test live_deepseek  # paid smoke test
BUTTERFLY_LIVE_TEST=1 BUTTERFLY_LIVE_MODEL=glm-5.3-flash ZAI_API_KEY=... cargo test -p butterfly-core --test live_deepseek
products/butterfly/bench/harbor/with-keys.sh <cmd>    # keys from 1Password (op) into the env, never printed
```

## Invariants

1. **The frozen transcript**: once a message is sent to the provider, its bytes and position never
   change — except during a compaction event, the only operation allowed to rewrite history.
   Prefix caching on every supported provider is automatic; byte-stability is the only lever.
   Nothing dynamic (timestamps, git status, counters) may enter the system prompt or be
   re-rendered into earlier history slots.
2. Compaction is rare and chunky: elision must clear the `min_savings_fraction` floor, and
   summarization rides the warm cache (current history + one appended instruction). New checkpoints
   supersede old ones; they never stack. The checkpoint's structured half is the typed working
   state (loop-observed files/commands + model-declared plan via `update_plan`), which lives
   outside the transcript and survives elision, compaction, and resume by construction.
3. `reasoning_content` is preserved on assistant tool-call messages sent back to the provider
   according to the model's `ReasoningReplay` capability (DeepSeek: required — absent is
   serialized as null, never omitted). Post-compaction tails start at a user message, keep
   tool-call/tool-result groups atomic, and retain reasoning on kept tool-call messages.
4. Deterministic code owns budgets, approval policy, tool confinement, and termination. The model
   never judges its own completion and read-only tools never prompt. In autonomous modes
   (`-p` with `--verify-cmd`/`--detect-checks`/`--require-diff`) a task is complete only when
   every verification check passes; failures are classified (`failure::FailureKind`) and each retry
   appends new evidence — never the same prompt twice. With `--completion-gate` a reply without
   tool calls is never an ending: the model must call `complete_task` (reviewed once, then
   recorded as a claim; the review is skipped only when the loop itself saw the proof — every
   requirement listed as met and a behavioural check it ran with exit 0 — `--always-review-claim`
   restores it). A claim, a diff, a model-declared check, a self-authored check, or an
   independent verifier's pass never yields `verified: true` or router `task_success: true`; only
   an explicit or detected check does. With `--independent-verifier` an accepted claim is reviewed
   by a fresh session (read-only tools, `report_verdict`, no memory of the work) and its unmet
   requirements become the next attempt's evidence.
5. Tool paths are confined to the workspace root: no absolute paths, no `..`, no `.git` internals,
   no secret-bearing files, realpath re-check after resolution. Commands execute as argv vectors —
   never a shell — with credential-stripped environments. Strictly-allowlisted commands may
   auto-run per the permission mode; anything else requires explicit human approval EVEN in auto
   mode (headless runs deny instead); privilege escalation (sudo/su/doas) never runs.
   `--sandboxed` is the one relaxation: for a disposable container the operator owns it disables
   path confinement and the allowlist and exposes the `shell` tool — sudo/su/doas stay forbidden.
   Never run it on a machine you care about.
6. Never log or persist provider credentials in sessions, transcripts, telemetry, or editor
   history. The ONE sanctioned location is the dedicated credential store written by `/connect`
   (`~/.config/butterfly/credentials.toml` via the platform config dir, 0600 in a 0700 dir, keyed
   by provider id; the provider's `api_key_env` variable always takes precedence when set).
   Structured payloads persisted about config or errors pass key-based redaction. Telemetry
   (`telemetry.jsonl` beside the session log) carries ids, counts, durations, tool names, call
   previews, and failure kinds — never file contents or prompts. Conversation content IS persisted
   (required for `--resume`), with 0700/0600 permissions.
7. Provider cache-hit, cache-miss, input, and output token counts remain distinct end to end.
   Money is integer micro-USD; decimal USD is presentation only. Price cards are versioned and
   tier-aware (DeepSeek peak/off-peak UTC windows) and carry `verified`; an unverified card taints
   every cost it produces (`price_verified: false` in reports) and must never be presented as real
   money.
8. Tests use the scripted provider and temporary directories; the deterministic suite must never
   touch the network or a paid model. Live tests are opt-in via `BUTTERFLY_LIVE_TEST=1`.
9. The system prompt and tool schemas are the cross-session cached prefix. Any change to their
   bytes — even a beneficial one — invalidates that prefix once per repo and requires the
   operator's explicit sign-off with the trade-off stated. Never change prefix bytes as a side
   effect of an unrelated fix. New capabilities extend the prefix only behind an explicit mode
   (Atlas line, sandbox line, `shell` schema, completion line + `complete_task` schema, lean
   line, clarify line + `ask_user` schema; the
   verifier line + `report_verdict` schema exist only in the separate verifier session) and only
   by APPENDING. The CLI turns the lean line on by default since 2026-09-16 (sanctioned,
   measured on the private suite); `--no-lean` reproduces the earlier default bytes.
10. Model-agnosticism is a capability contract, not a lowest common denominator: the loop reads
   `Capabilities` (tools, parallel_tools, reasoning + replay, prompt_cache, limits) and dialects
   own vendor syntax. Adding a model means a descriptor (built-in or `[[models]]`), never a branch
   in the loop.

## Feature map

`docs/butterfly-features.md` is the canonical inventory of shipped / in-progress / planned /
rejected features. Update it in the same change that lands or removes a feature.

## Verification

Run `bun run --filter @monet/butterfly-cli check` (or the cargo commands above) after every change.
Cache-critical changes must keep the byte-prefix property test
(`requests_grow_append_only_between_provider_calls`) and the compaction invariant tests green.
