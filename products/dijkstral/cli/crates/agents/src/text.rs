//! Every piece of prose dijkstral shows an assistant. Written for dijkstral; nothing here is taken
//! from Graphify, whose installed files are deliberately never read (see `docs/graphify-query.md`).

/// `additionalContext` for a `Grep` or a search program run through `Bash`.
pub const SEARCH_NUDGE: &str = "dijkstral: this project has a code graph in dijkstral-out/. \
Run `dijkstral context \"<question>\"` first: one call returns the symbols the question is \
about with their definition lines, callers (with the calling lines) and callees. Text search \
remains right for literal strings and for every occurrence of something.";

/// `additionalContext` for reading a source file, or globbing for source files.
pub const READ_NUDGE: &str = "dijkstral: this project has a code graph in dijkstral-out/. \
Before reading source to find where something lives or how parts connect, run \
`dijkstral context \"<question>\"`: one call returns the relevant definitions, callers with \
their calling lines, and callees, each cited by file:line. Give any subagent you start the \
same instruction.";

/// `additionalContext` for reading a file that changed after the graph was built.
pub const STALE_NOTE: &str = "dijkstral: this file changed after the code graph was last \
built, so graph answers about it may be out of date. Reading it directly is the right call; run \
`dijkstral update` afterwards to refresh the graph (only changed files are re-extracted).";

/// `permissionDecisionReason` for the once-per-session denial under `hook-guard read --strict`.
pub const STRICT_DENY: &str = "dijkstral strict mode: consult the code graph before reading \
source in this session. Run `dijkstral context \"<question>\"`; it cites file:line, so \
afterwards you can read only the lines that matter. This check blocks a single read per \
session; retrying the read now is allowed.";

/// Heading of the project section in `CLAUDE.md` and `AGENTS.md`.
pub const PROJECT_TITLE: &str = "dijkstral";

/// Project `CLAUDE.md` section (Claude Code also gets nudges from the installed hooks).
pub const CLAUDE_SECTION: &str = "## dijkstral

This project has a dijkstral code graph in `dijkstral-out/`.

- Start every question about this code with one call: `dijkstral context \"<question>\"`. It
  points to where the answer is: the symbols the question is about with their definition lines,
  their callers (with the calling lines) and what they call, and other symbols named like the
  question, each with `file:line`.
- It is a map, not the answer: before answering, read the code it points to (several ranges in
  one `sed -n` call) and check there each step or part you name. When it does not show what the
  question asks about, search the text with `rg`.
- For more of the structure: `dijkstral explain A B --incoming` (callers of several symbols),
  `dijkstral affected <symbol>` (what a change affects), `dijkstral path <a> <b>`.
- The graph is not a full record: for literal strings, exact values and dates, and when a
  question needs every occurrence (how many, all of, the latest), also search the text with `rg`.
- Run `dijkstral update` after changing code; it re-extracts only the files that changed.
";

/// Project `AGENTS.md` section for Codex, which gets no hook nudges, so it says when to ask.
pub const AGENTS_SECTION: &str = "## dijkstral

This project has a dijkstral code graph in `dijkstral-out/`.

- Start every question about this code with one call: `dijkstral context \"<question>\"`. It
  points to where the answer is: the symbols the question is about with their definition lines,
  their callers (with the calling lines) and what they call, and other symbols named like the
  question, each with `file:line`.
- It is a map, not the answer: before answering, read the code it points to (several ranges in
  one `sed -n` call) and check there each step or part you name. When it does not show what the
  question asks about, search the text with `rg`.
- For more of the structure: `dijkstral explain A B --incoming` (callers of several symbols),
  `dijkstral affected <symbol>` (what a change affects), `dijkstral path <a> <b>`.
- The graph is not a full record: for literal strings, exact values and dates, and when a
  question needs every occurrence (how many, all of, the latest), also search the text with `rg`.
- Run `dijkstral update` after changing code; it re-extracts only the files that changed.
";

/// Level-1 section for the user-level (or project-level) `.claude/CLAUDE.md` next to the skill.
pub const SKILL_POINTER_SECTION: &str = "# dijkstral

When a project contains `dijkstral-out/graph.json`, use the `dijkstral` skill: start with
`dijkstral context \"<question>\"` before searching or reading source broadly.
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

- `dijkstral context \"<question>\"`: start here. One call returns the symbols the question is
  about (or the best-matching ones) with their definition lines, callers with the calling lines,
  and callees; for documents, the matching facts with their source lines and a search for every
  occurrence. Often the whole answer.
- `dijkstral explain <symbol>...`: symbols and all their connections; `--incoming` only callers
  and users, `--outgoing` only what they use, `--all` every connection.
- `dijkstral affected <symbol>`: what depends on the symbol (`--depth N`, default 2).
- `dijkstral path <from> <to>`: the shortest chain of relations linking two symbols.
- `dijkstral query \"<question>\"`: a neighbourhood of matching symbols, Graphify's format.

Names can be qualified as `path/to/file.py::Symbol` when a bare name is ambiguous. Exit code 3
means the name was ambiguous (candidates are listed), 4 that nothing matched.

## Working with the answers

- Read `dijkstral-out/GRAPH_REPORT.md` once for orientation: communities, the most connected
  symbols, and surprising cross-module links.
- Open files only at the cited lines, not whole directories.
- The graph is a map, not a full record: use plain text search for literal strings (error
  messages, config keys), exact values and dates, and whenever a question needs every
  occurrence (how many, all of, the latest).
- If an answer looks wrong after recent edits, run `dijkstral update` and ask again.
";
