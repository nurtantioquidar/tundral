# Graphify extra inputs and custom providers: black-box specification

This document specifies the inputs `graphify extract` can add besides the walked files
(`--cargo`, `--postgres DSN`, `--google-workspace`), the walk and worker flags `--no-gitignore`,
`--max-workers` and `--output`, Graphify's custom LLM providers (`graphify provider
list|show|add|remove` and `--backend <name>`), and the community-naming flags `label
--missing-only` and `cluster-only --backend/--model/--batch-size`. The reference is PyPI
`graphifyy` 0.9.86. Everything here was observed by running its binaries on scratch inputs; its
source, prompts and skill text were not consulted. The CLI checklist that maps every Graphify
command and flag to dijkstral is `graphify-cli-checklist.md`.

Status labels as in `graphify-format.md`: **Rule** (observed, reproduce), **Quirk** (observed
defect or accident; not reproduced unless stated), **Open** (unresolved, §9).

## Contents

1. Harness and goldens
2. `--cargo`
3. `--postgres DSN`
4. `--google-workspace`
5. `--no-gitignore`, `--max-workers`, `--output`, `GRAPHIFY_FORCE`
6. Custom providers
7. Community naming flags
8. dijkstral's choices (deliberate differences)
9. Open questions

## 1. Harness and goldens

- Binaries: `~/.cache/dijkstral-bench/tools/graphify-0.9.86/bin/graphify` (base), `…-semantic`
  (`leiden,mcp,openai,pdf,office` extras) and `…-postgres`, made for this spec with
  `uv venv --python 3.13 …/graphify-0.9.86-postgres` and
  `uv pip install "graphifyy[leiden,postgres]==0.9.86"` (psycopg 3.3.6; the extra also brings
  `tree-sitter-sql` 0.3.11). Every run used `GRAPHIFY_QUERY_LOG_DISABLE=1` in a scratch
  directory, and `HOME` and the `XDG_*` directories pointed at a scratch directory for the
  provider probes (Graphify keeps providers under `~`).
- PostgreSQL: `postgres:16` run with Opossum (`opossum up` on a scratch compose file, port
  published on 127.0.0.1), the schemas in `tests/fixtures/inputs/postgres/<db>.sql` loaded with
  `psql` inside the container, the container and image removed afterwards.
- Model calls: none. Provider probes pointed `--base-url` at a local fake OpenAI-compatible
  server that logged requests and answered with an empty extraction or with `Named <id>` for
  naming requests; keys were fake values in scratch environment variables.

| Golden (`tests/fixtures/inputs/`) | Made with |
|---|---|
| `cargo/` (fixture), `cargo.graphify.json`, `cargo.raw.graphify.json`, `cargo.plain.graphify.json` | `extract . --code-only --cargo` (clustered, `--no-cluster`) and without `--cargo` |
| `postgres/<db>.sql`, `<db>.graphify.json`, `<db>.raw.graphify.json` for `shop`, `probe2`, `probe3` | `extract <empty dir> --code-only --postgres postgresql://…/<db>` (clustered, `--no-cluster`) |
| `postgres/<db>.catalog.json` | dijkstral's catalog snapshot of the same databases (§3.1), the offline input of the tests |
| `providers/providers.json`, `list.stdout`, `list.empty.stdout`, `show.stdout` | `provider add local-llm …`, `provider add gateway … --pricing-input 2.5 --pricing-output 10`, `list`, `show gateway` |

Tests: `crates/core/tests/inputs_goldens.rs` (cargo and postgres graphs record for record,
communities exact), `crates/core/src/inputs/*` and `crates/semantic/src/{gws.rs,run/backend/
provider.rs}` unit tests, `crates/cli/tests/inputs_cli.rs` (every flag end to end with a scratch
`HOME`, a fake `gws` and a fake model server), and the `#[ignore]`d live catalog test in
`crates/cli/src/bin/dijkstral-live/pg.rs` (`DIJKSTRAL_PG_TEST_DSN`).

## 2. `--cargo`

`extract <path> --cargo` prints `introspecting Cargo workspace...` and `Cargo: N nodes, M edges`
after the AST extraction and adds a crate graph read from the manifests. `cargo` is not run
(identical output with no `cargo` on `PATH` and with a logging fake `cargo`).

- **C1 Rule (manifest).** The workspace manifest is `<root>/Cargo.toml`; when there is none, the
  first `*/Cargo.toml` or `*/*/Cargo.toml` in path order (`aa/deep/Cargo.toml` before
  `bb/Cargo.toml`; `.hidden/Cargo.toml` before `vis/Cargo.toml`), including directories that
  `.graphifyignore` excludes. Deeper manifests are not considered: with only `a/b/c/Cargo.toml`
  it prints `--cargo: no Cargo.toml at scan root, skipping crate edges` and `Cargo: 0 nodes, 0
  edges`. A root manifest wins over an uppercase directory that sorts first (`A/Cargo.toml`).
- **C2 Rule (crates).** The crates are the manifest's own `[package]` (when it has a `name`) and
  every `[workspace] members` entry, globs expanded (`crates/*`), resolved relative to the
  manifest's directory. Members without `[package] name`, missing directories and non-directories
  are skipped silently. Manifests outside the members (or deeper than the members say) are not
  crates even when the code pass made a package node for them. Members are read from disk whatever
  the ignore files say.
  - **Quirk C2a.** `[workspace] exclude` is ignored (`exclude = ["crates/skip"]` still yields
    `crate:skip`). dijkstral honours it, as cargo does.
  - **Quirk C2b.** A self-dependency (`omega = { path = "." }`) gives a self-loop
    `crate:omega → crate:omega`, which survives clustering. Cargo rejects such a manifest;
    dijkstral emits no self-loop.
- **C3 Rule (dependencies).** A dependency is an entry of the crate's `[dependencies]` table (inline
  or `[dependencies.x]` form; path, version-only and `workspace = true` alike) whose package name
  is another crate of the workspace. The package name is the entry's `package` key when present
  (`gamma-lib = { package = "gamma", … }` → `gamma`), else its key. `[dev-dependencies]`,
  `[build-dependencies]` and `[target.'cfg(…)'.dependencies]` do not count; external crates
  (`serde`) and members excluded from the workspace produce nothing; a dependency listed twice
  gives one edge.
- **C4 Rule (raw records).** Nodes `{"id":"crate:<name>","label":"<name>","source_file":"<member>/
  Cargo.toml","source_location":"L1"}` (no `file_type`, no `_origin`), sorted by crate name, after
  every code node; edges `{"source":"crate:<a>","target":"crate:<b>","relation":"crate_depends_on",
  "context":"cargo_dependency","weight":1.0,"confidence":"EXTRACTED","source_file":"<a's
  manifest>","source_location":"L1"}`, sorted by (dependent, dependency) name, after every code
  edge. The counts printed are these nodes and edges.
- **C5 Rule (clustered).** In the clustered graph each `crate:<name>` node is merged into the
  first node with the same label and `source_file` — the `Cargo.toml` package node of the code
  pass (`pkg_alpha`, or `a_cargo_toml_pkg_same` under an id collision; §14.2 of
  `graphify-format.md`). Edges follow the merge. A crate with no such node (its manifest excluded
  from the walk by `.graphifyignore`) stays `crate:<name>` and gets `file_type: "concept"`,
  `community` and `norm_label` like any node.
- **C6 Rule (clustered edges).** A re-pointed `crate_depends_on` edge whose pair already has a
  link leaves that link unchanged (the `depends_on` of the code pass keeps its relation, context
  and `_origin`); on a new pair it is added and gets `confidence_score: 1.0`. On the fixture this
  adds exactly one link, the renamed dependency `pkg_alpha → pkg_gamma`, which the code pass misses
  because it matches dependency keys only.
- **C7 Rule.** Without `--cargo` none of this happens; the code pass's `depends_on` edges
  (`graphify-format.md` §11.6) are unaffected by the flag.

## 3. `--postgres DSN`

`extract <path> --postgres DSN` prints `introspecting PostgreSQL schema...` and `PostgreSQL: N
nodes, M edges` and adds the database's schema. Without the `postgres` extra it exits 1 with
`error: psycopg is required for --postgres…` after scanning; `--postgres` with no value is ignored.
Connection failures exit 1 with `error: could not connect to PostgreSQL: <libpq message>`; a
malformed DSN (`nonsense`) is an uncaught exception, exit 1.

- **P1 Rule (objects).** Ordinary and partitioned tables (partitions included), views (not
  materialized views), and functions and procedures (not aggregates) of every schema except
  `pg_catalog` and `information_schema`. Columns, indexes, triggers and sequences are not
  represented.
- **P2 Rule (source).** Every node's `source_file` is `postgresql:/<host>/<db>` (one slash), taken
  from the DSN text, not from the connection: a URL's host and path, or the `host=`/`dbname=` pairs;
  `localhost` and `db` when absent, so an empty DSN that connects through the `PG*` variables is
  labelled `postgresql:/localhost/db`. User, password, port and options never appear. The database
  node is `{"id":slug(source_file),"label":"<db>","file_type":"code","source_file":…,
  "source_location":null}` (`postgresql_127_0_0_1_shop`).
- **P3 Rule (object nodes).** `id = <db id>_<slug(schema)>_<slug(name)>`, label
  `"<schema>"."<name>"` for tables and views and `"<schema>"."<name>"()` for functions,
  `file_type: "code"`, no `_origin`. Edges carry `confidence: "EXTRACTED"`, `weight: 1.0`, no
  `context`, no `_origin`: `contains` from the database node to each object, `references` from a
  table to the table each of its foreign keys targets (self-references kept), `reads_from` from a
  view or function to each relation its SQL selects from.
- **P4 Rule (locations).** Locations are lines of a DDL listing the objects are extracted from:
  one line per table in (schema, name) byte order; then each view, its definition
  (`pg_get_viewdef`) starting on the view's line; then one line per foreign key, ordered by
  (schema, table, constraint name); then each function, its line followed by the lines of its body
  (`prosrc`) and a closing line (a body `"\n…\n"` of k newlines takes k + 1 lines, a one-line
  body one line). A node and its `contains` edge carry the object's line, a `references` edge its
  key's line, and a `reads_from` edge the line of the relation in the body. Probed: shop
  (`order_totals` L12, its reads L14–15, keys L18–23, functions L24–32), probe2 (a 12-line CTE
  view, quoted mixed-case names), probe3 (DML bodies, unresolved names).
- **P5 Rule (overloads).** Overloads share one node and one `contains` edge, at the first
  overload's line (`add_one(integer)` L27; `add_one(bigint)` takes L28 but adds nothing).
- **P6 Rule (reads).** A relation is read when it is a `FROM` list item (comma-separated items
  included) or a `JOIN` target: in view definitions and in SQL and PL/pgSQL bodies (`SELECT … INTO
  n FROM t`, `INSERT … SELECT … FROM t`, `UPDATE … FROM t`). `DELETE FROM t`, `INSERT INTO`,
  `UPDATE t` targets, CTE names and subqueries are not reads. A relation read twice by one object
  folds into one link carrying the later line (`sys_view2`: `pg_class` at L7 and L10 → L10).
- **P7 Rule (resolution).** Qualified names resolve to that schema's table or view; unqualified
  names to `public`, else to a relation of that name in another schema (`t1` → `app.t1`).
  Unresolved names become stubs `{"id":slug(name as written),"label":<as written>,
  "file_type":"code","source_file":"","source_location":"","origin_file":<source_file>}`
  (`pg_class`, `pg_catalog_pg_proc`, `"App"."Odd"` → `app_odd`), one per name.
- **P8 Rule (output).** Raw (`--no-cluster`) output appends the database node, the objects in
  listing order (stubs at their first reference) and the edges in listing order; the clustered
  graph treats them like any other nodes and links (`norm_label`, `community`, `confidence_score:
  1.0`).
- **Quirk P9.** In `FROM ((orders o JOIN order_items i ON …) JOIN products p ON …)` (PostgreSQL's
  rendering of a three-way join) Graphify reads `orders` and `order_items` but not `products`.
  dijkstral reads all three (the shop golden's one `ALLOWED` entry).

### 3.1 dijkstral's catalog reader

The `dijkstral-live` helper (`crates/cli/src/bin/dijkstral-live/pg.rs`; `dijkstral` writes it the
DSN over stdin and reads the catalog back as JSON) connects with the pure-Rust `postgres` crate
(plain TCP; a server
that requires TLS fails with `error performing TLS handshake`), filling a missing host, port,
user, password and database from `PGHOST` (else `localhost`), `PGPORT`, `PGUSER` (else `USER`),
`PGPASSWORD` and `PGDATABASE`, and reads `pg_class` (`relkind` `r`/`p` and `v`, with
`pg_get_viewdef`), `pg_constraint` (`contype = 'f'`) and `pg_proc` (`prokind` `f`/`p`, `prosrc`),
excluding `pg_catalog`, `information_schema`, `pg_toast*` and `pg_temp_*`. The snapshot
(`core::inputs::postgres::Snapshot`: host, database, catalog; never credentials) is mapped to
graph records by `core::inputs::postgres`, which tokenises SQL itself (comments, strings,
dollar quotes, quoted identifiers; `FROM` inside a call such as `extract(year FROM d)` and `IS
DISTINCT FROM` are not reads). Connection errors name the host and database and libpq's reason,
never the DSN.

## 4. `--google-workspace`

Google Drive for desktop leaves shortcut files (`.gdoc`, `.gsheet`, `.gslides`): small JSON
objects with `url`, `doc_id`, `email`, `resource_key`. Without the flag they are never counted or
sent (`graphify-semantic.md` §11).

- **G1 Rule (shortcuts).** With the flag every shortcut under the root (any depth) whose JSON
  names a Drive file — `doc_id`, else the `/d/<id>/` of `url` — is exported; malformed files and
  other URLs are skipped silently. Without a `gws` executable on `PATH` nothing is exported and
  nothing is printed (`converted/` stays empty); a failing `gws` (exit 3, message on stderr) is
  also silent.
- **G2 Rule (formats).** Docs export as `text/markdown` (`.md`), Slides as `text/plain` (`.txt`),
  Sheets as `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet` (`.xlsx`), the
  spreadsheet then converted like an office file (needs the `office` extra; without it the sheet
  is dropped).
- **G3 Rule (invocation).** One run per shortcut, in path order, with the working directory
  `<out>/graphify-out/converted/`: `gws drive files export --params {"fileId": "<id>",
  "mimeType": "<mime>"} -o <temporary name>.<ext>` (the params are one argument, Python
  `json.dumps` spacing).
- **G4 Rule (converted file).** `<out>/graphify-out/converted/<stem>_<h8>.md`, `h8` the first
  8 hex digits of `sha256(relative path)` (as for office files): a front-matter block with
  `source_file` (the shortcut's absolute path), `source_type: "google_workspace"`,
  `google_file_id`, `google_export_mime_type`, `source_url` (`""` without a `url`) and, when the
  shortcut has an `email`, `google_account_hash` (first 12 hex digits of `sha256(email)` as
  written, `72497f475e4f` for `someone@example.com`), all values double-quoted; a blank line;
  `<!-- converted from Google Workspace shortcut: <file name> -->`; a blank line; the export
  (Markdown and text verbatim, a spreadsheet as `## Sheet: <name>` tables).
- **G5 Rule (dispatch).** The converted files count as documents (`found 1 code, 3 docs`) and are
  sent to the model by their converted path like office conversions. They are re-exported on every
  run, `--code-only` runs included (where they are then skipped as non-code).

## 5. `--no-gitignore`, `--max-workers`, `--output`, `GRAPHIFY_FORCE`

- **W1 Rule.** `--no-gitignore` stops reading `.gitignore` files (at every depth) and
  `.git/info/exclude`; `.graphifyignore` still applies. A `.graphifyignore` negation re-includes a
  gitignored path even without the flag (`!gen/` over `.gitignore: gen/`).
- **W2 Rule.** `--max-workers N` sets the AST worker count. `0` and negative values exit 2 with
  `error: --max-workers must be > 0 (got N)`, non-numbers exit 2. Output does not depend on it.
- **W3 Rule.** `--output DIR` is an alias of `--out DIR` (`<DIR>/graphify-out/`); with both, the
  last wins; `--output` with no value is ignored.
- **W4 Rule.** `GRAPHIFY_FORCE=1` is `--force` for `extract` (skip the manifest gate and cache
  reads, `graphify-semantic.md` §9.2) and `update` (write even if the rebuild has fewer nodes; no
  probe made the shrink guard refuse a code-only rebuild that lost a function and a file).

## 6. Custom providers

### 6.1 Registry

- **R1 Rule.** Providers live in `~/.graphify/providers.json`: a JSON object, `indent=2`, trailing
  newline, insertion order, each entry `{"base_url", "default_model", "env_key", "pricing":
  {"input": <float>, "output": <float>}, "temperature": 0}` (golden `providers/providers.json`).
  `provider list` on an empty registry creates `~/.graphify/`; removing the last provider leaves
  `{}`.
- **R2 Rule (commands).** `provider` alone prints a usage line, exit 0; an unknown subcommand
  prints it, exit 1. `list` prints `  <name>  (<base_url>)` per provider (stdout; golden
  `list.stdout`) or a one-line notice. `show <name>` prints `{"<name>": <entry>}` with `indent=2`
  (golden `show.stdout`), or a not-found message on stderr, exit 1. `remove <name>` confirms on
  stdout, or not found → stderr, exit 1. `add <name> --base-url URL --default-model MODEL
  --env-key VAR [--pricing-input USD] [--pricing-output USD]` confirms on stdout; a missing
  required flag exits 1; unknown flags (`--temperature`) are ignored; `temperature` is always 0.
- **R3 Rule (base URL).** `http`/`https` with a host only (`ftp://host`, `nohost` → exit 1).
  Plain `http` to a host other than `localhost`, `127.0.0.1` or `[::1]` (private addresses
  included) is accepted with a warning on stderr.
- **Quirk R3a.** The plaintext warning is repeated on stderr by every later `graphify` command for
  every such provider in the registry (it is printed when the file is loaded).
- **Quirk R3b.** Names are not validated (`"bad name"` is stored); an existing name is refused as
  "a built-in provider"; the `env_key` value is stored verbatim, so a pasted key
  (`sk-…`) is written to the file in plain text.
- **Quirk R3c.** A malformed registry is read as empty, and the next `add` overwrites it.
- **R5 Rule (reserved names).** Graphify's built-ins cannot be registered: `gemini`, `kimi`,
  `claude`, `openai`, `deepseek`, `ollama`, `bedrock`, `claude-cli`, `azure`. (`anthropic`,
  `codex`, `google`, `moonshot`, `groq`, … are accepted.)

### 6.2 Requests

- **R6 Rule.** `--backend <provider>` sends OpenAI Chat Completions to `<base_url>/chat/
  completions` with `Authorization: Bearer <value of $env_key>`, `model` = `--model` or
  `default_model`, `temperature: 0`, `max_completion_tokens` (8192 for extraction, `256 + 48·n`
  for naming), `stream: false`. Without the variable (or empty): `error: backend '<name>' requires
  <VAR> to be set.`, exit 1, before any request. An entry without `env_key` asks for
  `AWS_PROFILE or AWS_REGION` (quirk). The `pricing` values only feed the cost estimate printed
  after a run (`est. cost (~<name>)`, per million tokens).
- **R7 Rule.** An unknown `--backend` exits 1 listing the built-ins and the registered providers.

### 6.3 Selection

- **R4 Rule.** Without `--backend`, Graphify's built-in keys are tried first (an `OPENAI_API_KEY`
  wins over a provider's key); then the providers in registry order, the first whose `env_key` is
  set. `label` and `cluster-only` select the same way.

## 7. Community naming flags

### 7.1 `cluster-only`

- **N1 Rule.** `cluster-only` with a `.graphify_labels.json` next to the graph reuses its names and
  ignores `--backend` with a warning. Without one it names communities through `--backend` (with
  `--batch-size`: 5 communities at 2 per call → 3 requests), and without `--backend` through any
  backend it can find on its own — on the probe machine a local assistant CLI answered (one
  unintended model call during probing). `--no-label` keeps `Community N`; `--no-viz` skips
  `graph.html`.

### 7.2 `label --missing-only`

- **N2 Rule.** `label --missing-only` re-clusters, keeps every saved name in
  `.graphify_labels.json` that is not the placeholder `Community <id>`, and sends only the other
  communities (placeholders and ids missing from the file) in its naming requests (probe: saved
  `{"0":"Kept Zero","1":"Community 1",(2 missing),"3":…,"4":…}` → one request for `1` and `2`).
  With no labels file it names every community; with every community named it sends nothing.
  It also writes a dated backup of the graph and sidecars under `graphify-out/<date>/` (quirk).

## 8. dijkstral's choices (deliberate differences)

- `--cargo` and `--postgres` are build inputs (`core::inputs`): they enter the build digest, so a
  build with a changed schema or manifests, or without the flag, is never skipped as unchanged,
  and their records join the graph after resolution and before clustering. dijkstral's
  `--no-cluster` graph is its usual clustered shape without `community`, so crate nodes are
  merged there too (C5), where Graphify's raw output keeps `crate:*` separate.
- Cargo: `exclude` honoured (C2a) and no self-loop (C2b).
- Postgres: all join targets read (P9); plain TCP only; `~/.pgpass` and `service=` files are not
  read (§9).
- `--google-workspace` exports only when the semantic pass runs (not with `--code-only`, where
  nothing would be sent); converted files go to `dijkstral-out/converted/`; their `source_file` is
  the shortcut's relative path (the absolute path would put the user's directory layout into
  every request); a missing `gws` is reported once and a failing export per file (Graphify is
  silent). `gws` is found as `DIJKSTRAL_GWS`, else on `PATH`, and run without a shell.
- Providers: dijkstral's registry is `~/.dijkstral/providers.json` in Graphify's format; Graphify's
  `~/.graphify/providers.json` is also read (never written), dijkstral's entry winning a shared
  name, so `--backend <name>` works for providers registered with either tool. `list` marks
  Graphify's with `[graphify]`; `remove` of one of them is refused with a pointer to `graphify
  provider remove`. Names are restricted to letters, digits, `.`, `_`, `-`; reserved names are
  Graphify's built-ins plus dijkstral's backends and aliases (`codex`, `anthropic`, `google`,
  `moonshot`, `replay`, `handoff`, `skill`), compared case-insensitively. `--env-key` must be an
  environment variable name: anything else (a pasted key) is refused without being repeated
  (R3b). A malformed dijkstral registry is an error and is never overwritten (R3c). The plaintext
  warning is printed by `add` and when the provider is used, not by every command (R3a). Errors
  name the variable, never its value. Missing required flags and unknown flags are argument errors
  (exit 2). `pricing` is stored but not reported.
- Without `--backend`, dijkstral tries Codex, then the built-in keys, then providers (R4); an entry
  without `env_key` is refused with a hint instead of asking for AWS settings.
- `cluster-only` never calls a model unless `--backend` is given; with it, it is `label
  --missing-only` (saved names kept, the rest named), which also covers Graphify's reuse of a
  saved labels file. `--no-label` is the default and `--no-viz` is accepted (dijkstral's
  `cluster-only` writes no `graph.html`). `label --missing-only` writes no dated backup; communities
  the model leaves unnamed get their structural name.
- `--max-workers` is an alias of `--threads` (0 is an argument error, exit 2); `--output` of
  `--out`. `GRAPHIFY_FORCE` set to `1`, `true` or `yes` is `--force` for `extract` and `update`;
  `update` has no shrink guard to override.

## 9. Open questions

1. Root-manifest fallback order: path-component order and plain string order agree on every probe;
   paths with characters sorting below `/` were not probed (C1).
2. Postgres objects not probed: foreign tables, extension-owned functions and tables, functions in
   other languages than SQL and PL/pgSQL, objects whose names contain `"`, databases whose name
   contains `.`, IPv6 hosts and multi-host DSNs, an unqualified name present in two non-public
   schemas (P7), `DELETE … USING`, `generate_series(...)` in `FROM` (dijkstral treats a call as no
   relation).
3. TLS for `--postgres` (Graphify uses libpq and supports `sslmode=require`); `~/.pgpass`.
4. `gws` behaviour with a real account: export size limits, multi-tab sheets, Drive shortcuts that
   need `resource_key`, authentication prompts. Whether `google_account_hash` lower-cases the
   e-mail (it does not on the one probe with capitals).
5. What `update`'s shrink guard (`--force`) refuses on a code-only rebuild (W4).
6. Whether `cluster-only`'s automatic backend order equals `extract`'s (N1); it was not probed
   further because it makes real model calls.
