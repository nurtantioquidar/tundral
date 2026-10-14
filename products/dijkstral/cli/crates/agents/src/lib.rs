//! Assistant integrations: the `PreToolUse` hook guard and the Claude Code and Codex installers.
//!
//! Behaviour follows `docs/graphify-query.md` §11–§12 (the black-box record of Graphify's hook
//! guard and install footprint); every instruction and skill text is dijkstral's own (`text.rs`).
//! The guard is on the hot path of every agent tool call, so this crate stays small: it never
//! opens the graph, only stats a few paths.
//!
//! Deliberate differences from Graphify 0.9.86 (§15.1 quirks not reproduced):
//!
//! - The guard watches `dijkstral-out/graph.json`; a directory there counts as no graph in both
//!   modes. A file counts as stale only if it is newer than both `graph.json` and the build digest
//!   (an unchanged rebuild touches the digest), or `needs_update` exists.
//! - Read/Glob extensions add `mjs mts cts pyi bash zsh`; `~/` is expanded; `*.{ts,tsx}` braces
//!   are expanded; an absolute Glob pattern outside the project is silent.
//! - Strict mode fails open (no denial) when its session marker cannot be written.
//! - Installers own only entries that run `dijkstral hook-guard|hook-check` under our matchers;
//!   user hooks merely mentioning dijkstral survive. Settings are spliced, not re-serialised, no
//!   `*-bak` copies are written, an invalid settings file stops the install before any write, and
//!   an uninstall drops the hook list and `hooks` object it emptied.
//! - Sections end at a heading of the same or higher level, ignore fenced code, and the global
//!   `.claude/CLAUDE.md` pointer is removed by `dijkstral uninstall`. `claude uninstall` does not
//!   touch the user-level skill.

mod bash;
pub mod guard;
pub mod install;
mod jsonedit;
mod section;
mod text;

/// Output directory a build writes, relative to the project root (`dijkstral_core::pipeline::OUT_DIR`).
pub const OUT_DIR: &str = "dijkstral-out";
/// The graph whose presence turns the guard on.
pub const GRAPH_FILE: &str = "graph.json";
/// Rewritten by every build and touched by an unchanged rebuild (`dijkstral_core::pipeline`), so
/// its mtime says when the graph was last confirmed current.
pub const DIGEST_FILE: &str = ".build-digest";
/// Marker that forces every `Read` to count as stale until the next build removes it.
pub const NEEDS_UPDATE_FILE: &str = "needs_update";
