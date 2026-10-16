# Graphify work memory, PR and diagnostic tools: black-box specification

This document specifies four Graphify commands that sit beside the graph rather than build it:
`save-result` and `reflect` (the work-memory loop), `prs` (a pull-request dashboard, plus the
three PR tools of `graphify-mcp`) and `diagnose multigraph` (an edge-collapse report). The
reference is PyPI `graphifyy` 0.9.86, as in `graphify-format.md` and `graphify-query.md`.
Everything here was derived by running the installed `graphify` and `graphify-mcp` binaries in
scratch directories (`HOME` pointed at a scratch directory, `GRAPHIFY_QUERY_LOG_DISABLE=1`) on the
fixture graphs and on small inputs written for one question each. GitHub was reached only through
a fake `gh` that logs its arguments and serves canned JSON, except for one read-only run of `prs`
in a scratch shallow clone of a public repository (fastapi) through a logging wrapper that refused
anything but `repo view`, `pr list`, `pr view`, `pr diff` and GET `api` calls. Graphify's source,
skill files and prompts were not read; its prose is not reproduced here or in dijkstral.

Status labels, as in the other specifications:

- **Rule**: observed consistently. Reproduce it.
- **Quirk**: observed, but a defect or an accident of implementation. Dijkstral reproduces it only
  where a consumer could depend on it; §7 lists what it does instead.
- **Open**: not resolved; collected in §8.

## Contents

1. Harness, goldens and conventions
2. `save-result`
3. The memory directory and the other commands
4. `reflect`
5. `prs` and the MCP PR tools
6. `diagnose multigraph`
7. Dijkstral differences
8. Quirks summary and open questions

---

## 1. Harness, goldens and conventions

- Goldens live in `tests/fixtures/memory/`, indexed by `cases.json` (`{"tool", "how",
  "save_result", "reflect", "diagnose", "prs", "prs_mcp"}`). Each record holds the argv, exit code,
  stdout (inline or as a file) and what stderr began with.
- `save/<case>.md` are files `save-result` wrote. Their time stamp and 8-hex token differ on every
  run (§2.2), so they are compared with the `date` value and the file name's stamp and token
  masked.
- `reflect/<case>/memory/` are crafted memory directories with fixed dates; `reflect/<case>/` also
  holds crafted graphs, analyses and labels, and the python fixture graph and analysis are used by
  path. `reflect/<case>.graphify.md` is Graphify's output. Because every date is fixed and every
  ordering depends only on date differences (§4.4), these outputs do not depend on the day they
  were captured, except for the dates in the future (2030, 2031), which are clamped (§4.4). The
  tests pin dijkstral's clock to the capture day with the test-only `DIJKSTRAL_NOW` (seconds since
  the epoch; `SOURCE_DATE_EPOCH` is deliberately not used, since build shells export it), so those
  dates stay in the future.
- `diagnose/<case>.txt|json` are the reports over the fixture graphs and the crafted graphs in
  `diagnose/*.json`. Every run passes `--extract-path diagnose/scan_probe.py.txt`, a crafted file,
  so no golden contains Graphify's own extractor source (§6.7).
- `prs/<scenario>.scenario.json` are the fake `gh`'s inputs: the `pr list` items, with `updatedAt`
  written as `{"days_ago": N, "hours_ago": H}` and materialised as now minus that interval when
  served, the default branch and the changed files per PR. `prs/<case>.txt` are the outputs, with
  the worktree and repository paths replaced by `<WORKTREE>` and `<REPO>`; `prs/mcp.*.txt` are the
  MCP PR tool texts.

---

## 2. `save-result`

```text
graphify save-result --question Q (--answer A | --answer-file F) [--type T] [--nodes N ...]
                     [--outcome useful|dead_end|corrected] [--correction TEXT] [--memory-dir DIR]
```

### 2.1 Arguments

- **Rule.** The parser is argparse: `--question` is required (`--question ""` is accepted), a
  repeated option keeps the last value, `--opt=value` works, an unknown option is a usage error.
  Usage errors print the usage block and `graphify save-result: error: …` to stderr and exit 2.
- **Rule.** An answer is required: `--answer` with a non-empty value, or `--answer-file`.
  `--answer ""` without a file is the same usage error as no answer (exit 2).
- **Rule.** `--answer-file F` (undocumented in `--help`) reads `F` as UTF-8, strips leading and
  trailing whitespace, and replaces `--answer` when both are given. `-` is a file name, not stdin.
  A missing file is an unhandled error (traceback, exit 1).
- **Rule.** `--outcome` is validated, case-sensitively, against `useful`, `dead_end`, `corrected`
  (exit 2 otherwise).
- **Quirk.** `--type` is not validated: any text is written, case kept and not escaped (§2.4). The
  help names `query`, `path_query` and `explain`; the default is `query`.
- **Rule.** `--nodes` takes zero or more values; `--nodes` with none records nothing.
  `--nodes ""` records one empty name.
- **Rule.** `--correction` is recorded whenever it is non-empty, whatever the outcome (also with
  `useful`, and with no outcome); an empty correction is dropped.

### 2.2 Location and name

- **Rule.** The file goes to `--memory-dir` (default `graphify-out/memory`, relative to the cwd),
  created with its parents. If the directory path is an existing file the command fails
  (traceback, exit 1).
- **Rule.** Name: `query_<YYYYMMDD>_<HHMMSS>_<token>_<slug>.md`. The prefix is `query_` for every
  `--type`. The stamp is the current UTC time. The token is 8 lowercase hex digits that differ
  between two calls in the same second with identical arguments (fitted: no hash of the question,
  answer or date reproduced it; treat it as random).
- **Rule (slug).** Lowercase the question (Unicode lowercasing: `ẞ` becomes `ß`, `İ` becomes
  `i̇`), replace every character that is not a word character (letters, digits and `_` in the
  Unicode sense; punctuation, spaces, symbols, emoji and combining marks are not) by `_`, keep the
  first 50 characters, then strip `_` from both ends. Runs are not collapsed (`Hello, World!` →
  `hello__world`). The truncation happens before the strip: `1234…49!xyz` (49 digits) gives the 49
  digits, and a question of only punctuation gives an empty slug (`query_…_<token>_.md`).
- **Rule.** stdout is `Saved to <path>\n`, where `<path>` is the memory directory as given joined
  with the file name. Exit 0.

### 2.3 File format

```text
---
type: "<type>"
date: "<ISO 8601 UTC with microseconds and +00:00>"
question: "<question>"
contributor: "graphify"
[outcome: "<outcome>"]
[correction: "<correction>"]
[source_nodes: ["<n1>", "<n2>", …]]
---

# Q: <question>

## Answer

<answer>
[
## Outcome

[- Signal: <outcome>]
[- Correction: <correction>]]
[
## Source Nodes

- <n1>
- <n2>]
```

- **Rule.** Keys appear in exactly this order; optional keys are absent when their value is. The
  date looks like `2026-10-15T15:56:41.434286+00:00`.
- **Rule.** The body repeats the raw (unescaped) values. The `## Outcome` section appears when
  there is an outcome or a correction; `## Source Nodes` when at least one name was given (an
  empty name gives the line `- `).
- **Rule.** The file has no trailing newline: it ends with the last character of the answer or of
  the last section line. The answer is written as given (inner and surrounding whitespace kept)
  except for the strip of §2.1 when it comes from a file.

### 2.4 Escaping in the front matter

- **Rule.** `question`, `correction` and every `source_nodes` item are written between `"` with:
  `\` → `\\`, `"` → `\"`, newline → `\n`, CR → `\r`, tab → `\t`, any other C0 control character
  and DEL → `\xNN` (two lowercase hex digits), U+2028 → `\L`, U+2029 → `\P`. Everything else is
  written as is, non-ASCII and C1 characters (U+0085, U+00A0) included.
- **Rule.** `source_nodes` is a one-line list: `[` + items joined by `, ` + `]`.
- **Quirk.** `type` is interpolated without escaping: `--type 'a "b"'` produces `type: "a "b""`.

---

## 3. The memory directory and the other commands

- **Rule.** `graphify update` indexes every file under `graphify-out/memory/` (recursively) like
  any other file of the corpus, although `graphify-out` is otherwise skipped: Markdown results
  become page and heading nodes (`graphify_out_memory_query_…`, headings `Q: …`, `Answer`,
  `Outcome`, `Source Nodes`, with the front matter as `frontmatter`, values HTML-escaped as
  `graphify-semantic.md` §3.2 says), and a code file there is extracted as code. Ignore rules do
  not apply inside it: with `graphify-out/` in `.gitignore` and a `.graphifyignore` inside the
  memory directory naming a file, that file is still indexed. Other `graphify-out` subdirectories
  (`reflections/`, anything else) and loose files are not.
- **Rule.** The semantic `extract` counts the same files as documents (`found 6 code, 2 docs` with
  one README and one saved result, `1 docs` without the result), so they are sent to the model.
- **Rule.** A memory directory outside `graphify-out/memory` (`--memory-dir notes`) is ordinary
  corpus content, indexed or ignored by the usual rules.
- **Rule.** `extract --code-only` on a fresh tree does not index the Markdown results.
- **Rule.** Nothing else reads the memory directory: `query`, `path`, `explain` and the MCP server
  answer only from the graph (results reach them only after an `update` folded them in), the MCP
  tool and resource lists have no memory entries, and `reflect` is the only reader of the result
  files' front matter.

---

## 4. `reflect`

```text
graphify reflect [--memory-dir DIR] [--out FILE] [--graph PATH] [--analysis PATH]
                 [--labels PATH] [--half-life-days N] [--min-corroboration N] [--if-stale]
```

### 4.1 Arguments and defaults

- **Rule.** argparse: unknown options and bad values exit 2 with the usage block
  (`--half-life-days` takes a float, `--min-corroboration` an integer). `--if-stale` is
  undocumented in the top-level help.
- **Rule.** Defaults: memory `graphify-out/memory`, output `graphify-out/reflections/LESSONS.md`
  (parents created), half-life 30 days, corroboration 2. `--graph` defaults to
  `graphify-out/graph.json` in the cwd when that file exists (whatever `--memory-dir` is).
  `--analysis` and `--labels` default to `.graphify_analysis.json` and `.graphify_labels.json` in
  the graph's directory.
- **Rule.** A missing or unreadable memory directory, a memory path that is a file, a missing or
  unparseable graph: no error, the run proceeds without that input (0 results, or no graph).
- **Quirk.** An analysis whose `communities` is a list crashes (traceback, exit 1).
- **Rule.** stdout: `Reflected <n> memories (<u> useful, <d> dead ends, <c> corrected) -> <out>`,
  `<out>` as given. Exit 0. Nothing on stderr.

### 4.2 Which files are results

- **Rule.** Only `*.md` directly in the memory directory, extension case-sensitive (`x.MD` and
  `x.Md` are skipped), no subdirectories, no other extensions.
- **Rule.** A file is a result when its text starts with `---` (a UTF-8 BOM before it, a blank
  first line or an empty file means it is not). The front matter is the lines after that up to the
  next line that is exactly `---` (CR before the newline tolerated); with no closing line it runs
  to the end of the file. Lines after the closing `---` are never read, the body included.
- **Rule.** `<n>` in the stdout line and the summary counts every result, marked or not.

### 4.3 Reading a result

- **Rule.** A front-matter line is read when it starts at column 0 with a known key, optional
  spaces, `:`, optional spaces and a value; trailing whitespace is ignored. Indented lines are
  ignored. `key:"v"` works.
- **Rule.** `date`, `question`, `outcome` and `correction` are read only when the value is a
  double-quoted string; `outcome: useful` and `'useful'` count as absent. Escapes are decoded:
  `\\`, `\"`, `\n`, `\r`, `\t`, `\0`, `\xNN`, `\uNNNN`, `\L` (U+2028), `\P` (U+2029); any other
  backslash pair (`\N`, `\e`, `\q`) is kept as written. A missing question is `""`.
- **Rule.** `outcome` must be exactly `useful`, `dead_end` or `corrected` (lowercase); anything
  else makes the result unmarked.
- **Rule.** `source_nodes` must be a JSON array on one line (`["a", "b"]`) with nothing after the
  closing bracket, where the strings may also use the `\xNN`, `\L` and `\P` escapes of §2.4
  (`"x\x41y"` is `xAy`); anything else (`[A, B]`, a YAML block list, a trailing comment) gives no
  names without making the result invalid. Non-string items are dropped; `""` is kept. Within one result,
  repeated names count once and keep their first position.
- **Rule.** `date` is parsed as ISO 8601 (`2026-01-04T00:00:00+07:00`, `…Z`, a date alone, a time
  with microseconds). A naive time is taken as UTC (whatever `TZ` is). An absent, empty or
  unparseable date (`garbage`, an unquoted value) gives the result weight 1 (§4.4); for sorting and
  display the raw string is used (`""` when absent).

### 4.4 Weight

- **Rule.** A result's weight is `0.5 ** (age_days / half_life)` where `age_days` is (now − date)
  in days, as a float, clamped at 0, so a result dated in the future weighs 1. Undated results
  weigh 1. A half-life that is 0, negative, infinite or NaN disables decay (every weight 1).
- **Rule (consequence).** Because weights decay exponentially from the same now, the ratio of any
  two weights depends only on the dates, so every ordering below is independent of the day
  `reflect` runs, except through the clamp and through underflow (a half-life of 1e-7 days makes
  every past weight 0.0, and ties fall back to the names).
- **Rule.** Comparisons round to nine decimal places (fitted on a sweep of near-ties at half-lives
  1000, 30 and 3: differences below 5·10⁻¹⁰ are ties, 5.3·10⁻¹⁰ is not).

### 4.5 Aggregation

Per name (a `source_nodes` entry, after §4.6):

- `useful`: the results marked `useful` that cite it, their count `u` and summed weight `wu`;
- `negative`: the results marked `dead_end` or `corrected` that cite it, count `n`, weight `wn`.

- **Rule (contested).** A name with `u > 0` and `n > 0` is contested. Its lean is `round(wu − wn,
  9)`: positive leans useful, negative leans dead end (also when the negative results are
  corrections), zero is evenly split. Its latest date is the greatest raw date string among all
  its results, cut to 10 characters (`garbage-lo` from `garbage-long-string`); with only empty
  dates there is no latest date. Contested names are listed by lean, highest first, then by name.
- **Rule (preferred and tentative).** The other names with `u > 0` are preferred when `u ≥
  min_corroboration` (count of distinct results, not weight) and tentative otherwise. Each list is
  ordered by `round(wu, 9)` descending, then by name in code-point order (`B`, `C`, `_x`, `a`,
  `b`). `min_corroboration` 0 or 1 puts every such name in preferred.
- **Rule.** Names cited only by negative results produce no name entry.
- **Rule (dead ends).** One entry per distinct question among `dead_end` results (exact string:
  `Same` and `same ` are different questions): the result with the greatest raw date string wins
  (so `…T00:00:00+07:00` beats `…T20:00:00+00:00` of the previous day although it is earlier),
  ties going to the file that sorts last by name. Entries are ordered by raw date string
  ascending (oldest first, empty first, unparseable dates by their text), then question. Each
  lists the winner's names (after §4.6), possibly none.
- **Rule (corrections).** The same for `corrected` results, each showing its correction text
  (empty when there is none).
- **Rule.** The summary counts results by outcome; unmarked results count only there.

### 4.6 The graph: stale names

- **Rule.** With a graph, a name is kept only when it equals a node id or a node label exactly
  (case-sensitive: `userservice` does not match `UserService`). Dropped names disappear from every
  list; a dead end or correction whose names were all dropped is still listed, without names.
- **Rule.** Without a graph nothing is dropped.

### 4.7 Topics

Topics need both the graph and an analysis whose `communities` maps community ids to node ids.
With no analysis, an empty `communities` or no graph, there is no topic section.

- **Rule (community of a name).** The communities of a name are those of every node whose id or
  label equals it, taken from the analysis's member lists. When there are several, the name takes
  the community whose id string sorts first (`10` before `9`; `0` before `1`), whether it came
  from an id or a label match.
- **Rule (topic name).** `.graphify_labels.json` (`{"<id>": "<name>"}`) names the community, else
  `Community <id>`.
- **Rule (topic of a result).** Each result is assigned to one topic: the topic named by most of
  its distinct kept names (one vote per name), ties broken by the topic name in code-point order.
  A result with no name in any community goes to `Uncategorized`. Topics with the same name merge
  (a community labelled `Uncategorized` merges into it).
- **Rule.** Each topic repeats §4.5 over its own results only (corroboration counted within the
  topic). Topics are listed by name in code-point order, with `Uncategorized` last. A topic whose
  results are all unmarked, or whose only results lost their names, is listed with the
  "no outcomes" line of §4.8.

### 4.8 Output layout

Graphify writes (prose paraphrased here, not quoted; the golden files hold the bytes):

```text
# Lessons

_<one italic line: generated by reflect from <n> session memories in graphify-out/memory/, no model, verify before relying>_

## Summary

- <u> useful · <d> dead ends · <c> corrected · <x> unmarked

## Lessons

**<preferred heading>** — <≥min_corroboration explanation>.

- `<name>` (<u>× useful)

**<tentative heading>** — <fewer than min_corroboration>.

- `<name>` (<u>× useful)

**<contested heading>** — <explanation>.

- `<name>` — <u>× useful, <n>× dead end/corrected → <lean> (latest <date10>)

**<dead-end heading>** — <explanation>.

- "<question>" — `<n1>`, `<n2>`

**<corrections heading>** — <explanation>.

- "<question>" → <correction>

[## By topic

### <topic>

<the same blocks for the topic's results>
…]
```

- **Rule.** A block is omitted when empty; when every block is empty, a single "no marked outcomes
  yet" line stands in for them. The order is always preferred, tentative, contested, dead ends,
  corrections. The lean reads `recency leans **useful**`, `recency leans **dead end**` or `evenly
  split`; ` (latest …)` is omitted when there is no date. A dead end without names has no ` — …`
  part. The file ends with one newline.
- **Quirk.** Names and questions are printed raw inside backticks and quotes (a backtick, a quote
  or a newline in them breaks the Markdown).
- **Quirk.** The italic line always names `graphify-out/memory/`, whatever `--memory-dir` was.

### 4.9 `--if-stale`

- **Rule.** With `--if-stale`, when the output exists and is at least as new (mtime) as every
  result file and the graph, analysis and labels files in use, nothing is written and stdout is a
  one-line "already up to date … (skipped; omit --if-stale to force)" message; exit 0.
- **Quirk.** Deleting a result does not make the lessons stale (only file mtimes are compared, not
  the directory's).

### 4.10 Determinism

- **Rule.** The output depends only on the result files, the graph inputs and the options, and on
  the clock only through §4.4. Directory listing order never matters (every tie has a defined
  winner).

---

## 5. `prs` and the MCP PR tools

```text
graphify prs [<number>] [--base BRANCH] [--repo OWNER/REPO] [--triage]
```

### 5.1 Data sources

- **Rule.** `gh` is the only source. Without `--base` the default branch comes from
  `gh repo view [--repo-less OWNER/REPO] --json defaultBranchRef` (`repo view OWNER/REPO …` with
  `--repo`); when that call fails the base is `main`. Then
  `gh pr list --state open --limit 50 --json number,title,headRefName,baseRefName,author,isDraft,reviewDecision,statusCheckRollup,updatedAt [--repo OWNER/REPO]`.
  With `--base` the `repo view` call is skipped. At most 50 open PRs are ever seen.
- **Rule.** Every call is read-only. The dashboard makes exactly these two calls. The detail view
  (§5.7) additionally runs `gh pr diff <n> --name-only` for **every** listed PR, concurrently (call
  order varies), when a graph is present, and uses only the requested one.
- **Rule.** When `gh` is missing, not authenticated, fails, or prints something that is not a JSON
  array, stderr is `  Error: gh CLI not found or not authenticated. Run: gh auth login` and the
  exit code is 1.
- **Rule.** Worktrees come from git (`git worktree list`), from the cwd. Outside a repository there
  are none.
- **Rule.** The graph for impact is `graphify-out/graph.json` in the cwd.
- **Quirk.** An empty or unparseable `updatedAt` crashes (traceback, exit 1).
- **Quirk.** Unknown arguments (`--json`, `--bogus`, `triage`) are ignored; `--help` prints the
  generic `Run 'graphify --help' for full usage.`; the first all-digit argument (also `#201`) selects
  the detail view; `0`, a negative or an unknown number is "not found".

### 5.2 CI state

From `statusCheckRollup`; only items with `__typename` `CheckRun` are considered (`StatusContext`
items never change the state).

- **Rule.** `FAILURE` when any check has conclusion `FAILURE`, `CANCELLED`, `TIMED_OUT`,
  `ACTION_REQUIRED` or `STARTUP_FAILURE`; else `PENDING` when any check has status `IN_PROGRESS` or
  `QUEUED` and no conclusion; else `SUCCESS` when any check concluded `SUCCESS`; else `NONE`
  (`NEUTRAL`, `SKIPPED`, `STALE`, statuses `PENDING`/`WAITING`/`REQUESTED`, no checks, `null`).
  Matching is case-sensitive (`failure` is `NONE`).
- **Rule.** Glyphs: `✗` failure, `…` pending, `✓` success, `–` none.

### 5.3 Status and order

- **Rule.** A PR whose `baseRefName` is not the base is `WRONG-BASE`: hidden from the dashboard and
  counted in its summary. Otherwise the first that applies: `CI-FAIL` (CI failure), `CHANGES-REQ`
  (`reviewDecision` `CHANGES_REQUESTED`), `DRAFT`, `STALE` (age ≥ 14 days), `APPROVED`
  (`APPROVED`), `PENDING` (CI pending), `READY`. `REVIEW_REQUIRED` and an empty decision are
  `READY`.
- **Rule.** Age is whole days, floor of (now − `updatedAt`) (23 hours is 0, 13 days 23 hours is
  13); in the future it is negative.
- **Rule (order).** Rank `CI-FAIL`, `CHANGES-REQ`, `DRAFT`, `STALE`, `PENDING`, `APPROVED`,
  `READY` (note: `PENDING` ranks above `APPROVED` although `APPROVED` wins the status), then age
  in whole days (newest first), then `gh`'s order: within one day the hour does not count
  (fastapi, 2026-10-16: #14939 updated 07:15 stays before #14865 updated 08:28, both 80 days old,
  as `gh` listed them).

### 5.4 Dashboard

```text
<blank>
  graphify prs  ·  base: <base>  ·  <k> PRs
<blank>
     #  CI  STATUS         UPDATED   IMPACT                  TITLE
  ────  ──  ─────────────  ────────  ──────────────────────  ────────────────────────────────────────
  #<n padded to 6> <worktree mark>  <glyph>  <status padded to 13>  <age right-aligned in 8>   <impact padded to 22>  <title>
…
<blank>
  <count> ready · <count> approved · <count> pending CI · <count> CI failing · <count> changes requested · <count> draft · <count> stale · <count> wrong base
<blank>
```

- **Rule.** `<k>` counts the PRs on the base. The age column reads `today` for 0 or less, else
  `<d>d`. The worktree mark is `⬡` when the PR's head branch is checked out in a worktree of the
  repository (the main one included), else a space; a local branch that is not checked out does
  not count.
- **Rule.** Titles longer than 52 characters are cut to 51 plus `…`; drafts get ` [draft]` after
  the cut. Titles are otherwise raw.
- **Rule.** The summary lists the non-zero counts in the order shown.
- **Rule.** With no PR on the base the table is replaced by a one-line "no open PRs" message.
- **Quirk.** The header columns are not aligned with the rows (rows are shifted right by up to
  four columns).
- **Quirk.** The IMPACT column is `–` in every probe, graph present or not; the dashboard never
  fetches changed files.
- **Quirk.** With no PRs at all the summary line is two spaces.
- **Quirk.** A title with a newline breaks the row.
- **Rule.** `--triage` asks a model to rank PRs (it failed here for want of a client library). It
  is out of scope (no model calls).

### 5.5 Detail view (`prs <n>`)

```text
<blank>
  PR #<n>  ·  <status>
  <full title>
<blank>
  branch:  <head>  ->  <base of the PR>
  author:  <login, '?' when author is null>
  updated: <d>d ago
  CI:      <glyph> <FAILURE|PENDING|SUCCESS|NONE>
[  review: <reviewDecision>]
[  worktree: <real path of the worktree>]
[<blank>
  Graph impact:  <m> nodes / <c> communit(y|ies)
  communities: [<sorted ids>]
  files changed: <f>
    <file>            (the first 10)
[    … and <f-10> more]]
<blank>
```

- **Rule.** The PR is looked up among all listed PRs, wrong base included (status `WRONG-BASE`).
  Not found: stderr `  PR #<n> not found in open PRs.`, exit 1.
- **Rule.** Impact: the nodes of the graph whose `source_file` is one of the changed files, and the
  distinct `community` values among them, as a Python list of integers. The block is shown only
  when at least one node matched.
- **Quirk.** `updated:` says `0d ago` where the dashboard says `today`; `review:` is aligned one
  column short.

### 5.6 MCP PR tools

`graphify-mcp` (`graphify-query.md` §10) exposes the same data as `list_prs`, `get_pr_impact` and
`triage_prs` (input schemas in `graphify-query.md` §10.3), running `gh` from the server's cwd.

- **Rule (`list_prs`).** First line `Open PRs targeting <base>: <k>  (<w> on wrong base, not
  shown)`, then for each PR in dashboard order a blank line, `#<n> [<status>] CI=<ci>
  review=<decision or none> age=<d>d author=<login>` and two-space-indented full title. Ages are
  not clamped (`age=-3d`). `isError: false`.
- **Rule (`get_pr_impact`).** `gh pr view <n> --json
  title,headRefName,baseRefName,author,isDraft,reviewDecision,statusCheckRollup,updatedAt`, then
  `gh pr diff <n> --name-only`; the text is `PR #<n>: <title>`, `CI: <ci>  Review: <decision or
  none>`, `Base: <base>  Author: <login>`, a blank line, `Graph impact: <m> nodes across <c>
  communities`, `Communities touched: [<ids>]`, `Files changed (<f>):` and the first 20 files
  indented two spaces, then `  … and <f-20> more`. With no changed files: `PR #<n>: no changed
  files found (may require gh auth).` A failing `pr view`: `PR #<n> not found or gh not
  authenticated.` with `isError: true`.
- **Rule (`triage_prs`).** The PRs that are not stale, ordered by rank then number, each as `PR #<n>
  [<status>] CI=… review=… age=…d author=…` followed by `  blast_radius=<m> nodes / <c>
  communit(y|ies)` when nodes matched and `  worktree=<path>` when mapped, and `  title: <title>`;
  a two-line header gives the count and tells the model how to rank. With none: a one-line "no
  actionable PRs" text.
- **Rule.** Without `gh`: `isError: true` and the CLI's error text without its indentation (list,
  triage) or the not-found text (impact).

---

## 6. `diagnose multigraph`

```text
graphify diagnose multigraph [--graph PATH] [--json] [--max-examples N] [--directed|--undirected]
                             [--extract-path PATH]
```

The command reports how many edges of a graph or extraction JSON would collapse when it is loaded
into a NetworkX graph that keeps one edge per endpoint pair. It writes nothing.

### 6.1 Arguments and errors

- **Rule.** Hand-parsed. Errors go to stderr with exit 1: `error: --directed and --undirected are
  mutually exclusive`, `error: --max-examples requires an integer`, `error: --max-examples must be
  >= 0`, `error: unknown diagnose option <opt>`. `diagnose` without `multigraph` (or with another
  word) prints the usage line `Usage: graphify diagnose multigraph [--graph path] [--json]
  [--max-examples N] [--directed] [--undirected] [--extract-path path]` and exits 1.
- **Rule.** The default graph is `graphify-out/graph.json`; any extension is accepted. A missing
  or unparseable file: `error: Cannot parse <path>: <reason>. The file may be corrupted — re-run
  'graphify extract'.`, exit 1. A JSON value that is not an object: `error: diagnostic input must
  be a JSON object`, exit 1.
- **Rule.** `--max-examples` defaults to 5 (0 allowed).
- **Rule.** Graphify also prints its extraction-schema warning line on stderr for most inputs
  (`[graphify] Extraction warning (…)`), which does not change the report.

### 6.2 Input

- **Rule.** Nodes are `nodes` (anything that is not a list counts as none). Edges are `edges` when
  that key exists, else `links`; `edges` wins when both exist.
- **Rule.** Direction: `--directed`/`--undirected` force it; otherwise undirected only when the
  top-level `directed` is JSON `false` (`"false"`, `0`, `null`, a nested `graph.directed` and
  `multigraph` do not count). A raw extraction has no flag and is directed.

### 6.3 Counters

- **Rule.** `node_count`: object nodes with an `id` key, counted by the text of the id (`1` and
  `"1"` are one node). Endpoints are compared the same way (`str()` of the value).
- **Rule.** Every edge is one of: non-object (`non_object_edges`); missing endpoint (`source` or
  `target` absent, `null` or `""`: `missing_endpoint_edges`); dangling (an endpoint that is not a
  node id: `dangling_endpoint_edges`); valid (`valid_candidate_edges`). `raw_edge_count` is their
  total. Self-loops are valid and also counted in `self_loop_edges`.
- **Rule.** `exact_duplicate_edges`: object edges (valid or not) equal to an earlier one as JSON
  values (key order and `1.50`/`1.5` do not matter, `1` and `1.0` differ), counted beyond the first.
  Non-object edges are all one value for this count (`"x"`, `1`, `[]` and `null` give 3 duplicates),
  distinct from `{}`.
- **Rule.** `directed_unique_endpoint_pairs` and `undirected_unique_endpoint_pairs` count distinct
  `(source, target)` and `{source, target}` among valid edges; each `*_same_endpoint_collapsed_edges`
  is valid edges minus that.
- **Rule.** `same_endpoint_group_count` counts **directed** pairs with two or more valid edges,
  whatever the effective direction. `relation_variant_groups` counts those groups with more than one
  distinct relation; `source_file_variant_groups`, `source_location_variant_groups` and
  `context_variant_groups` count groups where some single relation appears with more than one
  distinct value of that field (a missing value is `""`; different relations with different files
  do not count).
- **Open.** `unverified_node_count` and `external_reference_edges` were 0 in every probe.

### 6.4 Post-build simulation

- **Rule.** The graph is built as Graphify's loader would: `DiGraph` when directed, `Graph`
  otherwise; nodes are the node ids, edges the valid ones collapsed per pair.
  `post_build_node_count` and `post_build_edge_count` are its sizes.
- **Rule.** The build fails when a node or an edge is not an object, or an edge endpoint is
  present but not a string (`null`, a number, `true`); then the type is `""`, both counts are
  `null`, and `post_build_error` holds `<ExceptionType>: <message>`.
- **Open.** On a per-file AST cache entry (ids prefixed `$graphify-root$`), dangling targets were
  added to the built graph (4 nodes from 2); on plain graphs they are not.

### 6.5 Examples

- **Rule.** The groups of §6.3, ordered by edge count descending, ties in order of first
  appearance, at most `--max-examples`. Each has `source`, `target`, `edge_count` and the sorted
  distinct `relations`, `source_files`, `source_locations` and `contexts` (`""` for missing).

### 6.6 Report

Text (stdout):

```text
[graphify] MultiDiGraph edge-collapse diagnostic
input: <path as given>
input_stage: <one-line description>
effective_directed: True|False
nodes: …
unverified_code_nodes: …
raw_edges: …
valid_candidate_edges: …
missing_endpoint_edges: …
dangling_endpoint_edges: …
external_reference_edges: …
self_loop_edges: …
exact_duplicate_edges: …
directed_unique_endpoint_pairs: …
directed_same_endpoint_collapsed_edges: …
undirected_unique_endpoint_pairs: …
undirected_same_endpoint_collapsed_edges: …
same_endpoint_group_count: …
relation_variant_groups: …
source_file_variant_groups: …
source_location_variant_groups: …
context_variant_groups: …
post_build_graph_type: Graph|DiGraph|<empty>
post_build_edges: <n or None>
producer_suppression_sites: <n>
[post_build_error: <error>]
[producer_suppression_error: <error>]
[producer_suppression_examples:
  - L<line> <name> arity=<n or unknown>      (the first 8)]
[examples:
  - <s> -> <t> edges=<k> relations=<py list> locations=<py list> contexts=<py list>]
note: <one-line caveat>
```

- **Rule.** `non_object_edges` appears only in JSON. The lists in example lines are Python
  `repr` lists (`['', "it's"]`).

JSON (`--json`, indent 2, key order as listed):

```text
{"schema_version": 1,
 "summary": {node_count, unverified_node_count, raw_edge_count, non_object_edges,
             missing_endpoint_edges, dangling_endpoint_edges, external_reference_edges,
             self_loop_edges, valid_candidate_edges, exact_duplicate_edges,
             directed_unique_endpoint_pairs, directed_same_endpoint_collapsed_edges,
             undirected_unique_endpoint_pairs, undirected_same_endpoint_collapsed_edges,
             same_endpoint_group_count, relation_variant_groups, source_file_variant_groups,
             source_location_variant_groups, context_variant_groups, post_build_graph_type,
             post_build_node_count, post_build_edge_count, post_build_error, input_path,
             effective_directed},
 "examples": [{source, target, edge_count, relations, source_files, source_locations, contexts}],
 "producer_suppression": {path, total_sites, sites: [{line, name, tuple_arity, sample}], error},
 "notes": [three caveat strings]}
```

### 6.7 Producer suppression scan

The scan reads a Python source file (by default Graphify's own extractor) and lists the places that
could drop duplicate items.

- **Rule.** A site is a line whose stripped text starts with an identifier beginning `seen_`
  (case-sensitive), followed by optional spaces and `:` or `=` (not `==`): `seen_x = set()`,
  `seen_x: set[str] = set()`, `seen_x=set()`, `seen_x = {}`, `seen_x = []`, indented or not.
  `seen = …`, `self.seen_x = …`, `x = seen_x = …`, `seen_x.add(…)`, comments and `SEEN_X` are not.
- **Rule.** `tuple_arity`: the number of comma-separated parts between the first `tuple[` and the
  next `]` in the line (so `tuple[str, tuple[str, str]]` gives 3, `tuple[str, ...]` 2); 0 without a
  lowercase `tuple[`. Text shows 0 as `unknown`. `sample` is the stripped line.
- **Rule.** `--extract-path PATH` scans that file. A missing file records `error: "file not
  found"` and 0 sites; a directory or a non-UTF-8 file makes the whole command fail with
  `error: <reason>` and exit 1.

---

## 7. Dijkstral differences

Dijkstral keeps the data, the ordering and the file formats other tools read, and words everything
else itself.

- **Locations.** The memory directory is `dijkstral-out/memory/`, lessons go to
  `dijkstral-out/reflections/LESSONS.md`, and the default graph is `dijkstral-out/graph.dij` or
  `graph.json`, else `graphify-out/graph.json` (`diagnose` reads JSON only). A Graphify memory
  directory is ordinary corpus content to dijkstral, as dijkstral's is to Graphify.
- **Memory as corpus.** `update` (and the code extraction of every build) walks
  `dijkstral-out/memory/` the way Graphify walks `graphify-out/memory/` (§3), but ignore files
  inside the memory directory are honoured (the project's own ignore rules, which usually ignore
  the output directory, are not applied to it). The semantic pass never sends saved results to a
  model, so a code corpus with saved results needs no backend: they hold the user's questions and
  answers.
- **save-result.** Same file name scheme, front matter and body, so either tool's `reflect` reads
  the other's results; `contributor` is `dijkstral`. `--type` is escaped like the other values
  (Q: §2.4). Errors are clap's (exit 2); a missing answer file or an unusable memory directory is
  an error message with exit 1, not a traceback. The slug uses Rust's `char::is_alphanumeric`,
  which differs from Python's word class only for some combining marks.
- **reflect.** Same inputs, parsing, weights, rounding, lists, ordering and topics. `LESSONS.md` has
  the same sections in the same order with dijkstral's own headings and wording, names its real
  memory directory, and escapes backticks and line breaks in names and questions so the Markdown
  stays intact. The stdout line is dijkstral's own. A list-shaped analysis is ignored instead of
  crashing. `--if-stale` also notices deleted results (the directory's mtime is compared too).
- **prs.** Same `gh` calls (dijkstral's detail view diffs only the requested PR), same CI states,
  statuses, ranks, ages, truncation, worktree mark, summary order and detail fields. The header
  and messages are dijkstral's own; columns are aligned under their headings; IMPACT stays `–`
  (as in Graphify); a missing or bad `updatedAt` sorts last and shows `?` instead of crashing;
  newlines in titles are shown as spaces; `--help` documents the command, unknown options are
  errors, `--triage` is not offered (no model calls).
- **MCP PR tools.** Listed in `tools/list` with Graphify's schemas and dijkstral's descriptions.
  Same line formats; the header lines are dijkstral's own wording; every failure sets
  `isError: true`.
- **diagnose multigraph.** Same counters, groups, examples, scan rules and JSON keys. The banner,
  `input_stage`, `note` and `notes` texts are dijkstral's own; dijkstral writes no schema warning
  on stderr; without `--extract-path` there is no scan (dijkstral has no Python extractor:
  `path` is `""`, 0 sites); a post-build failure reports dijkstral's own reason text; error
  messages name dijkstral; options are parsed by clap (exit 2 on usage errors).

---

## 8. Quirks summary and open questions

### 8.1 Quirks

| # | Quirk | Where |
|---|---|---|
| M1 | `--type` is neither validated nor escaped. | §2.1, §2.4 |
| M2 | `--answer-file -` is a file name; a missing file or a file as memory directory crashes. | §2.1, §2.2 |
| M3 | Dead ends and corrections keep the result with the greatest date *string*. | §4.5 |
| M4 | Names and questions are printed raw in `LESSONS.md`; the italic line always names `graphify-out/memory/`. | §4.8 |
| M5 | `--if-stale` ignores deleted results. | §4.9 |
| M6 | A list-shaped `communities` crashes `reflect`. | §4.1 |
| P1 | The dashboard header is misaligned and the IMPACT column is always `–`. | §5.4 |
| P2 | The detail view diffs every open PR to show one. | §5.1 |
| P3 | An empty or unparseable `updatedAt` crashes `prs`. | §5.1 |
| P4 | Unknown `prs` arguments are ignored; `#12` selects PR 12. | §5.1 |
| D1 | `same_endpoint_group_count` and the variant counts are directed even when the report is undirected. | §6.3 |
| D2 | Only a JSON `false` makes the input undirected. | §6.2 |

### 8.2 Open

1. How the save-result token is made (§2.2): no hash of the inputs matched; random.
2. What `unverified_node_count` and `external_reference_edges` count (§6.3).
3. Why dangling targets of a per-file AST cache entry enter the post-build graph (§6.4).
4. Which events fill the dashboard's IMPACT column (§5.4).
5. Escapes other than those listed in §4.3 (`\a`, `\b`, `\U…`) in result files.
6. The HTTP transport of the MCP PR tools and their `project_path` handling (only stdio from the
   repository root was probed).
