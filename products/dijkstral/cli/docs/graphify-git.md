# Graphify git integration and `watch`: black-box specification

This document specifies Graphify's git integration — `graphify hook install|uninstall|status`, the
`graph.json` merge driver `graphify merge-driver <base> <current> <other>` and its wiring — and
`graphify watch <path>`, then what dijkstral does instead where it deliberately differs. The
reference is PyPI `graphifyy` 0.9.86, the release of `graphify-format.md`. Everything was derived
by running the installed `graphify` binary in scratch directories and scratch `git init`
repositories with `GRAPHIFY_QUERY_LOG_DISABLE=1`; Graphify's source, skill files and hook scripts
were not read. `graphify hook install` was **never run** (the probing environment does not allow
it to write hooks, even into a scratch repository), so its footprint is inferred from what
`hook status` and `hook uninstall` recognise in hand-written files (§2), and what Graphify's
hook scripts execute is **Open**. dijkstral's hook scripts are its own (§2.6), so only their
structure and behaviour are specified, never Graphify's text.

`watch` needs the `watchdog` package, which the benchmark install lacks (`error: watchdog not
installed. Run: pip install watchdog`, exit 1); the probes put a scratch `watchdog` 6.0.0 on
`PYTHONPATH` and ran the same binary.

Status labels, as in the other specifications:

- **Rule**: observed consistently. Reproduce it, unless §6 lists a deliberate difference.
- **Quirk**: observed, but a defect or an accident of implementation. dijkstral does not
  reproduce it.
- **Open**: not resolved. Collected in §7.2.

## Contents

1. Harness, goldens and conventions
2. Hooks
3. `hook status`, `hook uninstall`: output and exit codes
4. The merge driver
5. `watch`
6. dijkstral differences
7. Quirks summary and open questions

---

## 1. Harness, goldens and conventions

- `../bench/git_goldens.py` regenerates `tests/fixtures/git/`: it builds every merge case from the
  fixture graphs, runs `graphify merge-driver O A B` on scratch copies, and runs `hook status` and
  `hook uninstall` in scratch repositories prepared with hand-written hook files, `.gitattributes`
  lines and `merge.graphify.*` keys. It never runs `hook install`.
- `tests/fixtures/git/merge/<case>/` holds `base.json`, `current.json`, `other.json` and, when the
  driver succeeded, Graphify's result as `graphify.json`. `tests/fixtures/git/cases.json` indexes
  the merge cases (exit code, last stderr line, whether `current` was left untouched) and the hook
  cases (command, exit code, stdout with the repository path as `<repo>`, and the hook files,
  `.gitattributes` and `merge.*` config after `uninstall`).
- `crates/git/tests/merge_goldens.rs` compares dijkstral's merge with Graphify's as data (nodes
  by id, links by endpoint pair, graph attributes) and applies each deliberate difference (§4.6)
  explicitly. `crates/cli/tests/git_cli.rs` and `watch_cli.rs` drive the commands in scratch
  repositories with `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`.

## 2. Hooks

### 2.1 Files and markers

- **Rule.** `hook install` manages two hooks, `post-commit` and `post-checkout` (`graphify --help`:
  "install post-commit/post-checkout git hooks"). `status` and `uninstall` report exactly these
  two, in that order, then the merge driver (§4.1). No `post-merge` or `post-rewrite` hook is
  involved.
- **Rule.** Graphify's code in a hook file is a block between marker lines:
  `# graphify-hook-start` … `# graphify-hook-end` in `post-commit`, and
  `# graphify-checkout-hook-start` … `# graphify-checkout-hook-end` in `post-checkout`. A marker
  is recognised only as a whole line: `echo x graphify-hook-start`, `# Graphify`, `# graphify` and
  the other spellings probed are not.
- **Rule.** A block is reported as installed even when its body is not the current one; `status`
  then says it is out of date (§3). With a start marker and no end marker `status` says
  "installed" without the out-of-date note. The exact body Graphify writes is **Open**.
- **Rule.** Hooks are looked up where git runs them: `core.hooksPath` when set (relative to the
  work tree top), else the common `.git/hooks`. From a linked worktree the shared hooks of the main
  repository are reported. `status` and `uninstall` work from any subdirectory of the work tree.

### 2.2 Coexisting with other hooks

- **Rule.** Removal (`uninstall`) deletes only the lines from the start marker through the end
  marker; text before and after is kept byte for byte (`#!/bin/sh\necho before\n<block>echo
  after\n` → `#!/bin/sh\necho before\necho after\n`).
- **Rule.** When nothing but the shebang (and blank lines) is left, the hook file is deleted.
- **Rule.** A hook without Graphify's markers is never modified by `uninstall`.
- **Open.** Where `install` puts the block in an existing hook (append, prepend), whether it
  refuses hooks in other languages, which mode it sets, and whether it writes the
  `.gitattributes` line and config keys of §4.1 (inferred from `status` and `uninstall`, and from
  `graphify --help` naming `merge-driver` "set up via hook install").

### 2.3 What the hooks run

- **Open.** The command lines, whether the rebuild is synchronous or in the background, its
  output, and what happens without a graph or without `graphify` on `PATH`. The help text says the
  hooks rebuild "the graph" after commits and checkouts.

### 2.4 Outside a repository

- **Rule.** `hook status` outside a git work tree prints `Not in a git repository.` and exits
  **0**.
- **Quirk.** `hook uninstall` outside a repository dies with a Python traceback ending
  `RuntimeError: No git repository found at or above <dir>`, exit 1.
- **Rule.** `hook` with no action or an unknown one prints `Usage: graphify hook
  [install|uninstall|status]`, exit 1.

### 2.5 Worktrees and `core.hooksPath`

See §2.1: the hooks directory is git's. A relative `core.hooksPath` resolves against the work tree
top.

### 2.6 dijkstral's hooks

dijkstral's block is its own text between `# dijkstral-hook-start` and `# dijkstral-hook-end` (the
same marker pair in both hooks). It is POSIX `sh`, uses no `exit`, so code after it in the same
file still runs, and:

- **Never breaks git.** All output goes to `/dev/null`, a failure is swallowed (`|| :`), and
  `DIJKSTRAL_HOOK_SKIP=<non-empty>` turns it off. It calls the absolute path of the binary that
  ran `hook install`, falling back to `dijkstral` on `PATH`, and does nothing when neither exists.
- **Is fast.** It runs `dijkstral hook run <hook> "$@"`, which returns at once: it starts a
  detached worker (its own process group, git's `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE`/
  `GIT_PREFIX` removed) and exits 0 without waiting. A commit with the hook measured 0.05 s wall
  time in the end-to-end check. `DIJKSTRAL_HOOK_SYNC=1` rebuilds in the foreground instead (CI,
  tests).
- **Rebuilds only an existing graph.** Nothing happens unless `dijkstral-out/graph.json` exists at
  the work tree top; building the first graph stays the user's decision. `post-commit` always
  rebuilds; `post-checkout` only for a branch checkout (third argument `1`) that changed `HEAD`
  (first two arguments differ).
- **Rebuilds with `dijkstral update <top>`**: code re-extracted through the cache, the markdown
  structure, the clustered graph, the report and `graph.html`, no model call; non-code changes the
  semantic pass has not seen set `needs_update` for `check-update` exactly as a manual `update`
  does. The output of the last hook rebuild is in `dijkstral-out/hook.log`.
- **Serialises and coalesces.** Workers take an exclusive lock on `dijkstral-out/.hook.lock`. A
  trigger that finds the lock held writes `dijkstral-out/.hook.pending` and leaves; the running
  worker rebuilds once more when it finds the note (at most 8 rounds per worker), so a rebase that
  fires `post-commit` for every commit costs one or two rebuilds, not one per commit.
- **Install** appends the block to an existing shell hook (after one blank line), replaces an
  older block in place ("refreshed"), leaves an identical one alone ("already installed"), and
  creates a missing hook as `#!/bin/sh` plus the block, mode 0755. An existing hook is made
  executable (and the report says so if it was not). A hook whose shebang names another
  interpreter (`python3`, `node`, …) is refused with a hint to call `dijkstral hook run` from it,
  as is a block with a start marker and no end marker; the command then exits 1 after doing the
  rest.
- **Uninstall** removes the block and the blank line install put before it, so a foreign hook is
  restored byte for byte, and deletes a hook left with only a shebang.

## 3. `hook status`, `hook uninstall`: output and exit codes

Graphify (goldens in `cases.json`, `hooks`):

| State | `status` line | `uninstall` line |
|---|---|---|
| no hook file | `post-commit: not installed` | `post-commit: no post-commit hook found - nothing to remove.` |
| hook without markers | `post-commit: not installed (hook exists but graphify not found)` | `post-commit: graphify hook not found in post-commit - nothing to remove.` |
| block, other content | `post-commit: installed (out of date: run \`graphify hook install\` to refresh it)` | `post-commit: graphify removed from post-commit at <path> (other hook content preserved)` |
| block only | same | `post-commit: removed post-commit hook at <path>` |
| no driver | `merge driver: not registered` | `merge driver: not registered - nothing to remove.` |
| config only | `merge driver: partially registered (git config set, .gitattributes line missing)` | `merge driver: not registered - nothing to remove.` (the config section **is** removed) |
| attributes only | `merge driver: partially registered (.gitattributes line set, git config missing)` | `merge driver: removed from .gitattributes (other entries preserved)` |
| both | `merge driver: registered` | `merge driver: removed (.gitattributes deleted - no other entries)` or the line above |

- **Rule.** Three lines, always in the order post-commit, post-checkout, merge driver; exit 0.
- **Quirk.** With the config section but no `.gitattributes` line, `uninstall` removes the section
  yet reports "not registered - nothing to remove".

dijkstral (its own words, same three-line shape):

| `status` | meaning |
|---|---|
| `post-commit: installed` | current block, and the binary it names is executable |
| `post-commit: installed, outdated (run \`dijkstral hook install\` again)` | an older block, or its binary is gone |
| `post-commit: not installed` / `not installed (a hook exists, without dijkstral's block)` | |
| `post-commit: broken (…); fix it by hand` | start marker without end marker |
| `merge driver: configured` / `half configured (…)` / `not configured` | §4.1 |

`uninstall` lines: `removed <path>` (file deleted), `removed dijkstral's block from <path>; the
rest is kept`, `nothing to remove`; for the driver, `removed git config merge.dijkstral.* and its
.gitattributes entry` (or `.gitattributes (nothing else was in it)` when the file is deleted).
Exit 0, or 1 when part of an install was refused (§2.6). Outside a work tree (or in a bare
repository) every action prints `error: not inside a git work tree` to stderr and exits 1.

## 4. The merge driver

### 4.1 Wiring

- **Rule.** Registered means two things, both checked by `status` (§3): a `merge.graphify.driver`
  key in the repository's git config, and a `.gitattributes` line routing the graph to it,
  `graphify-out/graph.json merge=graphify`. `status` accepts any line containing
  `merge=graphify` (also `graph.json merge=graphify`, or with `-diff` after it); `uninstall`
  removes only lines that are exactly that line after trimming, and the whole `[merge "graphify"]`
  section (`name` and `driver`).
- **Rule.** When removing the line leaves `.gitattributes` empty, the file is deleted.
- **Open.** The `name` and `driver` values `install` writes (git passes `%O %A %B`; the help's
  argument order `<base> <current> <other>` matches).

dijkstral registers `merge.dijkstral.name = dijkstral graph.json merge`,
`merge.dijkstral.driver = '<absolute dijkstral>' merge-driver %O %A %B` (local config, shared by
linked worktrees) and appends `dijkstral-out/graph.json merge=dijkstral` to the top-level
`.gitattributes` (which the user commits; every clone runs `hook install` for the config).
`status` requires a line whose pattern is exactly `dijkstral-out/graph.json` with the attribute
`merge=dijkstral`; `uninstall` removes that attribute, keeps other attributes on the same line,
and deletes `.gitattributes` when nothing else is left.

### 4.2 Invocation and exit codes

- **Rule.** `merge-driver O A B` reads the three files, writes the result over `A` (git's
  `%A`, the current branch's version) and prints nothing; exit 0.
- **Rule.** On failure it exits 1 and leaves `A` byte-identical, which git reports as a conflict:
  `B` not JSON (`[graphify merge-driver] error loading graphs: <json error>` on stderr), a JSON
  array, one side directed and the other not, or an **empty base** (§4.5).

### 4.3 Identity and union semantics

Observed on synthetic graphs and the fixture cases:

- **Rule (base).** The base is read but does not influence the result: a node or link deleted on
  one side and present on the other is kept (**Quirk G1**, the deletion is undone); an edit made
  only on the current side is overwritten by the other side's untouched copy (**Quirk G2**).
- **Rule (nodes).** Identity is `id`. The result has the current side's nodes in its order, then
  the other side's new ones in theirs. A node on both sides gets the current side's attributes
  updated by the other side's (`{**current, **other}`): every conflicting key takes the other
  side's value, keys only one side has are kept.
- **Rule (links).** The graph is undirected and simple: identity is the **unordered** endpoint
  pair. A pair on both sides merges like a node (`{**current, **other}`), including its
  direction: the other side's `source`/`target` win (a flipped link comes out flipped). Two
  different relations on one pair become one link with the other side's relation.
- **Rule (link order).** Links come out in NetworkX adjacency order: for each node in the result
  order, its neighbours in first-insertion order, each pair once.
- **Rule (missing endpoints).** A link to an id that is not a node creates a node `{"id": …}`
  with no other key.
- **Rule (duplicates).** Duplicate ids or pairs within one side fold the same way, later wins.
- **Rule (graph attributes).** `graph` is `{**current.graph, **other.graph}`. Since Graphify keeps
  its hyperedges in `graph.hyperedges` as well (`graphify-semantic.md` S7), the other side's
  hyperedge list replaces the current side's whole.
- **Quirk G3.** The top-level `hyperedges` array is dropped, and so are `built_at_commit` and any
  other top-level key; only `directed`, `multigraph`, `graph`, `nodes`, `links` are written.
- **Quirk G4.** A raw `--no-cluster` graph (`edges`, no `directed`/`multigraph` header) is loaded
  as a multigraph: the result says `"multigraph":true`, every link gets `"key":0`, and `graph` is
  `{}`.
- **Rule.** No re-clustering: `community` values are whatever the inputs had, so ids from two
  builds can mean different groups until the next build.

### 4.4 Output format

- **Rule.** Compact JSON, ASCII-escaped (`ensure_ascii`), no trailing newline, Python float
  `repr` (`1e-07`, `1e+20`, `0.30000000000000004`).
- **Quirk G5.** Key order is NetworkX's: a node's attributes then `id` last; a link's attributes,
  then `source`, `target` (and `key` in a multigraph). Neither the key order nor the array order of
  a build (`graphify-format.md` §2.2–§2.3) is kept, so merging a graph with itself rewrites it.

### 4.5 Failures

- **Quirk G6.** An empty base file — what git passes when both branches added `graph.json`
  without a common ancestor — fails (`All graphs must be graphs or multigraphs.`, exit 1), so such
  merges always conflict.
- **Rule.** Mixed directedness fails (`All graphs must be directed or undirected.`, exit 1).
- Both print a Python traceback before the last line.

### 4.6 dijkstral's merge (deliberate differences)

`crates/git/src/merge.rs`; goldens and tests in `crates/git/tests/merge_goldens.rs`.

1. **Three-way, per value.** For every node (by `id`), link (by unordered endpoints; ordered when
   `directed`; plus `key` in a multigraph) and hyperedge (by `id`), and for every key inside them,
   in `graph` and among other top-level keys: a side that kept the base value yields to the side
   that changed it; when both changed it differently the other side wins (as in Graphify), except
   that a removal never beats a change. A link's `source`/`target` merge as one value, so a link
   is never half flipped. Graphify's G1 and G2 do not happen.
2. **Deletions stand** unless the other side changed the item. Changes to `community` and
   `community_name` alone do not count as changes (every build renumbers communities), so a node
   deleted on one branch is not resurrected because the other branch re-clustered. Links whose
   node the merge removed are dropped with it; links to ids that were never nodes are kept.
3. **Hyperedges merge by id** and are written both as `hyperedges` and, when non-empty, as
   `graph.hyperedges`, like a build (no G3). `built_at_commit` is kept only when both sides carry
   the same value.
4. **The output is a build's** (`graphify-format.md` §2.2–§2.3): nodes and links sorted by
   `json.dumps(obj, sort_keys=True, ensure_ascii=False)`, `id`/`label` and
   `source`/`target`/`relation` first and the rest sorted, compact, ASCII-escaped, Python floats,
   no trailing newline. Merging a graph with itself reproduces it byte for byte, for dijkstral's
   builds and for Graphify's clustered graphs (no G5), so the result is deterministic and
   independent of array order.
5. **Raw graphs stay raw** (no G4): two `--no-cluster` graphs merge with links identified by
   `(source, target, relation)`, as the raw writer dedupes them, and are written raw (`nodes`,
   `edges`, `hyperedges`, then the other keys, in merged order).
6. **An empty or unreadable base is a union** (no G6). Bad current or other input, or
   disagreeing `directed`/`multigraph`, exits 1 with `A` untouched and one stderr line
   `dijkstral merge-driver: cannot merge graph.json (<reason>); left as a conflict`.
7. No re-clustering, as in Graphify: the hooks' next rebuild renumbers communities.

## 5. `watch`

### 5.1 Startup and stop

- **Rule.** `graphify watch <path>` prints three lines and waits; it does **not** build at start:

  ```text
  [graphify watch] Watching <real path> - press Ctrl+C to stop
  [graphify watch] Code changes rebuild graph automatically. Doc/image changes require /graphify --update.
  [graphify watch] Debounce: 3.0s
  ```

- **Rule.** Ctrl-C (SIGINT) prints an empty line and `[graphify watch] Stopped.`, exit 0. SIGTERM
  kills it silently (exit status −15).
- **Rule.** A missing path: `error: path not found: <path>`, exit 1. A file path is accepted and
  watched.

### 5.2 Triggers and debounce

- **Rule.** The debounce is **trailing**: changes at 2.0 s, 4.0 s and 6.0 s gave one rebuild at
  9.2 s, 3.1–3.4 s after the last change, reported as `N file(s) changed` with the number of
  distinct paths.
- **Rule.** Creating, editing and deleting a code file all trigger; a deletion rebuild drops the
  file's nodes.
- **Rule.** Each rebuild block starts with an empty line, then `[graphify watch] N file(s)
  changed`, then either `[graphify watch] Rebuilt: <n> nodes, <m> edges, <c> communities` and
  `[graphify watch] graph.json, graph.html and GRAPH_REPORT.md updated in <out dir>`, or
  `[graphify watch] No code-graph topology changes detected; outputs left untouched.` (an edit
  that changes no node or edge). Warnings of the build appear in between. The first rebuild in a
  directory without a graph creates `graphify-out/` (`graph.json`, `graph.html`,
  `GRAPH_REPORT.md`, the labels, manifest and cache files).

### 5.3 Code versus non-code changes

- **Rule.** A batch with only non-code files (`README.md`, `notes/a.txt`) does not rebuild; it
  writes `graphify-out/needs_update` (one byte, `1`) and prints four lines: an empty line,
  `[graphify watch] New or changed files detected in <path>`, `… Non-code files changed - semantic
  re-extraction requires LLM.`, `… Run \`/graphify --update\` in Claude Code to update the graph.`,
  `… Flag written to <out>/needs_update`. `check-update` then reports pending changes
  (`graphify-semantic.md` §9.3). The flag is not cleared by later code rebuilds.
- **Quirk G7.** A later code rebuild indexes the new `README.md` as a code file ("yielded no
  symbols").

### 5.4 Ignore rules

- **Rule.** Changes under `graphify-out/` and under `.git/` are ignored.
- **Quirk G8.** Changes under hidden directories (`.hidden/z.py`) are ignored although the build
  indexes hidden directories.
- **Quirk G9.** `.gitignore` is not consulted and skipped directories still count:
  `ignored_dir/y.py` (gitignored) and `node_modules/x.py` each started a rebuild that changed
  nothing.

### 5.5 dijkstral's `watch`

`crates/cli/src/watch_cmd.rs`, tests in `crates/cli/tests/watch_cli.rs`.

- `dijkstral watch [PATH] [--debounce SECS]` (default `.` and **1.0 s**, trailing) uses native
  file events (`notify`: FSEvents, inotify, ReadDirectoryChangesW).
- It runs `dijkstral update PATH` **once at start** (a current graph costs a stat pass and says
  "unchanged"), then after every relevant batch. Each batch prints an empty line, then
  `[dijkstral watch] N changed: a, b, c (+k more)`, then `update`'s own output (counts, timings,
  `wrote …/graph.json`, or `… is current`).
- A path counts only if a build would read it: a file in the corpus before or after the batch
  (discovery with every ignore rule, skipped directory and the sensitive-file filter), an ignore
  file (`.gitignore`, `.graphifyignore`, `.dijkstralignore`), or a directory whose corpus files
  appeared or disappeared. Events under `dijkstral-out/` or any skipped directory (`.git`,
  `node_modules`, `__pycache__`, …) are dropped before the debounce, so the build's own writes
  never trigger another build (no G8, no G9).
- Code and document changes are treated alike: `update` re-extracts code through the cache and
  rebuilds the markdown structure without a model, and flags `needs_update` only when a file the
  semantic pass read changed or a new one appeared (`graphify-semantic.md` §11.1). No flag is
  written for a directory where the semantic pass never ran.
- Ctrl-C or SIGTERM stops it between rebuilds: an update in progress finishes (or, for a terminal
  Ctrl-C, is interrupted with it; its writes are atomic), then `[dijkstral watch] stopped`, exit 0.
  A second Ctrl-C exits at once with 130. A missing directory: `error: reading <path>: …`, exit 1.

## 6. dijkstral differences

| Area | Graphify | dijkstral |
|---|---|---|
| names | `graphify-out/`, `merge.graphify.*`, `merge=graphify`, `# graphify-hook-start` | `dijkstral-out/`, `merge.dijkstral.*`, `merge=dijkstral`, `# dijkstral-hook-start` |
| hook body | Open (§2.3) | §2.6: background `dijkstral update`, lock and coalescing, fail-open, skip/sync switches |
| hooks without a graph | Open | do nothing |
| non-shell hook | Open | refused, exit 1 |
| status/uninstall wording | §3 | own wording, same three-line shape |
| outside a work tree | `status` exit 0, `uninstall` traceback | every action: stderr error, exit 1 |
| merge semantics | base ignored (G1, G2), hyperedges dropped (G3), raw → multigraph (G4), NetworkX order (G5), empty base fails (G6) | three-way, deletions stand, hyperedges kept, raw stays raw, build order, empty base is a union (§4.6) |
| `watch` start | no build | one `update` at start |
| debounce | 3.0 s | 1.0 s, `--debounce` |
| non-code changes | `needs_update`, no rebuild | `update` (markdown structure), `needs_update` via the semantic index |
| watch ignores | `.git`, `graphify-out`, hidden dirs (G8); not `.gitignore` (G9) | the build's corpus rules exactly |

## 7. Quirks summary and open questions

### 7.1 Quirks (not reproduced)

| # | Quirk | Where |
|---|---|---|
| G1 | The merge driver undoes deletions: an item deleted on one side comes back from the other | §4.3 |
| G2 | An edit made only on the current side is overwritten by the other side's unchanged copy | §4.3 |
| G3 | The merged graph loses the top-level `hyperedges`, `built_at_commit` and other top-level keys; `graph.hyperedges` is the other side's list whole | §4.3 |
| G4 | Raw graphs merge into `"multigraph":true` with `key` on every link | §4.3 |
| G5 | Merged output is in NetworkX key and array order, not a build's | §4.4 |
| G6 | An empty base (add/add) fails, so the merge always conflicts | §4.5 |
| G7 | After a non-code change, the next code rebuild indexes the new document as code | §5.3 |
| G8 | `watch` ignores hidden directories the build indexes | §5.4 |
| G9 | `watch` rebuilds for gitignored and skipped paths | §5.4 |
| — | `hook uninstall` outside a repository raises a traceback; with only the config section it removes it and says nothing was there | §2.4, §3 |

### 7.2 Open

1. What `graphify hook install` writes: hook bodies, placement in existing hooks, modes,
   refusal cases, the `merge.graphify.name`/`driver` values and the `.gitattributes` line it adds
   (§2.2, §2.3, §4.1). It needs an environment that permits running `hook install` in a scratch
   repository.
2. Whether Graphify's hooks rebuild synchronously or in the background, and with which command
   (`update` is likely, given the help text), and what they print (§2.3).
3. `graphify watch` with no path argument (not probed, it blocks).
4. Graphify's merge driver on multigraph or directed inputs on both sides beyond the synthetic
   probe (both directed: ordered pairs, both directions kept).
5. dijkstral: `watch` on Linux watches every directory recursively through inotify, skipped ones
   included; trees with very many directories (large `target/` or `node_modules/`) can exceed
   `fs.inotify.max_user_watches`.
