# Graphify cross-repo graphs: black-box specification

This document specifies Graphify's cross-repository commands: `merge-graphs`, the global graph
(`global add|remove|list|path`, `extract --global`) and `clone`. The reference is PyPI `graphifyy`
0.9.86. Everything here was derived by running that binary on scratch copies with `HOME` set to a
scratch directory and `GRAPHIFY_QUERY_LOG_DISABLE=1`, and by reading the files it wrote. Graphify's
source was not consulted. Where a rule is inferred from a pattern rather than read from an example,
the text says so.

Status labels, as in `graphify-format.md`:

- **Rule**: observed consistently. Reproduce it.
- **Quirk**: observed, but a defect or an accident of implementation. Each says whether dijkstral
  reproduces it (for byte parity) or not (listed in §7).
- **Open**: not resolved (§8).

## Contents

1. Fixtures and goldens
2. The graph model
3. `merge-graphs`
4. `extract --global`
5. The global graph
6. `clone`
7. Dijkstral's deliberate differences
8. Quirks and open questions

---

## 1. Fixtures and goldens

`tests/fixtures/multirepo/` holds:

| Path | Contents |
|---|---|
| `regen.py` | the case table and the crafted input graphs; rerunning it with Graphify rewrites everything below |
| `inputs/*.json` | crafted graphs (communities, externals, edge kinds, hyperedges, stubs) |
| `cases.json` | every case: `files` (workspace path → source under `tests/fixtures/`) and `steps` (`args`, optional `cwd`, `exit`, `outputs` → golden) |
| `goldens/*` | Graphify's outputs: merged graphs, global graphs and normalized manifests |

Real repository graphs are the clustered goldens `python`, `go` and `rust` from `tests/fixtures/`,
copied to `<repo>/graphify-out/graph.json`. Each case runs in a fresh workspace with `HOME`
inside it; `@global` in a step stands for `$HOME/.graphify` (Graphify) or `$DIJKSTRAL_HOME`
(dijkstral, set to `<workspace>/home/.dijkstral` so both tools see the same tag for a graph inside
it). Manifests are compared after replacing `added_at` values with `<added_at>` and the workspace
path with `<ws>`. Stdout is never compared: each tool's messages are its own.

```bash
GRAPHIFY=~/.cache/dijkstral-bench/tools/graphify-0.9.86/bin/graphify python3 -I tests/fixtures/multirepo/regen.py
```

`crates/cli/tests/multirepo_goldens.rs` runs every case against `dijkstral` and must match every
output byte for byte, except the cases in its `ALLOWED` list (§7). `crates/cli/tests/
multirepo_cli.rs` covers what the goldens cannot: graphs built by dijkstral, queries on merged and
global graphs, `extract --global` and `clone` against a local repository.

Repeated runs of every case were byte-identical apart from the manifest's `added_at`.

---

## 2. The graph model

Both commands load each input with NetworkX's node-link reader and build the result in a NetworkX
`Graph` (undirected, one edge per node pair). Their observable consequences:

- **Rule.** Input shapes: the clustered shape (`links`), the raw shape (`edges`), with or without
  `graph`, `directed` and `multigraph`. A directed input loads as a directed graph, a multigraph
  input as a multigraph (a link's `key` names its parallel edge and is dropped from its
  attributes; without one the next free integer is used). A non-multigraph input keeps a `key`
  attribute as data.
- **Rule.** A link endpoint missing from `nodes` becomes a node with no attributes, inserted when
  the link is read (source first). A node object without `id` is keyed by its array index (an
  integer). A node id repeated in `nodes` is one node whose attributes are updated by the later
  record (keys keep their first position).
- **Rule.** Within one undirected input, links between the same pair collapse to one edge: the
  later link updates the attributes (`dict.update`: existing keys keep their position, new keys go
  last) and its direction wins.
- **Rule.** A link's direction is its `source` → `target`, unless it carries `_src` and `_tgt`
  naming its two endpoints: then they give the direction and are dropped from its attributes (the
  global graph writes them, §5.2).
- **Rule.** Edges are taken from each input in NetworkX's `G.edges()` order, not file order: for
  each node in insertion order, its neighbours (successors, when directed) in the order they were
  first connected, skipping neighbours already visited in an undirected graph; parallel edges in
  key order. Fixture `merge_external_adjacency`: an input whose links are `x→q` then `p→x`, with
  nodes `p, q, x`, contributes `p→x` first.
- **Rule.** Adding a node that already exists updates its attributes; adding an edge between a
  pair that already has one updates that edge's attributes and takes the new direction.
- **Rule.** Output order: nodes in insertion order; links in `G.edges()` order of the result.
  Every node object is its attributes followed by `id`, every link its attributes followed by
  `source`, `target`.

---

## 3. `merge-graphs`

### 3.1 Command line and output

`graphify merge-graphs <g1> <g2> [...] [--out PATH] [--previous PATH]`

- **Rule.** At least two graphs; fewer prints a usage line and exits 1. Options may come before or
  after the graphs. `--out` defaults to `graphify-out/merged-graph.json` relative to the cwd; its
  parent directories are created.
- **Rule.** Stdout: when tags collided (§3.2), one note line listing every tag in input order;
  with `--previous`, one line with the number of nodes whose community was restored and the
  previous path; then one line with the number of graphs, merged node and edge counts, and one
  line with the output path as given. Exit 0.
- **Rule.** Nothing else is written: no clustering, no report, no analysis sidecar. The merged
  graph is a schema-1 node-link graph that `cluster-only --graph` can re-cluster.

### 3.2 Tags

- **Rule.** An input's tag is the name of its path's second parent, taken lexically from the path
  as given (not resolved): `pyrepo/graphify-out/graph.json` → `pyrepo`,
  `one/two/graph.json` → `one`, `five/graphify-out/sub/graph.json` → `graphify-out`. When that
  name is empty (`graph.json`, `beta/graph.json`, `graphify-out/graph.json` from inside the repo,
  an absolute path at the root) the tag is `repo`. `.graphify_root` is not consulted.
- **Rule.** When any two inputs share a tag, every input in a shared group whose path has a third
  parent becomes `<third>_<tag>` (`/…/X/a/proj/graphify-out/graph.json` → `a_proj`,
  `x/proj/graphify-out/graph.json` → `x_proj`); inputs whose path has none keep the tag
  (`proj/graphify-out/graph.json` → `proj`). Unshared tags never change.
- **Rule.** Then, in input order, the k-th input holding an already-used name gets `<name>-k`
  (`proj, proj-2, proj-3` for one path given three times; `a_proj, a_proj-2` for
  `X/a/proj` and `Y/a/proj`).
- **Quirk M1 (not reproduced).** The suffix only counts repeats of the same name, so it can
  produce a name another input already holds: `proj/…`, `proj-2/…`, `proj/…` are tagged
  `proj, proj-2, proj-2`, and the first `proj-2` repository's nodes merge with the third's
  (fixture `merge_tags_suffix_taken`). Dijkstral gives the next free suffix (`proj-3`).
- **Quirk M2 (reproduced).** `..` is a name: `../graphify-out/graph.json` is tagged `..`
  (fixture `merge_tags_dotdot`).

### 3.3 Nodes

- **Rule.** A node whose `external` attribute is truthy (`true`, `"yes"`, `1`) is shared: its id is
  kept as is, it gets no `repo` or `local_id`, and a later input's record of the same id updates
  its attributes (last wins, keys keep their first position). `type: "external"`, an empty
  `source_file` or `external: false` do not make a node external.
- **Rule.** Every other node's id becomes `<tag>::<id>`, with `str(id)` for non-string ids
  (`7` → `numid::7`). Its attributes are kept in order, then `repo` is set to the tag (replacing an
  existing value in place) and `local_id` is added as `str(id)` unless the node already has one
  (a merged graph merged again keeps its `local_id`, while its id gains a second prefix:
  `premerged::t::a`). `local_community` follows (§3.4). `id` is last.
- **Rule.** The same local id in two repositories gives two nodes; nothing else is deduplicated
  across inputs (contrast §5.3).

### 3.4 Communities

- **Rule.** Each input has an offset: one more than the largest integer `community` among the
  nodes merged so far (externals included, with the values they hold at that point), or 0 when
  there is none. Booleans count as integers (`true` = 1); floats and strings do not.
- **Rule.** With a non-zero offset, a repository node with an integer community gets
  `community + offset` (written as an integer) and `local_community` = the original value. With a
  zero offset (the first input, or any input after inputs without integer communities) nothing is
  rewritten and no `local_community` is added, so a `true` community stays `true`.
- **Rule.** External nodes are never shifted. A node without `community`, or with a float or
  string one, keeps it unchanged.
- **Quirk M3 (reproduced).** Unshifted externals share community ids with the first repository's
  communities, and a large external community (fixture `merge_offsets_bigext`: 50) leaves a gap
  in the next input's ids.

### 3.5 Edges, hyperedges and graph attributes

- **Rule.** Each input's edges are added in its `G.edges()` order (§2) between the namespaced
  endpoints; edges to externals connect to the shared node. Pairs collapse across inputs as in §2.
  Self-loops are kept.
- **Rule.** Hyperedges of an input come from `graph.hyperedges` when that key exists (even when
  empty), else from the top-level `hyperedges`; `null` is none. Entries that are not objects, or
  have no `id` (or a `null` one), are dropped. A kept entry has its `id` replaced in place by
  `<tag>::<id>` and each member of `nodes` that is a non-external node of the same input replaced
  by its namespaced id; other members (externals, unknown ids) are kept. Other keys are kept in
  order. Hyperedges are concatenated in input order.
- **Rule.** `graph` is the update of every input's `graph` object in input order (later values
  win, keys keep their first position), without `hyperedges`; when the merged hyperedge list is
  non-empty it is added as `graph.hyperedges` (last) as well as the top-level `hyperedges`.
  Without any input `graph` the result is `{}`. `schema_version` and `graphify_version` are
  whatever the inputs said.
- **Rule.** `directed` and `multigraph` are always `false`; `built_at_commit`, `input_tokens`
  and `output_tokens` are dropped.

### 3.6 `--previous`

- **Rule.** `--previous PATH` reads a previous merged graph and maps each node `id` with an
  integer `community` to it. After merging, every node whose id is in the map takes that community
  (in place; inferred: added after its other attributes when it had none); **every other node
  loses `community`**, externals
  included, so a later `cluster-only` assigns them. `local_community` is untouched.
- **Rule.** The restored count printed (§3.1) is the number of merged nodes found in the map.
- **Rule.** A missing file exits 1 with an error naming `--previous`; a file that is not JSON
  exits 1 with the parser's message.

### 3.7 Encoding

- **Rule.** Compact JSON (`separators=(",", ":")`), ASCII-escaped, Python float repr, no trailing
  newline, top-level keys `directed, multigraph, graph, nodes, links, hyperedges`: the same
  encoding as Graphify's `graph.json` (`graphify-format.md` §2.3). Arrays are not sorted.

### 3.8 Errors

- **Rule.** A missing input exits 1 with an error naming it; no output is written. An unknown
  option is treated as an input path and fails the same way.
- **Quirk M4 (not reproduced).** Invalid JSON, a top-level array, a graph without `nodes` or a
  link without `target` end in a Python traceback (exit 1). Dijkstral exits 1 with an error
  message.

### 3.9 Reading a merged graph

- **Rule.** `query`, `path`, `explain` and `god-nodes` work on merged and global graphs. A name
  shared by two repositories is ambiguous (exit 3); the full id `<tag>::<id>` resolves exactly,
  before any `path::symbol` interpretation; `pkg/service.py::UserService` still narrows by file.
- **Rule.** Their output is the same as on any graph (`graphify-query.md`), with `ID:` showing the
  namespaced id.

---

## 4. `extract --global`

- **Rule.** `extract <path> --global [--as TAG]` builds as usual, then adds the graph it wrote to
  the global graph exactly as `global add` (§5.3) would, printing one progress line between the
  extraction lines. The tag is `--as`, else the name of the resolved target directory (`.` inside
  `gox` → `gox`), also with `--out` (the target directory, not the output directory).
- **Rule.** An unchanged graph (same bytes as the tag's last add) is skipped with a note.
  `--as` without `--global` is ignored. `--no-cluster --global` adds the raw graph.
- **Rule.** `update` has no `--global` (`error: unknown update option`, exit 2).

---

## 5. The global graph

### 5.1 Location

- **Rule.** Graphify keeps `~/.graphify/global-graph.json` and `~/.graphify/global-manifest.json`
  (`$HOME` decides). `global path` prints the graph's path even when it does not exist yet; no
  command creates the directory until the first add.

### 5.2 File format

- **Rule.** `global-graph.json` is NetworkX's own node-link output, compact and ASCII-escaped with
  no trailing newline: top-level keys `directed` (`false`), `multigraph` (`false`), `graph`
  (`{}`), `nodes`, `links`, and **no** `hyperedges`. Nodes are attributes then `id`.
- **Rule.** Each link is its attributes, then `_src` and `_tgt` (the true direction, as ids), then
  `source` and `target` in `G.edges()` iteration order, which is often the reverse direction
  (fixture `global_direction`: `Alpha → Beta` is written `"source":"dirt::b","target":"dirt::a"`).
  Graphify's readers (`path`, `explain`, `query`, `merge-graphs`) follow `_src`/`_tgt`.
- **Rule.** Each add or remove reloads the file (links in file order) and writes it back; the
  edge sequence of an unchanged part is stable across rewrites.

### 5.3 `global add <graph.json> [--as TAG]`

- **Rule.** Default tag: the name of the resolved graph path's second parent
  (`r1/graphify-out/graph.json` → `r1`; `nest/flat/graph.json` → `nest`; `./graph.json` in
  `c8` → the parent of `c8`).
- **Rule.** The file's bytes are hashed (SHA-256, first 16 hex digits). When the tag is already in
  the manifest with the same hash, nothing is written; a note says so; exit 0. Otherwise:
  1. Every node whose `repo` equals the tag is removed with its edges (the count is reported as
     pruned). Externals are never pruned.
  2. A label map is built from the remaining nodes whose `source_file` is falsy (missing, `""`,
     `null`) and whose `label` is truthy: label → node, the last such node winning.
  3. Each input node (in order) with a falsy `source_file` and a truthy `label` found in the map
     joins that node: it is not added and the existing node's attributes are not changed. This
     covers stubs and externals alike (Go's stub `T` joins Python's `pyrepo::t`; `go_pkg_fmt`
     joins the existing `go_pkg_fmt`). Two such nodes in the same input are not joined to each
     other. Labels are compared exactly (`foo` does not join `Foo`).
  4. Every other node is added: externals under their own id (updating an existing one), others
     as `<tag>::<id>` with `repo` and `local_id` as in §3.3. Communities are **not** shifted and
     no `local_community` is written.
  5. Each input edge (in `G.edges()` order) connects the mapped endpoints, unless they are the
     same node: self-loops are dropped, including those created by step 3. Its attributes gain
     `_src` and `_tgt`; an existing edge between the pair is updated (§2).
  6. Input hyperedges and `graph` attributes are dropped.
- **Rule.** The manifest entry is replaced in place (or appended): `added_at` (UTC ISO-8601 with
  microseconds and `+00:00`), `source_path` (the resolved input path), `node_count` (nodes added in
  step 4, externals included), `edge_count` (the input's edge count after its own pair collapse,
  self-loops included), `source_hash`.
- **Rule.** When the tag pointed to another `source_path`, a warning naming both paths goes to
  stderr, before the unchanged check.
- **Rule.** Stdout: one line with the tag, the nodes added and pruned, and the global graph's path.
- **Rule.** A missing graph exits 1 with an error naming it; invalid JSON exits 1 with the
  parser's message.
- **Quirk G1 (reproduced).** Step 3 joins unrelated stubs that share a label across languages and
  repositories (`T`, `Builder`), merging their neighbourhoods.
- **Quirk G2 (reproduced).** When a repository is re-added or removed, the edges other
  repositories had into its joined stubs are removed with them and do not come back (fixture
  `global_replace`: `r2`'s reference to `Foo` disappears when `r1` is re-added).
- **Quirk G3 (reproduced).** A tag re-added from a new path with the same bytes is "unchanged":
  the manifest keeps the old `source_path`.

### 5.4 `global remove <tag>`

- **Rule.** A tag not in the manifest exits 1 with an error naming it. Otherwise the tag's nodes
  are removed as in §5.3 step 1, its manifest entry is deleted, both files are written, and one
  line reports the number of nodes removed. Externals stay even when no repository is left.

### 5.5 `global list` and `global path`

- **Rule.** `list` with no manifest (or no repositories): one line saying the global graph is
  empty and how to add a project. Otherwise a header line with the graph's path, then one line per
  repository in manifest order with its tag, `node_count` and the date part of `added_at`.
- **Rule.** `global` alone, an unknown subcommand, or `remove` without a tag print a usage line
  and exit 1.

### 5.6 Manifest

- **Rule.** `global-manifest.json` is `json.dumps(indent=2)` (ASCII-escaped, no trailing newline)
  of `{"version": 1, "repos": {<tag>: {added_at, source_path, node_count, edge_count,
  source_hash}}}`, tags in insertion order.

---

## 6. `clone`

### 6.1 Command line and output

`graphify clone <github-url> [--branch B] [--out DIR]`

- **Rule.** The first argument is the URL (an option given first is taken as the URL); extra
  arguments are ignored. Without a URL: usage line, exit 1.
- **Rule.** Destination: `--out DIR` as given (relative paths stay relative), else
  `~/.graphify/repos/<owner>/<repo>` with the owner and repository as written in the URL.
- **Rule.** A new destination is cloned shallowly (`--depth 1`, so only the cloned branch is
  fetched), with `--branch B` when given. The remote is the URL without a trailing `/` and `.git`,
  plus `.git` (`https://github.com/octocat/Hello-World` → `…/Hello-World.git`; the `git@` form keeps
  SSH). Stdout: a progress line with that URL and the destination, a ready line, then the
  destination alone as the last line (for `cd "$(graphify clone URL | tail -1)"`).
- **Rule.** An existing destination is pulled instead (`git pull`; with `--branch`, from
  `origin <branch>`) and reported as already cloned; a failed pull is a stderr warning and the
  command still prints the path and exits 0.
- **Rule.** A failed clone (unknown repository, unknown branch) exits 1 with an error followed by
  git's own stderr.

### 6.2 URL forms

- **Rule.** Accepted: `https://github.com/<o>/<r>`, `http://…`, `https://www.github.com/…`, each
  with optional `.git` and trailing `/`; `git@github.com:<o>/<r>[.git]`; `github.com/<o>/<r>`.
  Owner and repository letter case are kept.
- **Rule.** Rejected with an error naming the URL, exit 1, nothing cloned:
  `<o>/<r>`, other hosts, and extra path segments (`…/tree/master`).
- **Quirk C1 (not reproduced).** `github.com/<o>/<r>` is accepted but cloned as given, which git
  reads as a local path: the clone fails. Dijkstral clones it over https.
- **Quirk C2 (not reproduced).** A query string or fragment is accepted and passed to git, which
  fails. Dijkstral rejects such URLs up front (owner and repository must be `[A-Za-z0-9._-]+` and
  not `.`/`..`).
- **Quirk C3 (not reproduced).** Any existing destination counts as a clone: an empty directory or
  an unrelated directory is "pulled" (git fails, warning, exit 0, the path printed). Dijkstral
  clones into an empty directory and refuses a non-empty directory that is not a git clone
  (exit 1).
- **Quirk C4 (reproduced).** `--branch` on an existing clone pulls that branch into the checked-out
  branch rather than switching to it. Dijkstral pulls with `--ff-only`, so it never creates a
  merge commit and fails (with a warning) instead.
- **Quirk C5 (not reproduced).** git may try to prompt for credentials for a missing or private
  repository. Dijkstral sets `GIT_TERMINAL_PROMPT=0`.

---

## 7. Dijkstral's deliberate differences

| # | Graphify | Dijkstral |
|---|---|---|
| D1 | state under `~/.graphify/` | `$DIJKSTRAL_HOME` (default `~/.dijkstral`): `global-graph.json`, `global-manifest.json`, `repos/<owner>/<repo>`; tests set `DIJKSTRAL_HOME`, never touching a real home |
| D2 | `--out` default `graphify-out/merged-graph.json` | `dijkstral-out/merged-graph.json` |
| D3 | its own messages | dijkstral's own words; the files are what is compared |
| D4 | usage errors exit 1 | clap usage errors exit 2 (fewer than two graphs, missing values, unknown options, `--as` without `--global`) |
| D5 | Quirk M1: a suffix can repeat a tag | the next free `-N` (golden `merge_tags_suffix_taken` is in the test's `ALLOWED`) |
| D6 | Quirk M4: tracebacks on malformed input | an error message, exit 1 |
| D7 | `update` rejects `--global` | `update --global [--as]` adds the updated graph too |
| D8 | `clone` progress on stdout | progress on stderr; stdout is only the path |
| D9 | Quirks C1, C2, C3, C5 | https for scheme-less URLs, strict owner/repository names, empty or non-git destinations handled, no credential prompt; pulls are `--ff-only` |
| D10 | — | `DIJKSTRAL_CLONE_BASE=<base>` (test-only) clones `<base>/<owner>/<repo>.git` instead of GitHub, so tests use a local `file://` repository |
| D11 | a failed `--global` add during `extract` (not observed) | the build is kept, the error reported, exit 1 |

Everything else, including Quirks M2, M3, G1–G3 and C4, is reproduced so that merged and global
graphs are byte-identical to Graphify's for the same inputs.

---

## 8. Quirks and open questions

### 8.1 Quirks

| Id | § | Reproduced | Summary |
|---|---|---|---|
| M1 | 3.2 | no | tag suffix can repeat another input's tag |
| M2 | 3.2 | yes | `..` as a tag |
| M3 | 3.4 | yes | externals unshifted; large external communities leave gaps |
| M4 | 3.8 | no | tracebacks on malformed input |
| G1 | 5.3 | yes | label-joined stubs across repositories |
| G2 | 5.3 | yes | joined-stub edges lost on re-add or remove |
| G3 | 5.3 | yes | unchanged-by-hash keeps the old `source_path` |
| C1 | 6.2 | no | scheme-less URL cloned as a local path |
| C2 | 6.2 | no | query and fragment passed to git |
| C3 | 6.2 | no | any existing destination is "pulled" |
| C4 | 6.2 | yes (ff-only) | `--branch` on an existing clone does not switch |
| C5 | 6.2 | no | credential prompt |

### 8.2 Open

1. `global add` when the resolved graph path has no second parent name (a graph at `/graph.json`):
   dijkstral uses `repo`.
2. A hyperedge `id` of `""` or `0`: dijkstral drops only a missing or `null` id.
3. Id equality across JSON types (`1`, `1.0`, `true`, `"1"` are one key in Python): dijkstral keys
   strings and other values separately.
4. Concurrent `global add`s: no locking was observed; dijkstral writes each file atomically but
   does not lock either.
5. Whether `graph.hyperedges` keeps its position in `graph` when an input already had that key:
   dijkstral removes and re-appends it (indistinguishable in every probe, where it was last).
6. `global add` of a graph whose nodes already carry `local_id` (a merged graph): assumed to keep
   it, as `merge-graphs` does.
