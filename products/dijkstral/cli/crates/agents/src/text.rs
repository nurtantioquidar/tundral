//! Every piece of prose dijkstral shows an assistant. Written for dijkstral; nothing here is taken
//! from Graphify, whose installed files are deliberately never read (see `docs/graphify-query.md`).

/// `additionalContext` for a `Grep` or a search program run through `Bash`.
pub const SEARCH_NUDGE: &str = "dijkstral: this project has a code graph in dijkstral-out/. \
For who calls or uses a symbol run `dijkstral explain <symbol>`, for what a change affects \
`dijkstral affected <symbol>`, for how two symbols connect `dijkstral path <a> <b>`, for a broad \
topic `dijkstral query \"<question>\"`; each answer cites file:line. Text search remains right \
for literal strings the graph cannot know.";

/// `additionalContext` for reading a source file, or globbing for source files.
pub const READ_NUDGE: &str = "dijkstral: this project has a code graph in dijkstral-out/. \
Before reading source to find where something lives or how parts connect, ask the graph: \
`dijkstral explain <symbol>` for a symbol, its definition, callers and callees, \
`dijkstral affected <symbol>` for what depends on it, `dijkstral path <a> <b>` for how two \
symbols link, `dijkstral query \"<question>\"` for a broad topic. Answers cite file:line, so you \
can read just those lines. Give any subagent you start the same instruction.";

/// `additionalContext` for reading a file that changed after the graph was built.
pub const STALE_NOTE: &str = "dijkstral: this file changed after the code graph was last \
built, so graph answers about it may be out of date. Reading it directly is the right call; run \
`dijkstral update` afterwards to refresh the graph (only changed files are re-extracted).";

/// `permissionDecisionReason` for the once-per-session denial under `hook-guard read --strict`.
pub const STRICT_DENY: &str = "dijkstral strict mode: consult the code graph before reading \
source in this session. Run `dijkstral explain <symbol>`, `dijkstral affected <symbol>`, \
`dijkstral path <a> <b>` or `dijkstral query \"<question>\"`; each answer cites file:line, so \
afterwards you can read only the lines that matter. This check blocks a single read per session; \
retrying the read now is allowed.";

/// Heading of the project section in `CLAUDE.md` and `AGENTS.md`.
pub const PROJECT_TITLE: &str = "dijkstral";

/// Project `CLAUDE.md` section (Claude Code also gets nudges from the installed hooks).
pub const CLAUDE_SECTION: &str = "## dijkstral

This project has a dijkstral code graph in `dijkstral-out/` (`graph.json`, plus `GRAPH_REPORT.md`
with its communities and most connected symbols).

Ask the graph before grepping or reading source broadly. Pick the command that fits the
question; each cites `file:line`, so read only those lines next:

- Where a symbol is defined, and what calls it or what it calls: `dijkstral explain <symbol>`
  (`<--` lines are callers and users, `-->` lines what it uses; `--all` lists every connection).
- What a change to a symbol would affect: `dijkstral affected <symbol>`.
- How one symbol reaches another: `dijkstral path <from> <to>`.
- A broad topic or feature with no symbol name yet: `dijkstral query \"<question>\"`; it returns
  a neighbourhood of matching symbols, so `explain` the ones that look central.
- A name in several files is reported as ambiguous with its candidates; retry as
  `path/to/file.py::symbol`.
- Text search is still right for literal strings, log messages and configuration values.
- Run `dijkstral update` after changing code; it re-extracts only the files that changed.
";

/// Project `AGENTS.md` section for Codex, which gets no hook nudges, so it says when to ask.
pub const AGENTS_SECTION: &str = "## dijkstral

This project has a dijkstral code graph in `dijkstral-out/` (`graph.json`, plus `GRAPH_REPORT.md`
with its communities and most connected symbols).

Ask the graph before running `rg`, `grep` or `find` over the source, or opening files to learn
the code's structure. Pick the command that fits the question; each cites `file:line`, so
read only those lines next:

- Where a symbol is defined, and what calls it or what it calls: `dijkstral explain <symbol>`
  (`<--` lines are callers and users, `-->` lines what it uses; `--all` lists every connection).
- What a change to a symbol would affect: `dijkstral affected <symbol>`.
- How one symbol reaches another: `dijkstral path <from> <to>`.
- A broad topic or feature with no symbol name yet: `dijkstral query \"<question>\"`; it returns
  a neighbourhood of matching symbols, so `explain` the ones that look central.
- A name in several files is reported as ambiguous with its candidates; retry as
  `path/to/file.py::symbol`.
- Text search is still right for literal strings, log messages and configuration values.
- Run `dijkstral update` after changing code; it re-extracts only the files that changed.
";

/// Level-1 section for the user-level (or project-level) `.claude/CLAUDE.md` next to the skill.
pub const SKILL_POINTER_SECTION: &str = "# dijkstral

When a project contains `dijkstral-out/graph.json`, use the `dijkstral` skill: ask the code graph
(`dijkstral explain`, `affected`, `path`, `query`) before searching or reading source broadly.
";

/// `SKILL.md` for Claude Code and Codex.
pub const SKILL_MD: &str = "---
name: dijkstral
description: Build and query a dijkstral code knowledge graph of the current repository. Use when exploring unfamiliar code, locating where a feature or symbol lives, tracing how two parts connect, or before broad grep/find/read sweeps in a project that has dijkstral-out/graph.json.
---

# dijkstral

dijkstral indexes a repository with tree-sitter into a graph of files, classes, functions and
their relations (imports, calls, inheritance, containment), clusters it into communities, and
answers questions about it locally. Building the code graph involves no model or network.

## Build or refresh

- `dijkstral extract . --code-only` builds the code graph into `dijkstral-out/` (`graph.json`,
  `GRAPH_REPORT.md`), locally.
- `dijkstral update .` refreshes it after edits, also without a model; unchanged files come from
  the cache, and an unchanged tree is detected without rebuilding.

Build once when a project has no `dijkstral-out/graph.json`; update after changing code.

`dijkstral extract .` without `--code-only` also sends the project's documents, PDFs and images
to a model (the Codex CLI by default) to add concepts from them. Run it only when the user asks
for it: it uses their model quota.

## Ask

Pick the command that fits the question:

- `dijkstral explain <symbol>`: one symbol, where it is defined, and what it connects to; `<--`
  lines are its callers and users, `-->` lines what it uses (`--all` lists every connection).
- `dijkstral affected <symbol>`: what depends on the symbol, following calls, imports and
  inheritance backwards (`--depth N`, default 2): what a change to it would affect.
- `dijkstral path <from> <to>`: the shortest chain of relations linking two symbols.
- `dijkstral query \"<question>\"`: a neighbourhood of the symbols matching a broad question,
  cut to a token budget; use it to find the central symbols, then `explain` them.

Names can be qualified as `path/to/file.py::Symbol` when a bare name is ambiguous. Exit code 3
means the name was ambiguous (candidates are listed), 4 that nothing matched.

## Working with the answers

- Read `dijkstral-out/GRAPH_REPORT.md` once for orientation: communities, the most connected
  symbols, and surprising cross-module links.
- Open files only at the cited lines, not whole directories.
- Use plain text search for literal strings (error messages, config keys) the graph does not
  model.
- If an answer looks wrong after recent edits, run `dijkstral update` and ask again.
";
