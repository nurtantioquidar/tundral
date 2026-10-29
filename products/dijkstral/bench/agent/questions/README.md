# Key-fact question sets

Stage-7 agent-outcome benchmark, `keyfacts` suite: 20 questions each for fastapi (Python),
ripgrep (Rust) and kubernetes (Go), plus a held-out set of 8 each for okhttp and excalidraw
(below), at the commits pinned in `../../corpus.toml`. An agent answers each question with no
code graph, with Graphify's graph and with Dijkstral's; an answer is scored
by the key facts it names (plus a blind judge). This directory holds only the questions, their
ground truth and its provenance; the harness lives elsewhere in `bench/agent/`.

## Files

| Path | What |
|---|---|
| `<repo>.jsonl` | The tasks, one JSON object per line (generated; do not edit by hand) |
| `provenance/<repo>.json` | Per question: each fact typed `file`, `dir` or `symbol` with the files it must occur in, the expanded SCIP lookups, and the source ranges read (generated) |
| `tools/sources/<repo>.json` | Source of truth: the task fields plus `in` (files per symbol fact), `scip` (lookups) and `read` (line ranges with a note) |
| `tools/build.py` | `sources/` → `<repo>.jsonl` + `provenance/<repo>.json`, expanding every SCIP lookup |
| `tools/scip_refs.py` | Stdlib SCIP reader: `defs`, `refs` and `callers` of a symbol in the accuracy indexes |
| `check.py` | Validator (below) |

## Task contract

```json
{"id": "fastapi-k01", "suite": "keyfacts", "corpus_id": "fastapi",
 "corpus": "~/.cache/dijkstral-bench/corpus/fastapi", "semantic": false,
 "question": "...", "question_date": null,
 "kind": "locate | callers | impact | architecture | flow",
 "reference": "one or two sentence reference answer",
 "facts": [{"fact": "fastapi/routing.py", "aliases": ["routing.py"]}, {"fact": "get_request_handler", "aliases": []}],
 "min_facts": 3}
```

A fact is a repo-relative file path, a directory (a crate or Go package), or an identifier. An
alias is an accepted alternative spelling: for files, a shorter path suffix that resolves to
exactly one file at the pinned commit; for directories, the Cargo package name (optionally followed
by " crate") or a Go package directory name that is unique in the repository. `min_facts` is how
many facts a correct answer must name: all of them for most locate questions, a majority for
callers, impact and flow, where the list is the decisive core rather than an exhaustive one.

Scoring should match a fact or alias case-sensitively on identifier boundaries (no `[A-Za-z0-9_]`
immediately before or after; for paths, also not preceded by `/`), so that `get_openapi` is not
credited by `get_openapi_path` and `Preempt` is not credited by `DefaultPreemption`.

## How the sets were made

Spread per repository for fastapi, ripgrep and kubernetes: 5 locate, 5 callers, 4 impact, 3
architecture, 3 flow.

- **Questions** are what a developer new to the repository would ask, in plain language. None
  uses graph vocabulary (node, edge, community, god node, graph) or names either tool; `check.py`
  enforces this, treating "node" as domain vocabulary only for kubernetes, where it means a
  cluster Node. Most questions need references followed across files rather than one grep for the
  question's own words; kubernetes questions all cross package boundaries (scheduler plugins,
  queue, cache, preemption, apimachinery serializers and conversions, kubelet admission).
- **Callers and impact facts** come from the SCIP indexes built by `bench/accuracy/index.sh`
  (`$DIJKSTRAL_BENCH_HOME/scip/<repo>/parts/*.scip`: scip-python 0.6.6, rust-analyzer 0.3.3065,
  scip-go 0.2.7). `tools/scip_refs.py` decodes them directly and attributes each reference to its
  enclosing definition using the chains in the accuracy oracle
  (`$DIJKSTRAL_BENCH_HOME/accuracy/oracle/<repo>.json`, derived from the same indexes by
  `bench/accuracy/oracle.py`). For impact questions the facts are the direct reverse references
  (calls, type references, trait or interface implementations) and, where stated, one more hop.
  Provenance records the SCIP symbol, its definition, every reference occurrence (file: lines)
  and the enclosing definitions with the relation family.
- **Locate, architecture and flow facts** come from reading the source at the pinned commit;
  provenance lists the line ranges read, each with a note.
- **Every fact was re-checked against the source** at the pinned commit by `check.py`.
- **Independence from both tools**: no fact was taken from Graphify's or Dijkstral's graphs.
  `graphify-out/`, `dijkstral-out/`, `$DIJKSTRAL_BENCH_HOME/reference/` and
  `$DIJKSTRAL_BENCH_HOME/accuracy/dijkstral/` were not opened while writing the questions, so the
  set cannot be tuned to either tool. No model was called.

### Held-out set: okhttp and excalidraw

Added on 2026-10-29: 8 questions each for okhttp (Kotlin/Java) and excalidraw (TypeScript), 4
flow and 4 architecture per repository, at the commits pinned in `../../corpus.toml`. They are a
held-out evaluation set, written from the source code only by an author who did not see either
tool's output or code. No SCIP index was used (the sources carry no `scip` lookups): every fact
comes from reading the source at the pinned commit, and provenance lists the line ranges read.
One fact was changed in review, before any tool ran on the set: `register` (excalidraw-k05), a
common English word an answer could name by accident, became its file
`packages/excalidraw/actions/register.ts`. The set was committed before it was first scored;
`../context_eval.py --blind` reports it as aggregates only, and no tuning uses it.

### Second tuning set: erpnext, jq, serilog and alamofire

Added on 2026-10-29, after the blind set showed `context` covering far fewer of its facts than of
the first two repositories': 8 questions each for erpnext (Python and JavaScript), jq (C), serilog
(C#) and alamofire (Swift), 4 flow and 4 architecture per repository, at the commits pinned in
`../../corpus.toml`. It was written exactly as the blind set was (the same brief, from the source
code only, by authors who saw neither tool's output or code, without SCIP), in languages the
blind set does not use, so that `context` can be tuned on questions like the blind set's while
the blind set stays unseen. It was committed before any change was tuned on it;
`../context_eval.py` reports it per question as `tuning-2`.

### Scope notes

- **kubernetes**: the SCIP index covers `pkg/scheduler/...` and
  `staging/src/k8s.io/apimachinery/...` only (see `../../accuracy/README.md`), so every
  SCIP-backed kubernetes fact lies in those trees. `kubernetes-k05` also involves
  `pkg/kubelet/lifecycle/predicate.go`, outside the index; that reference was established by
  reading (its provenance says so) and its SCIP lookup records only the definition of
  `AdmissionCheck`.
- **"Outside tests"** qualifiers matter: ripgrep keeps unit tests inside source files and
  fastapi's `tests/` reference many internals, so callers questions exclude tests explicitly and
  the provenance still lists every reference, tests included.
- fastapi 0.143.0 includes features absent from older releases (built-in OpenTelemetry under
  `fastapi/telemetry/`, frontend routes, SSE); the questions describe the pinned code, not the
  documentation of older versions.

## Regenerate and validate

```bash
cd products/dijkstral
# prerequisites: corpus checkouts (bench/setup.sh), SCIP indexes and oracle (bench/accuracy/README.md)
python3 -I bench/agent/questions/tools/build.py            # all repos, or: build.py ripgrep
python3 -I bench/agent/questions/tools/scip_refs.py fastapi callers --in fastapi/ solve_dependencies
python3 -I bench/agent/questions/check.py
```

`check.py` loads every `<repo>.jsonl`, validates the schema (key order and types, ids
`<repo>-kNN`, kind, 2–5 facts, `min_facts` range, no repeated fact or alias, no graph or tool
wording), confirms the provenance matches the task and the pinned commit, confirms each file and
directory fact exists at the pinned commit (read from the corpus repository's object store with
`git show`, never the working tree) and that file aliases resolve uniquely, confirms each symbol
fact and its aliases occur as whole words in every file the provenance lists, checks that every
read range lies inside its file, and prints counts per repo and kind. It exits non-zero on any
problem. Stdlib only; needs `git` and the corpus checkouts.

Counts at the time of writing:

| repo | locate | callers | impact | architecture | flow | total |
|---|---|---|---|---|---|---|
| fastapi | 5 | 5 | 4 | 3 | 3 | 20 |
| ripgrep | 5 | 5 | 4 | 3 | 3 | 20 |
| kubernetes | 5 | 5 | 4 | 3 | 3 | 20 |
| okhttp | 0 | 0 | 0 | 4 | 4 | 8 |
| excalidraw | 0 | 0 | 0 | 4 | 4 | 8 |
| erpnext | 0 | 0 | 0 | 4 | 4 | 8 |
| jq | 0 | 0 | 0 | 4 | 4 | 8 |
| serilog | 0 | 0 | 0 | 4 | 4 | 8 |
| alamofire | 0 | 0 | 0 | 4 | 4 | 8 |
