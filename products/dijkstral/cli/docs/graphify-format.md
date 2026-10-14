# Graphify `graph.json` format: black-box specification

This document specifies what `graphify extract <dir> --code-only` writes to `graph.json` for Python,
TypeScript/TSX, JavaScript/JSX, Rust, Go, Java, Kotlin and Scala, plus what JSON, TOML and shell files
contribute.
The reference is PyPI `graphifyy` 0.9.86. It was derived only by running that binary on inputs and
reading the outputs: the fixtures under `tests/fixtures/`, small probe repositories, and scratch
copies of the benchmark corpus (`~/.cache/dijkstral-bench/corpus/<repo>/graphify-out/graph.json`).
Graphify's source was not consulted. Every rule here is an observation. Where a rule was inferred
from a pattern rather than read directly from an example, the text says so.

Status labels used below:

- **Rule**: observed consistently. Reproduce it.
- **Quirk**: observed, but it is a defect or an accident of implementation. Do not reproduce it in
  dijkstral's own output unless byte parity with a specific Graphify graph is the goal. Graph-parity
  diffs should list it as an expected difference.
- **Open**: not resolved. The open questions are collected in §15.

## Contents

1. Fixtures and goldens
2. Output shapes and serialization
3. Discovery: which files are indexed
4. Node ids
5. Labels and `norm_label`
6. Node fields
7. Edge fields and relations
8. Build stage: from extraction to the clustered graph (one edge per pair)
9. Python
10. TypeScript / TSX and JavaScript / JSX
11. Rust
12. Go
13. Java
14. JSON, TOML, shell and markdown in a code-only build
15. Quirks summary and open questions
16. Addenda from implementation
17. Clustering, analysis and `GRAPH_REPORT.md`
18. C#
19. PHP
20. Ruby
21. Kotlin
22. Scala
23. Swift
24. C
25. C++
29. Elixir
30. Erlang
31. OCaml
32. Julia
38. Vue
39. Svelte
40. Astro
41. PowerShell
42. Visual Basic

---

## 1. Fixtures and goldens

| Fixture | Files | Raw golden | Clustered golden |
|---|---|---|---|
| `python/` | `main.py`, `pkg/{__init__,models,service,util}.py`, `tests/test_service.py` | `python.graphify.json` | `python.clustered.graphify.json` |
| `typescript/` | `src/models/{index,user,repo}.ts`, `src/service.ts`, `src/util.ts`, `src/ui/App.tsx`, `src/__tests__/service.test.ts` | `typescript.graphify.json` | `typescript.clustered.graphify.json` |
| `javascript/` | `lib/store.js`, `lib/format.mjs`, `lib/default.cjs`, `components/App.jsx`, `test/store.test.js` | `javascript.graphify.json` | `javascript.clustered.graphify.json` |
| `rust/` | `Cargo.toml`, `src/{lib,main,model}.rs`, `src/util/{mod,fmt}.rs`, `tests/it.rs` | `rust.graphify.json` | `rust.clustered.graphify.json` |
| `go/` | `go.mod`, `store/{store,helpers}.go`, `service/{service,service_test}.go`, `cmd/app/main.go` | `go.graphify.json` | `go.clustered.graphify.json` |
| `java/` | `src/main/java/com/shop/{model,service}/*.java`, `src/test/java/com/shop/service/UserServiceTest.java` | `java.graphify.json` | `java.clustered.graphify.json` |
| `mixed/` | `package.json`, `tsconfig.json`, `Cargo.toml`, `pyproject.toml`, `scripts/{build,common}.sh`, `README.md`, `app/main.py` | `mixed.graphify.json` | `mixed.clustered.graphify.json` |
| `csharp/` | `src/Shop/Models/{Entity,User,User.Validation,Values}.cs`, `src/Shop/Services/{IRepository,Extensions,UserService}.cs`, `tests/Shop.Tests/UserServiceTests.cs` | `csharp.graphify.json` | `csharp.clustered.graphify.json` |
| `php/` | `src/Models/{Model,Repository,Status,User}.php`, `src/Service/UserService.php`, `src/helpers.php`, `bin/app.php`, `tests/UserServiceTest.php` | `php.graphify.json` | `php.clustered.graphify.json` |
| `ruby/` | `lib/shop.rb`, `lib/shop/{util,service}.rb`, `lib/shop/models/{base,user}.rb`, `bin/shop` (ruby shebang), `tasks/release.rake`, `test/service_test.rb` | `ruby.graphify.json` | `ruby.clustered.graphify.json` |
| `kotlin/` | `build.gradle.kts`, `src/main/kotlin/Main.kt`, `src/main/kotlin/com/shop/{model,service,util}/*.kt`, `src/test/kotlin/com/shop/service/UserServiceTest.kt` | `kotlin.graphify.json` | `kotlin.clustered.graphify.json` |
| `scala/` | `build.sbt` (not classified), `src/main/scala/Main.scala`, `src/main/scala/shop/{model,service,util}/*.scala`, `src/test/scala/shop/service/UserServiceSuite.scala` | `scala.graphify.json` | `scala.clustered.graphify.json` |
| `swift/` | `Sources/App/main.swift`, `Sources/Shop/{Models/{Entity,User,Shape},Services/UserService,Extensions/User+Format}.swift`, `Tests/ShopTests/UserServiceTests.swift` | `swift.graphify.json` | `swift.clustered.graphify.json` |
| `c/` | `main.c`, `include/{shape,util}.h`, `src/{shape,util}.c`, `tests/test_shape.c` | `c.graphify.json` | `c.clustered.graphify.json` |
| `cpp/` | `include/geo/{shape.hpp,point.h}`, `src/{shape.cpp,main.cc,legacy.cxx,kernels.cu,kernels.h}`, `tests/shape_test.cc` | `cpp.graphify.json` | `cpp.clustered.graphify.json` |
| `elixir/` | `mix.exs`, `config/config.exs`, `lib/shop.ex`, `lib/shop/{user,repo,util,format}.ex`, `priv/repo/seeds.exs`, `test/shop_test.exs` | `elixir.graphify.json` | `elixir.clustered.graphify.json` |
| `erlang/` | `src/{shop,shop_util}.erl`, `include/shop.hrl`, `test/shop_tests.erl`, `tools/{release,versions}.py` | `erlang.graphify.json` | `erlang.clustered.graphify.json` |
| `ocaml/` | `lib/shop.{ml,mli}`, `bin/main.ml`, `test/test_shop.ml`, `tools/{release,versions}.py` | `ocaml.graphify.json` | `ocaml.clustered.graphify.json` |
| `julia/` | `src/{Shop,models,service}.jl`, `test/runtests.jl` | `julia.graphify.json` | `julia.clustered.graphify.json` |
| `vue/` | `src/{main,store,types}.ts`, `src/utils/format.ts`, `src/composables/useFilter.ts`, `src/App.vue`, `src/components/{TodoHeader,TodoList,TodoItem,EmptyState,LegacyCounter}.vue`, `src/__tests__/format.spec.ts` | `vue.graphify.json` | `vue.clustered.graphify.json` |
| `svelte/` | `src/lib/{stores.ts,format.js,Cart.svelte,CartRow.svelte,Badge.svelte}`, `src/routes/{+page.svelte,+page.ts}` | `svelte.graphify.json` | `svelte.clustered.graphify.json` |
| `astro/` | `src/utils/posts.ts`, `src/layouts/Base.astro`, `src/components/{Header,PostCard}.astro`, `src/components/Like.tsx`, `src/pages/{index,about}.astro` | `astro.graphify.json` | `astro.clustered.graphify.json` |
| `powershell/` | `build.ps1`, `scripts/Common.ps1`, `Modules/Shop/{Shop.psd1,Shop.psm1}`, `Modules/Shop/Private/Pricing.psm1`, `tests/Shop.Tests.ps1` | `powershell.graphify.json` | `powershell.clustered.graphify.json` |
| `vb/` | `Shop/Program.vb`, `Shop/Models/{Entity,User}.vb`, `Shop/Services/UserService.vb` | `vb.graphify.json` (empty) | none (Graphify stops: "graph is empty") |

How the goldens were produced (from `tests/fixtures/`):

```bash
export GRAPHIFY_QUERY_LOG_DISABLE=1
cp -R <lang>/. "$SCRATCH/<lang>/" && cd "$SCRATCH/<lang>"
graphify extract . --code-only --no-cluster   # -> <lang>.graphify.json
# fresh copy, then:
graphify extract . --code-only                # -> <lang>.clustered.graphify.json
python3 -c 'import json,sys; g=json.load(open(sys.argv[1])); open(sys.argv[2],"w").write(json.dumps(g, indent=1, sort_keys=False)+"\n")' graphify-out/graph.json <golden>
```

The goldens are re-indented for review (`indent=1`, keys in Graphify's order, a trailing newline
added). Graphify's own bytes differ only in whitespace and in the `ensure_ascii` escaping described
in §2. The fixtures are not git repositories, so `built_at_commit` is absent from every clustered
golden.

Repeated cold runs of every fixture were byte-identical. For the TS/JS fixtures a warm rerun (with
`graphify-out/cache` present) also matched, but warm runs can lose dynamic imports (§8.4).

How to compare against the goldens:

- **Clustered** goldens are the compatibility target. Compare node and link **sets** after
  dropping `community` (Leiden comes later in the delivery plan). Also check the array order (§2.2)
  and the key order (§2.3) once community assignment exists.
- **Raw** goldens record what extraction produced before the build stage. They are the best
  oracle for per-file extraction, but they are lossy (§2.4), so compare them as sets.

---

## 2. Output shapes and serialization

### 2.1 Three shapes of `graph.json`

| Command | Top-level keys, in order | Edge array |
|---|---|---|
| `extract --no-cluster` (raw) | `nodes, edges, hyperedges, input_tokens, output_tokens` | `edges` |
| `extract` (clustered, default) | `directed, multigraph, graph, nodes, links, hyperedges[, built_at_commit]` | `links` |
| `update --no-cluster` | `input_tokens, output_tokens, nodes, links, hyperedges, directed` | `links` |
| `update`, `cluster-only` (clustered) | same as clustered `extract` | `links` |

**Rule.** The clustered shape is NetworkX node-link data and is the compatibility contract:

```json
{"directed":false,"multigraph":false,"graph":{"schema_version":1,"graphify_version":"0.9.86"},
 "nodes":[...],"links":[...],"hyperedges":[],"built_at_commit":"e89fff89ac9af12e8d4ce9d5fd07beb408ca730f"}
```

- `directed` is `false`, `multigraph` is `false`, and `graph` is exactly
  `{"schema_version":1,"graphify_version":"0.9.86"}`.
- `hyperedges` is always `[]` in code-only builds.
- `built_at_commit` is present **only** when the scan root is inside a git work tree with at least
  one commit. Its value is `git rev-parse HEAD` of the enclosing repository, even when the scan root
  is a subdirectory of it. When the root is not in a repository, or the repository has no commits,
  the key is **absent**, not `null`. Raw output never has the key.
- In raw output, `input_tokens` and `output_tokens` are `0` in code-only builds.
- `update --no-cluster` writes raw-style node and edge objects in insertion order, with neither
  `confidence_score` nor `community`.

### 2.2 Clustered array order

**Rule.** `nodes` and `links` are each sorted ascending by the string
`json.dumps(obj, sort_keys=True, ensure_ascii=False)`. Here `obj` is the final node or link object,
including `community`, `norm_label` and `confidence_score`.

- This held for every array in all six corpus graphs and in every golden.
- The variant with `ensure_ascii=True` fails on the graphify and fastapi corpora, because escaped
  non-ASCII characters sort differently.
- Separator choice (`", "`/`": "` versus `","`/`":"`) made no observable difference. Use Python's
  default separators for the sort key.

Consequences: nodes whose first sorted key is `_callable` come first; among those, classes come
before functions, because `_callable_class` sorts before `_origin`. Then come the remaining nodes
in order of `_origin`, `community`, `file_type`, `id`, and so on. Externals have no `_origin`, so
they sort by `community`. Links sort by `_origin`, `confidence`, `confidence_score`, `context`, ….
In other words, **community ids determine the order**, so exact array order can only be matched
once clustering matches.

### 2.3 Key order and formatting

**Rule (clustered).**

- Node keys are `id`, `label`, then every other key in sorted order:
  `{"id":…,"label":…,"_callable":…,"_callable_class":…,"_origin":…,"community":…,"community_name":…,"external":…,"file_type":…,"metadata":…,"norm_label":…,"source_file":…,"source_location":…,"type":…}`.
- Link keys are `source`, `target`, `relation`, then the rest sorted:
  `_origin, confidence, confidence_score, context, deferred, metadata, source_file, source_location, type_only, weight`.
- Formatting is compact (`separators=(",", ":")`) with `ensure_ascii=True`, so `—` is written as
  `—`. There is **no trailing newline**. Re-dumping a corpus graph with
  `json.dumps(g, separators=(",",":"), ensure_ascii=True)` reproduced all six files byte for byte.
- Raw output uses the same compact, ASCII-escaped formatting with no newline, but keeps insertion
  order (§2.4).

### 2.4 Raw (`--no-cluster`) output specifics

- **Node order.**
  - File nodes are processed in byte-sorted relative-path order (`B/c.py` before `a.py`).
  - Within a file: the file node, then definitions in source pre-order. An unresolved-reference stub
    is inserted at the point of first reference (`t` sits between `pkg_models_repo_add` and
    `pkg_models_repo_get`). Python rationale nodes come after the symbols: docstrings first, then
    comments.
  - External nodes are appended after all files, in first-seen order.
- **Edge order.** Per-file edges in file order, then the cross-file resolution block: `re_exports`,
  symbol-level `imports`, cross-file `calls`, `uses`, and so on.
- **Raw key order** follows insertion and varies by edge kind. For example, Python `calls` edges are
  `source,target,relation,context,confidence,[confidence_score],source_file,source_location,weight,_origin`,
  while decorator `references` put `context` after `weight`. Do not use it as a contract.
- **Raw deduplicates edges by `(source, target, relation)` and keeps the FIRST occurrence.** The
  build stage sees the undeduplicated list and keeps the LAST (§8.2). Raw output therefore hides
  some edges that decide the clustered result. Example from the `dup` probe:
  `def f(a: K, b: K) -> K` gives raw `a_f -references-> a_k` with context `parameter_type`, while
  the clustered link has `return_type`. The per-file cache in
  `graphify-out/cache/ast/<version>/*.json` holds the undeduplicated per-file lists.
- **`confidence_score`.** Raw edges from the per-file pass have none. Edges from the cross-file pass
  carry `1.0` (EXTRACTED) or their inferred score (`0.85`, `0.95`).

### 2.5 Fields added by the clustered build

- `community` (int) on every node.
- `norm_label` on every node (§5.4).
- `confidence_score` on every link. When missing it is filled from `confidence`: `EXTRACTED` → `1.0`,
  `INFERRED` → `0.55`, `AMBIGUOUS` → `0.2`, anything else → `1.0`. This was verified by feeding a
  hand-written raw graph to `graphify cluster-only`. Existing scores are kept, which is how a merged
  edge ends up `EXTRACTED` with score `0.85` (§8.2).
- `community_name` is **absent** from a fresh `extract`. It is present on every node after `update`,
  `label` or `cluster-only` (`"Community N"` with `--no-label`; otherwise a hub-based name).
- `.graphify_analysis.json` (communities, cohesion, god nodes, surprises) is written next to the
  graph and is not part of `graph.json`. `extract` writes no `GRAPH_REPORT.md`; `update` and
  `cluster-only` do (§17).

---

## 3. Discovery: which files are indexed

The full probe tables are in §14 and in the mixed fixture notes. In summary:

- **Code extensions** (case-insensitive):
  - This spec: `.py .ts .tsx .mts .cts .js .jsx .mjs .cjs .rs .go .java .kt .kts .scala`, and `.cs` (§18).
  - Also counted as code but out of scope here: `.json` (allowlisted manifests only, §14.1),
    `Cargo.toml`/`pyproject.toml` (only those names, §14.2), `.sh .bash` (§14.3), and `.rb .c .h
    .cpp .cc .cxx .hpp .cu .cs .kt .kts .swift .php .scala .groovy .gradle .dart .lua .zig .ex .exs
    .erl .ml .jl .m .mm .v .sol .f90 .vb .vue .svelte .astro .ps1 .r .sql .tf`.
  - `.d.ts` and `.min.js` are ordinary code files.
  - Extensionless files with a python, sh, bash, zsh, node, ruby or perl shebang are code. Example:
    ripgrep's `benchsuite/benchsuite` becomes file node `benchsuite_benchsuite`.
- **Skipped in `--code-only`:**
  - Docs: `.md .mdx .rst .txt .html .yaml .yml`.
  - Papers and images: `.pdf`, `.png .svg`, and so on.
  - Not classified: `.css .scss .graphql .proto .xml .ini .cfg .lock .pyi .ipynb`, `Dockerfile`,
    `Makefile`, and so on.
- **Default-skipped directories, at any depth, matched by exact name:** `.git node_modules target
  dist build __pycache__ .venv venv site-packages .tox .nox .mypy_cache .pytest_cache .ruff_cache
  .cache .idea .vscode .next .nuxt .svelte-kit .angular .turbo .parcel-cache .serverless .terraform
  .claude __snapshots__ storybook-static .eggs *.egg-info graphify-out`.
  Hidden directories in general are **not** skipped, nor are `vendor`, `third_party`, `generated`,
  `coverage`, `out` and `bin`.
- **Ignore files:**
  - `.gitignore`, including nested ones, is honoured **even outside a git repository**.
  - `.graphifyignore` is always honoured.
  - `--no-gitignore` disables only `.gitignore`.
- **Lockfiles and sensitive files:**
  - `package-lock.json` is silently dropped.
  - Files that look sensitive (`.env*`, `*.pem`) are dropped.
  - `credentials.py` and `secrets.py` are not dropped.
- **Test files** (`tests/`, `test_*.py`, `*.test.ts`, `__tests__/`, `*_test.go`, `src/test/java`)
  get no special treatment in any language.
- **Empty files.** A 0-byte file still gets its file node, with `source_location: null` (933 empty
  `__init__.py` in erpnext). A file containing only blank lines or a comment gets `"L1"`.

---

## 4. Node ids

### 4.1 `slug`

**Rule.** `slug(s)`: lowercase (Unicode-aware); replace every maximal run of characters that are not
Unicode letters or digits **(underscore counts as a separator)** with one `_`; strip leading and
trailing `_`. Unicode letters are kept, not transliterated.

Examples:

| Input | Slug |
|---|---|
| `__init__` | `init` |
| `__tests__` | `tests` |
| `$store` | `store` |
| `a$b` | `a_b` |
| `quoted-name` | `quoted_name` |
| `snake_Case_X` | `snake_case_x` |
| `Ñandú` | `ñandú` |
| `2fast` | `2fast` (a leading digit is kept) |
| `os.path` | `os_path` |

### 4.2 File ids

**Rule.** The file id is `slug(relative path without its final extension)`.

| Path | File id |
|---|---|
| `pkg/models.py` | `pkg_models` |
| `pkg/__init__.py` | `pkg_init` |
| `src/__tests__/service.test.ts` | `src_tests_service_test` |
| `crates/globset/src/pathutil.rs` | `crates_globset_src_pathutil` |
| `benchsuite/benchsuite` | `benchsuite_benchsuite` |
| `a.b/c.ts` | `a_b_c` |

`a-b/c.py` and `a_b/c.py` both slug to `a_b_c` (resolved by §4.5).

Internally the per-file cache uses `$graphify-root$_<slug(path with extension)>` (`…_pkg_models_py`).
The prefix and the extension segment are removed when ids are finalised. That intermediate form is
not part of the output contract.

### 4.3 Symbol ids (summary; details per language)

| Construct | Python | TS/JS | Rust | Go | Java |
|---|---|---|---|---|---|
| top-level function | `file_fn` | `file_fn` | `file_fn` (also fns of inline `mod` blocks, flattened) | `file_fn` | n/a |
| class/type | `file_cls` | `file_cls` | `file_type` (struct/enum/trait; generic or foreign impl owners get `file_slug(type text)`) | **`dir_type`** | `file_type` (flat, also for nested types) |
| method | `cls_m` | `cls_m` | `owner_m` | `dir_type_m` | `type_m` |
| nested function | `parent_fn` | `parent_fn` | not emitted | not emitted | n/a |
| nested class | `outer_inner` | not emitted | not emitted (items in fn bodies) | n/a | `file_inner` (flat) |
| macro | n/a | n/a | `file_macro_name` | n/a | n/a |
| enum member | n/a | `enum_member` | `enum_variant` | n/a | `enum_CONST` |
| rationale | `file_rationale_<line>` | `file_rationale_<line>` | none | none | none |

`file` is the file id, `cls`/`type` the containing type's id, and every name component goes through
`slug`.

### 4.4 Stub and external nodes

Three kinds of nodes have no source file of their own.

1. **External import nodes** (`file_type: "concept"`):
   `{"id":"json","label":"json","file_type":"concept","type":"external","external":true,"source_file":""}`.
   - They have no `source_location` and no `_origin`.
   - They are shared across files and never collision-prefixed.
   - Ids by language:
     - Python: `slug(module path)` (`os_path`; an unresolved relative import gives `pkg_nope`).
     - TS/JS: `ref_<slug(package)>`.
     - Rust: `slug(last path segment)` (`cow`; aliases give `circle_as_round`).
     - Go: `go_pkg_<slug(import path)>`.
     - Java: `slug(last segment)`.
   - In every language the label equals the id (`os_path`, `ref_react`, `go_pkg_fmt`), not the
     module path as written.
2. **Unresolved-reference stubs** (`file_type: "code"`):
   `{"id":"t","label":"T","file_type":"code","source_file":"","source_location":"","_origin":"ast"}`.
   - Created for names referenced as types, bases or decorators that do not resolve to a local
     definition: Python `T`, `Foo`, `ExtBase`, `fixture`, `object`; Go type parameters; Rust generic
     parameters; Java fully qualified annotation and type stubs.
   - Id `slug(name)` (Java: `slug(FQN)`; Go qualified: `go_type_<slug(path)>_<slug(Name)>`).
   - Label = the name as written.
   - They are per file, so they go through the collision rule. A name referenced from two files
     becomes `a_py_foo` and `p_b_py_foo`.
3. **Dynamic-import phantoms (TS/JS, quirk).** Code nodes with a `confidence` key and no
   `source_location` (§10).

### 4.5 Id collisions across files

**Rule.** After per-file extraction, every id that is emitted by **more than one source file** is
rewritten, for each emitter, to `slug(source path WITH extension) + "_" + id`. Ids that do not
collide are left alone. If the rewritten ids still collide, `"_" + sha1(relative path).hexdigest()[:6]`
is appended. External import nodes are exempt.

| Case | Result |
|---|---|
| `a/util.py` + `a/util.ts` | `a_util_py_a_util`, `a_util_ts_a_util`; their `run` symbols become `a_util_py_a_util_run`, `a_util_ts_a_util_run` |
| `a.py` defines `b()` (`a_b`) + file `a/b.py` (`a_b`) | function `a_py_a_b`, file `a_b_py_a_b` |
| `a-b/c.py` + `a_b/c.py` | `a_b_c_py_a_b_c_e32f91`, `a_b_c_py_a_b_c_0ae3b3` |
| Python stub `base` (from `class B(Base)` in `sub.py`, unresolved during per-file extraction) + file `base.py` | the file node becomes **`base_py_base`**, label `base.py` (probe `py3`; also a **quirk**, since the stub is later resolved to `base_base` and the file id still stays renamed) |
| two `package.json` depending on `react` | `package_json_ref_react`, `sub_package_json_ref_react` |

Corpus prevalence:

- erpnext: 938 rewritten file ids (`x.py` + `x.js` + `x.json` doctype triples) and 4 sha1-suffixed
  ids.
- graphify: 46.
- excalidraw: 2.

**Same-file id collisions** are not disambiguated. The nodes merge, and in raw output the **first
node wins** (Python: a redefined `def f` keeps the first line, L1). TS/JS, Go and Java have
language-specific variants (§10, §12, §13). **Quirk.**

---

## 5. Labels and `norm_label`

### 5.1 File labels

**Rule.** The file label is the shortest `/`-delimited suffix of `source_file` that is unique among
all file nodes in the graph. If no suffix is unique, it is the full path. This matched 100% of about
8,500 file nodes across the six corpora. Examples:

- `crates/globset/src/pathutil.rs` → `globset/src/pathutil.rs`, because
  `crates/ignore/src/pathutil.rs` makes `src/pathutil.rs` ambiguous.
- `pkg/__init__.py` → `pkg/__init__.py` next to `pkg/sub/__init__.py` → `sub/__init__.py`.
- Unique basenames get the basename: `models.py`.
- `b/c.py` together with `x/b/c.py` gives `b/c.py` (no unique suffix, so the full path) and
  `x/b/c.py`.

The candidate set is the files that produced nodes. A non-extracted JSON file does not count. In
`update` graphs, markdown and code share one namespace. The original case is kept (`App.tsx`).

### 5.2 Symbol labels

| Construct | Label |
|---|---|
| function (any language), nested function | `name()` |
| method, constructor, accessor (Python, TS/JS, Go, Java) | `.name()` (Java constructor `.User()`, TS `.constructor()`, Python `.__init__()`) |
| class, interface, struct, enum, trait, type alias, namespace | bare name (`User`, `Repo`) |
| enum member / constant, value const | bare name (`ACTIVE`, `CONFIG`) |
| Rust macro (`macro_rules! square`) | `square!` |
| Rust associated const | `.UNIT` |
| Rust generic/foreign impl owner | type text verbatim (`Wrapper<T>`) |
| shell function / entry | `clean()` / `build.sh script` |
| JSON key | key text |
| stub | the name as written (`T`, `ExtBase`; Java FQN `org.junit.jupiter.api.Test`; Go `net/http.Request`) |
| external | id-like slug (`os_path`, `ref_react`, `go_pkg_fmt`) |

### 5.3 Rationale labels

**Python docstrings:**

- The raw source text of the docstring with whitespace runs collapsed to single spaces and stripped.
  Escapes are kept as written.
- If the text is longer than 80 characters, the label is `text[:80].rsplit(' ', 1)[0] + "…"`
  (U+2026). This was verified on 56 of 56 corpus docstrings: N = 80 exactly, and 78, 79 and 81 all
  fail.

**Comments:**

- Python keeps the marker: `# NOTE: persistence is faked in this fixture.`
- TS/JS strip it: `NOTE: …`.
- The same 80-character truncation applies.

### 5.4 `norm_label`

**Rule.** `norm_label = lower(remove_combining_marks(unicodedata.normalize("NFKD", label)))`, applied
in that order. This matched 86,373 of 86,373 corpus nodes. Plain `lower()` fails on 9,374 of them:

| Label | `norm_label` |
|---|---|
| `…` | `...` |
| `Über` | `uber` |
| `función` | `funcion` |
| `सशर्त` | `सशरत` |

---

## 6. Node fields

| Field | Values | When present |
|---|---|---|
| `id`, `label` | strings | always |
| `file_type` | `code`, `rationale`, `concept` (`document` only in `update`) | always |
| `source_file` | relative `/` path; `""` on stubs and externals; the specifier on TS/JS phantoms | always |
| `source_location` | `"L<n>"` (1-based line, never a range or column); `""` on stubs; **absent** on externals and phantoms; `null` on 0-byte files | see left |
| `_origin` | `"ast"` | every node except externals |
| `_callable` | `true` | Python functions, methods and classes; TS/JS functions, methods, classes, interfaces, type aliases and enums; Java types and methods. **Never on Go, Rust, shell or JSON.** |
| `_callable_class` | `true` | the class-like subset of the above (Python classes; TS/JS classes, interfaces, type aliases, enums; Java types) |
| `external`, `type: "external"` | `true` / `"external"` | external import nodes |
| `type: "package"`, `ecosystem`, `version` | `cargo`/`python`/`go` | TOML and `go.mod` package nodes |
| `metadata` | object | Python `{"python_opaque_bases": true}`; Java `{"unresolved_calls": [...]}`; shell `{"language":"bash","kind":…}`. Never on Rust, Go or TS/JS nodes. |
| `_rust_declaration_count` | int | every Rust struct/enum/trait node (count of same-name declarations in the file, e.g. two `#[cfg]` variants → 2) |
| `_rust_impl_key` | `"Name/n"` | Rust owner nodes created by plain-generic inherent impls (`impl<T> Pair<T>` → `Pair/1`) |
| `confidence` | `"EXTRACTED"` | TS/JS dynamic-import phantoms only (quirk) |
| `node_kind`, `frontmatter` | | markdown nodes from `update` only, never in code-only builds |
| `community`, `norm_label`, `community_name` | | clustered only (§2.5) |

Raw key order for code nodes is
`id, label, file_type, source_file, source_location, [metadata], [_callable], [_callable_class], _origin`.
For externals it is `id, label, file_type, type, external, source_file`.

---

## 7. Edge fields and relations

### 7.1 Fields

| Field | Values | Notes |
|---|---|---|
| `source`, `target`, `relation` | | direction as extracted. Clustered output keeps the surviving edge's own direction (§8). |
| `confidence` | `EXTRACTED`, `INFERRED` | `AMBIGUOUS` never occurs in 180,410 corpus edges or in any fixture |
| `confidence_score` | `1.0`, `0.85`, `0.95` (`0.55` default for INFERRED without a score) | raw: cross-file pass only. Clustered: always (§2.5). |
| `context` | per relation (§7.3) | optional |
| `source_file`, `source_location` | file of the **source construct**; `L<n>` | `dynamic_import` has no `source_location` |
| `weight` | `1.0`; `0.8` on `uses` | `dynamic_import` has no `weight` |
| `type_only` | `true` | TS `import type` on `imports_from`, `re_exports` and `imports` |
| `deferred` | `true` | TS/JS `import()` inside a function, on `imports_from` |
| `metadata` | object | never for the six languages here (C#, Swift and Razor only in the corpus) |
| `_origin` | `"ast"` | always |

### 7.2 Relations by language

| Relation | Py | TS/JS | Rust | Go | Java | Confidence / score | Weight |
|---|---|---|---|---|---|---|---|
| `contains` | ✓ | ✓ | ✓ | ✓ | ✓ | EXTRACTED | 1.0 |
| `method` | ✓ | ✓ | ✓ | ✓ | ✓ | EXTRACTED | 1.0 |
| `calls` | ✓ | ✓ | ✓ | ✓ | ✓ | EXTRACTED (none in per-file pass / 1.0) or INFERRED 0.85 | 1.0 |
| `indirect_call` | ✓ | ✓ | | | | INFERRED 0.85 | 1.0 |
| `uses` | ✓ | | | | | INFERRED 0.95 | **0.8** |
| `references` | ✓ | TS | ✓ | ✓ | ✓ | EXTRACTED | 1.0 |
| `inherits` | ✓ | ✓ | ✓ | | ✓ | EXTRACTED | 1.0 |
| `implements` | | TS | ✓ | | ✓ | EXTRACTED | 1.0 |
| `case_of` | | TS | ✓ | | ✓ | EXTRACTED | 1.0 |
| `embeds` | | | | ✓ | | EXTRACTED | 1.0 |
| `imports` | ✓ | ✓ | | | ✓ | EXTRACTED (1.0 for cross-file symbol imports) | 1.0 |
| `imports_from` | ✓ | ✓ | ✓ | ✓ | | EXTRACTED | 1.0 |
| `re_exports` | ✓ | ✓ | | | | EXTRACTED | 1.0 |
| `dynamic_import` | | ✓ | | | | EXTRACTED | absent |
| `rationale_for` | ✓ | ✓ | | | | EXTRACTED | 1.0 |
| `defines` | | | | | | EXTRACTED (shell) | 1.0 |
| `extends` / `depends_on` | | | | | | EXTRACTED (JSON / TOML) | 1.0 |

Corpus counts per relation, confidence and extension, clustered `update` graphs:

| Relation, confidence (score) | .py | .ts | .tsx | .js family | .rs |
|---|---|---|---|---|---|
| calls, EXTRACTED | 39055 | 3067 | 4039 | 1123 | 3498 |
| calls, INFERRED (0.85) | 1687 | 135 | 197 | | 241 |
| contains | 21191 | 2454 | 1418 | 1743 | 2166 |
| method | 9544 | 614 | 211 | 97 | 2448 |
| imports | 9692 | 3045 | 4377 | 2 | |
| imports_from | 9701 | 1753 | 3111 | 124 | 1424 |
| references | 8715 | 455 | 160 | | 2770 |
| inherits | 1613 | 4 | 9 | | 3 |
| implements | | 8 | | | 182 |
| case_of | | 42 | | | 475 |
| indirect_call | 1192 | 29 | 64 | 20 | |
| uses | 844 | | | | |
| re_exports | 43 | 238 | 95 | | |
| rationale_for | 7266 | 43 | 15 | 6 | |

Go and Java barely appear in the corpus, so their rules rest on the fixtures.

### 7.3 `context` values

| Relation | Contexts |
|---|---|
| `calls` | `call` |
| `contains` | none; `namespace_export` (TS `export * as X`); **`call`** in 935 corpus edges, a collapse artefact (§8.2) |
| `method` | none (`call` from collapse, rare) |
| `imports` | `import` (Java's second pass: none) |
| `imports_from` | `import`; `submodule_import` (Python `from pkg import submodule`); `re-export` (TS `export … from`) |
| `re_exports` | `export` (to a module); `re-export` (TS, to a symbol) |
| `references` | `parameter_type`, `return_type`, `generic_arg`, `field`, `decorator` (Python/TS), `attribute` (Java annotations), `constructor` (Rust tuple-struct and variant calls; INFERRED 0.85 cross-file) |
| `uses` | none (it can inherit `parameter_type`/`return_type`/`generic_arg` through collapse) |
| `indirect_call` | `argument`, `collection`, `return`, `assignment`, `getattr` |
| `inherits`, `implements` | none (TS: `type`) |
| `case_of`, `embeds`, `rationale_for`, `dynamic_import`, `defines` | none |
| `extends` (JSON) | `import` |
| `depends_on` (TOML) | `dependency` |

---

## 8. Build stage: from extraction to the clustered graph

The clustered graph is built from the **undeduplicated** extraction (per-file edges in file order,
then cross-file edges). It is not built from the raw `graph.json`. Steps observed, in order of
effect:

### 8.1 Node-level transforms

1. **Duplicate node ids: the first node wins.** Its attributes and `source_location` are kept.
   Python probe: `def f` defined at L1 and redefined at L11 → node `a_f` L1. Exception: Go merges
   with the last writer's attributes (§12).
2. **Rationale deduplication.** Rationale nodes with the same `source_file` and the same `label` are
   merged into the first one. Their `rationale_for` edges are re-pointed to that node and keep their
   own `source_location`.
   - ripgrep `benchsuite`: `…_rationale_466`, `_602` and `_627` all become `…_rationale_442`.
   - Identical docstrings in **different** files are not merged.
3. **External stubs are remapped to file nodes.** This happens in clustered output only; raw output
   is never remapped. When an external's label equals the filename stem of exactly one file node,
   the external is deleted.
   - Import edges from files of the **same language** as that file are re-pointed to it.
   - Import edges from other languages are **dropped**.
   - With 0 or 2+ matching files, nothing changes.

   ripgrep: `hostname` → `crates_cli_src_hostname`, and `path` → `crates_printer_src_path` even
   across crates (21 of 21 such stubs). The fixture wrongly maps `use std::fmt` to `src/util/fmt.rs`.
   Python externals are dotted module paths and rarely match a stem; fastapi's `http`, `types` and
   `gzip` were **not** remapped (**Open**: why). **Quirk.** Details in §11.2.
4. Externals and stubs remain in the graph. They get `community` and `norm_label` like any other
   node.

### 8.2 One edge per node pair

The graph is an undirected simple graph, so every unordered pair `{u, v}` keeps exactly one link.
Edges are folded in extraction order. For each new edge `N` on a pair that already holds an edge `E`:

**Rank table.** This is inferred: it is the smallest total order consistent with every observed
pair.

| Rank | Relations |
|---|---|
| 1 | `references` |
| 2 | `uses` |
| 3 | `calls`, `case_of`, `dynamic_import` |
| 4 | `contains`, `method`, `inherits`, `implements`, `rationale_for` |
| 4.5 | `imports` |
| 5 | `imports_from`, `indirect_call` |
| 6 | `re_exports` |

**Same orientation** (`N.source == E.source`):

- **If `rank(N) >= rank(E)`, N wins.**
  - When `N` is `contains` or `method`, it replaces `E` whole (Java: an earlier
    `references generic_arg` leaves no trace on `contains`).
  - Otherwise `E = {**E, **N}`: `N`'s keys win, and keys only `E` had survive. Examples:
    `references(parameter_type)` + `uses` gives `uses` with `context: "parameter_type"`; an INFERRED
    0.85 `calls` followed by an EXTRACTED `calls` gives EXTRACTED with `confidence_score: 0.85`.
  - For the same relation the later edge wins: a duplicate `references` with `return_type` beats
    the earlier `parameter_type`; duplicate Rust `method` edges from two `impl` blocks give the later
    line.
- **Otherwise, E wins.**
  - When `E` is `contains` or `method`, nothing is merged, except that a folded `calls` edge sets
    `context: "call"` if `E` has no context. This is the source of the 935 `contains` links with
    context `call`. A folded `references` edge leaves no trace (Go `store_store_go_store_store →
    store_id` keeps `contains` with no context).
  - Otherwise `E = {**N, **E}`.

**Opposite orientation:**

- There is **no key merging**. One edge survives whole, with its own direction.
- If the ranks differ, the higher-ranked edge survives.
- If the ranks are equal, the edge whose **source id is lexicographically smaller** survives,
  regardless of extraction order (probe `rev1`: `x_alpha→x_zeta` and `x_beta→x_gamma` both survive).
  **Except in rank 4** (`contains`, `method`, `inherits`, `implements`…), where the **larger**
  source survives: a nested class that inherits its outer class keeps `contains` when its id sorts
  first (`f_result -contains-> f_ok`, Java `r_r -contains-> r_o`) and `inherits` otherwise
  (okhttp `…connectionevent_connectstart -inherits-> …connectionevent_connectionevent`).

**Validation.** Replaying this model on raw edges reproduced 759 of 769 multi-edge pairs (all six
language fixtures plus scratch `extract` runs of ripgrep and fastapi). The ten misses:

- Six only because raw output hides a later duplicate edge (§2.4). With the duplicate restored, the
  "later wins" rule explains them.
- Four are the quirk below.

Worked examples (from goldens and probes):

| Edges on one pair (in order) | Clustered survivor |
|---|---|
| `pkg_models_admin_promote -contains-> …_bump` L52; `-calls->` L55 | `contains` L52 with `context: "call"` |
| `pkg_util_traced -contains-> …_wrapper` L7; `-indirect_call->` (return) L10 | `indirect_call` L10, INFERRED 0.85 |
| `pkg_service_run -references-> …userservice` (return_type) L42; `-calls->` L43 | `calls` L43 |
| `pkg_service_run -calls-> pkg_models_repo` L43; `-uses->` L43 | `calls` (score 1.0) |
| fastapi `…custom_generate_unique_id -references-> …apiroute` (parameter_type); `-uses->` | `uses` INFERRED 0.95, weight 0.8, **`context: "parameter_type"`** |
| `main -imports-> pkg_init` L5; `-imports_from->` L6 | `imports_from` L6 |
| Java `…userservice -contains-> …userservice_formatter` L67; 2nd-pass `-imports->` L8 (no context) | `imports` L8 |
| Go `store_store_go_store_store -contains-> store_id` L12; `-references->` (field) L40 | `contains` L12, no context |
| `pkg_init -imports_from-> pkg_models` L2; `-re_exports->` L2 | `re_exports` (context `export`) |
| `pkg_models_user -method-> …_parse`; `…_parse -references-> pkg_models_user`; `…_parse -calls-> pkg_models_user` | `method` (no context; reversed edges never merge) |
| `x_zeta -calls-> x_alpha`; `x_alpha -calls-> x_zeta` | `x_alpha -calls-> x_zeta` |
| ripgrep `close -calls-> drop` (INFERRED 0.85) vs `drop -calls-> close` (EXTRACTED) | `close→drop`, INFERRED 0.85 (smaller source) |
| erpnext: INFERRED 0.85 `calls` then EXTRACTED `calls`, same pair, same orientation | EXTRACTED with `confidence_score: 0.85` (a leftover key) |
| TS `src_service -imports_from-> src_util` + `-dynamic_import->` | `imports_from` |
| JS `lib_store -contains-> lib_store_helper` + file `-indirect_call->` (collection, `module.exports = { helper }`) | `indirect_call` |

**Quirk (import cycle through a re-exporting package).** A package `__init__.py` that does
`from .m import X` (`imports_from` + `re_exports` to `m`) while `m` does `from . import Y`
(`imports_from` back to the package) keeps the edge whose source is lexicographically **larger**,
against the tie rule above:

| Probe | Edges | Survivor |
|---|---|---|
| `tri_a` | `pkg_init→pkg_a` (imports_from, re_exports); `pkg_a→pkg_init` (imports_from) | `pkg_init→pkg_a` `re_exports` |
| `tri_z` | `pkg_init→pkg_z` (imports_from, re_exports); `pkg_z→pkg_init` (imports_from) | `pkg_z→pkg_init` `imports_from` |

It occurs in 5 of 5 cases (4 in fastapi, 1 in the fixture: `pkg_service -imports_from-> pkg_init`).
**Open.**

### 8.3 Other edge transforms

- **Self-loops are kept** (259 `calls` self-loops in the corpus; TS `console.log` and Go `contains`
  self-loops survive). The exception is a Python package importing itself (`from . import sub` in
  `pkg/__init__.py` gives raw `pkg_init -imports_from-> pkg_init`), which is **dropped**.
- Edges whose endpoint was renamed by §8.1 follow the rename.
- `confidence_score` is filled (§2.5). `weight` is unchanged.

### 8.4 Determinism

- Cold builds are byte-identical across runs (ripgrep, excalidraw, all fixtures).
- **Quirk: warm rebuilds lose dynamic imports.** On a warm `graphify-out/cache`, re-running
  `extract --code-only --no-cluster` on excalidraw produced 5,012 nodes and 20,041 edges against
  5,024 and 20,054 cold. Exactly the 13 `dynamic_import` edges and the 12 phantom nodes they created
  went missing, so dynamic-import facts are not cached. This explains the "warm build has fewer
  nodes" observation on excalidraw and graphify. For Python and Rust (ripgrep) warm equals cold.
  Dijkstral must produce identical output warm or cold.

---

## 9. Python

Fixture `python/`; ids below are from `python.graphify.json`.

### 9.1 Nodes

| Construct | Id | Label | Fields |
|---|---|---|---|
| file | `pkg_models` (`pkg/__init__.py` → `pkg_init`, `main.py` → `main`) | `models.py` | — |
| class (any decorators) | `pkg_models_user` | `User` | `_callable`, `_callable_class`; `metadata.python_opaque_bases` (below) |
| nested class | `a_mine_inner` (`<outer>_<inner>`) | `Inner` | as class |
| method (instance, `@staticmethod`, `@classmethod`, `@property`, `async`, dunder) | `pkg_models_user_parse`, `pkg_models_repo_init` | `.parse()`, `.__init__()`, `.display()` | `_callable` |
| function | `pkg_models_validate` | `validate()` | `_callable` |
| nested function (any depth, inside a function or method) | `pkg_models_admin_promote_bump`, `pkg_util_traced_wrapper` | `bump()` (no dot) | `_callable` |
| docstring rationale | `pkg_models_rationale_12` (line of the docstring) | collapsed text | `file_type: "rationale"` |
| comment rationale | `pkg_models_rationale_15` | `# NOTE: …` | `file_type: "rationale"` |
| unresolved type/base/decorator stub | `t`, `fixture`, `extbase`, `foo`, `object` | `T`, `fixture` | stub shape (§4.4) |
| external module | `sys`, `os_path`, `typing`, `pkg_nope` | same | external shape |

The node's `source_location` is the `def`/`class` line, not the decorator line.

**Not emitted:**

- module-level variables and constants (`DEFAULT_ROLE`, `T = TypeVar(...)`, `INSTANCE = Model()`);
- class attributes and dataclass fields;
- lambdas;
- `if __name__ == "__main__":` blocks;
- string-annotation types (`-> "User"`, `c: "Circle"`);
- builtin types (`str`, `int`, `bool`, `list[...]`, `dict[...]`);
- names imported from external modules when used as types (`Optional`, `List`);
- local-variable annotations (`x: K = None`, `svc: Svc` produce no `references`).

`metadata: {"python_opaque_bases": true}` is set when any **positional** base is not a bare
identifier, that is a subscript, attribute or call: `Repo(Generic[T])`, `K3(abc.ABC)`,
`K4(make_base())`. Keyword arguments (`metaclass=`) are ignored. Opaque bases produce no `inherits`
edge (`K2(Base[int])` has none).

### 9.2 Rationale nodes

- **Docstrings** of modules, classes and functions/methods produce a node when the collapsed text is
  **longer than 20 characters** (21 characters emitted, 20 not; `"Same doc."` is dropped). The
  `rationale_for` target is the owner: module → file node (`pkg_init_rationale_1 → pkg_init`),
  class → class, function → function. Location: the docstring's line.
- **Comments** produce a node when the line, after indentation, starts with exactly `# ` followed
  by `NOTE`, `WHY`, `TODO`, `FIXME`, `HACK`, `IMPORTANT` or `RATIONALE` and then `:`. The match is
  case-sensitive and there is no minimum length (`# NOTE: x` counts).
  - Rejected: `#NOTE:`, trailing comments after code, `# Note:`, and the markers `XXX`, `SAFETY`,
    `PERF`, `BUG`, `WARNING`, `DESIGN`, `INVARIANT`, `REASON`, `DECISION`, `because`.
  - `rationale_for` **always targets the file node**, even for a comment inside a method
    (`pkg_models_rationale_15 → pkg_models`).

### 9.3 Edges

| Relation | Source → target | Emitted when | Confidence (raw score) | Context | Location |
|---|---|---|---|---|---|
| `contains` | file → top-level def/class; class → nested class; function → nested function | always | EXTRACTED | — | child's def line |
| `method` | class → method | every method | EXTRACTED | — | method line |
| `inherits` | class → base | bare-identifier base: same-file class, imported class (resolved during per-file extraction, no score), or stub (`extbase`, `object`) | EXTRACTED | — | class line |
| `references` | function/method → type | annotations of params and returns that resolve to a local or imported class, or become stubs. `Optional[Foo]` → `foo` with `generic_arg`; `Repo[U]` → `pkg_models_repo` `parameter_type` + `pkg_models_user` `generic_arg` | EXTRACTED | `parameter_type`, `return_type`, `generic_arg` | def line |
| `references` | decorated def → decorator | `@traced`, `@deco` (imported), `@ext_deco(arg=1)` → stub, `@pytest.fixture` → stub `fixture`. Builtins (`@property`, `@staticmethod`, `@classmethod`) and `@dataclass` produce nothing. | EXTRACTED | `decorator` | **decorator line** |
| `imports` | file → external module | `import x`, `import x.y`, `import x.y as z` (target `slug(x.y)`) | EXTRACTED | `import` | statement line |
| `imports` | file → local module | `import pkg` → `pkg_init`; `import pkg.sub.mod` → `pkg_sub_mod` | EXTRACTED | `import` | statement line |
| `imports_from` | file → module | `from M import …` where M is local (`pkg_models`), a package (`from . import models` → **`pkg_init`**), or external (`collections`). `from __future__` is ignored. | EXTRACTED | `import` | statement line |
| `imports_from` | file → submodule | `from pkg import submodule` (cross-file pass) | EXTRACTED 1.0 | `submodule_import` | statement line |
| `imports` | file → symbol | each name in `from M import a, b as c` that resolves to a definition, **following re-exports** (`from pkg import run` → `pkg_service_run`). Also inside `if TYPE_CHECKING:`. Not for `*`, not for module names, not for external modules. | EXTRACTED 1.0 | `import` | statement line |
| `re_exports` | `__init__.py` file → module | every `from .m import …` inside a package `__init__.py` (independent of `__all__`) | EXTRACTED | `export` | statement line |
| `calls` | caller → callee | §9.4 | EXTRACTED / INFERRED 0.85 | `call` | first call line |
| `indirect_call` | enclosing function → function | a function **name** passed as an argument (`sorted(xs, key=helper)`, `map(make, [])`) → `argument`; or returned (`return wrapper`) → `return`. Corpus also has `collection`, `assignment` and `getattr`. Resolution follows bare-call rules. An aliased import passed as an argument (`map(util_helper, …)`) is **not** resolved. | INFERRED 0.85 | as listed | first occurrence |
| `uses` | enclosing **top-level function**, or the **class** when inside a method or class body → class in **another file** | any mention of an imported class name: annotation (also local-variable annotations through an alias: `svc: Svc` → `main_main -uses-> pkg_service_userservice`), constructor call, `isinstance` argument, plain reference (`x = Mine`), base class | INFERRED 0.95, weight 0.8 | — | line of the mention |
| `rationale_for` | rationale → owner | §9.2 | EXTRACTED | — | rationale line |

There is no `uses` for same-file classes, for functions, or from module-level code. Module-level
statements produce **no** `calls` or `uses` edges at all: there is no file-level caller.

### 9.4 Call resolution

Calls are attributed to the nearest enclosing def. They are deduplicated per (caller, callee) and the
first call line is kept.

| Call form | Result |
|---|---|
| `f()` defined in the same file (incl. nested `bump()`, constructor `User(...)` → class node) | EXTRACTED, no score |
| `f()` imported by name (`slugify` via `from .util import …`), via star import (`format_name` via `from .models import *`), or a function-level import | EXTRACTED 1.0 |
| `f()` not imported, globally unique function name (`lonely_b()` defined only in `b.py`) | INFERRED 0.85 |
| `f()` not imported, name defined in ≥2 files (`shared_name`), or undefined (`unknown_function()`), or a builtin | dropped |
| aliased constructor `U(name=…)` (`User as U`) | dropped (`calls`); the class still gets `uses` |
| `self.m()` | own class, else inherited base (same file EXTRACTED; cross-file base EXTRACTED 1.0: `sub_b_go → base_base_inherited_only`) |
| `super().m()` | base method, EXTRACTED (`pkg_models_user_save → pkg_models_base_save`) |
| `Cls.m()` with `Cls` local or imported | EXTRACTED (1.0 cross-file): `test_parse → pkg_models_user_parse`, `a_caller → a_mine_m` |
| `x.m()` where `x`'s class is known from a parameter annotation (incl. string annotation `admin: "Admin"`) or `x = Cls()` in the same function | INFERRED 0.85 to `Cls.m` **only if `m` is defined directly on `Cls`**: `service.register` → `pkg_service_userservice_register`; `c.area()` → `lib_shapes_circle_area`. Inherited methods are not walked (`c.uniq_shape_method()` → dropped). |
| `x.m()` with unknown receiver type, even if `m` is unique in the repo (`obj.unique_m()`); `self.repo.add(...)`; chained `Mine().am()` | dropped |
| `mod.f()` on a **local** module alias (`models.validate(...)`, `sh.make()`, `lib.shapes.make()`) | dropped |
| `mod.f()` on an **external** module bound by `import x` / `import x.y as z`, inside a **top-level function** | `calls` → the external node, EXTRACTED 1.0 (`pkg_util_slugify → re`, `app_f1 → json`, `→ os_path`) |
| the same inside a **method**, a dotted receiver (`os.path.join`), or `from os import path as p2; p2.join()` | dropped |
| a name imported from an external module (`OrderedDict()`, `defaultdict(list)`) | dropped |

### 9.5 Imports summary (fixture `pkg/service.py`)

```
import json                              -> service -imports-> json                 (L2)
import os.path as osp                    -> service -imports-> os_path              (L3)
from collections import defaultdict      -> service -imports_from-> collections     (L4)
from . import models                     -> service -imports_from-> pkg_init        (L7)
from .models import *                    -> service -imports_from-> pkg_models      (L8)
from .models import Repo, User as U      -> (imports_from deduplicated) + service -imports-> pkg_models_repo, pkg_models_user (L9, 1.0)
from .util import helper as util_helper, slugify -> -imports_from-> pkg_util + -imports-> pkg_util_helper, pkg_util_slugify
if TYPE_CHECKING: from .models import Admin -> -imports-> pkg_models_admin (L13)
```

Also:

- An unresolvable relative import (`from .nope import zz`) creates the external `pkg_nope`.
- `from . import missing_mod` gives only `imports_from pkg_init`.
- `try: import ujson as json / except ImportError: import json` gives both.

### 9.6 Duplicate names

- `helper()` exists in `models.py`, `service.py` and `util.py`.
- Calls resolve by scope:
  - the local `helper` is chosen for `key=helper` in `service.py`;
  - `util.py`'s own `helper` is used by `slugify`;
  - none of them is ever reached through the unique-name fallback.
- Two `.save()` methods (`Base`, `User`) are separate nodes. `user.save()` on `U(...)` is dropped.

---

## 10. TypeScript / TSX and JavaScript / JSX

Fixtures `typescript/` and `javascript/`. Everything below describes raw output unless it says
otherwise. TS and JS share one extractor, so TS-only syntax is just absent from JS, and CommonJS
handling applies to `.ts` too.

### 10.1 Nodes

| Construct | Id | Example |
|---|---|---|
| File | `slug(path without last extension)` | `src/__tests__/service.test.ts` → `src_tests_service_test`; `Dir-Name/Some.Component.tsx` → `dir_name_some_component` |
| Top-level function, class, interface, type alias, enum, namespace, value const | `file_id_slug(name)` | `src_models_user_formatname`, `src_models_repo_storage` |
| Method (incl. constructor, getter/setter pair, class-field arrow, interface method signature, `#priv`) | `class_id_slug(name)` | `src_models_user_user_constructor`, `src_ui_app_app_handleclick` |
| Enum member | `enum_id_slug(member)` | `src_models_user_role_member` |
| Nested function declaration (any depth inside a named function or method) | `parent_id_slug(name)` | `src_models_user_admin_promote_bump` |
| Function inside a namespace | `file_id_slug(name)`: namespaces are **flattened** | `export namespace Storage { function key }` → `src_models_repo_key` |
| Destructured `require` / call result | `file_id_slug(pattern text)` | `const { join } = require("path")` → `src_util_join` |
| `export * as nsA from` | `file_id_slug(alias)` | `lib_re_nsa` |
| Rationale comment | `file_id_rationale_<line>` | `src_models_user_rationale_21` |

**Labels.**

- Functions, arrow/function-expression consts, generators and `exports.x = function` → `name()`.
- Methods → `.name()`, using the raw name text (`.constructor()`, `.#priv()`, `.['computed']()`).
- Types, namespaces, enum members and consts → bare name.
- Destructured `require` → the raw pattern text (`{ join }`).

**Emitted:**

- Function declarations at any depth inside named functions or methods.
- At top level and inside namespaces, `const`/`let` initialised with an arrow function or function
  expression.
- Classes, abstract classes, interfaces, type aliases, enums, const enums, namespaces, and
  `declare module`.
- Class methods, getters, the constructor, abstract methods, `#private` methods, class fields
  initialised with a function, and interface method signatures.
- Enum members.
- `exports.name = function…` and `module.exports.name = …`.
- Top-level non-function `const`/`let` when **exported** (any initializer), or when not exported but
  initialised with an object literal, array literal, `new`, or a call (including `require`). These
  carry no flags.

**Not emitted:**

- `var` declarations;
- non-exported primitive consts;
- classes declared inside functions;
- arrow consts inside functions (their calls count for the enclosing function);
- object-literal methods;
- non-function class properties;
- TS overload signatures (only the implementation is emitted);
- `declare function`;
- anonymous default exports (`export default function () {}`, `export default {…}`,
  `module.exports = function …`);
- IIFEs;
- class expressions. Quirk: `const K = class Inner { m(){} }` emits `m` as a top-level function
  `decls_m`.

**Fields:**

- `_callable` on functions, methods, classes, interfaces, type aliases and enums.
- `_callable_class` on classes, abstract classes, interfaces, type aliases and enums. Not on
  namespaces, consts, enum members or files.
- `source_location` is the declaration keyword's line. A decorated class takes the `class` line
  (`src_models_repo_repo` L10; its decorator is on L9).
- There is never `metadata` or `node_kind` on TS/JS nodes.

**Same-file id collisions (quirk).**

- Matching labels (`function dup` twice; `get x` with `set x`): the first node wins.
- Differing labels (`function foo` with `class Foo`): the merged node takes the later node's bare
  name and line and loses `_callable`/`_callable_class`. Calls to the shadowed definition disappear.

### 10.2 Edges

| Relation | Direction | Confidence | Context | When |
|---|---|---|---|---|
| `contains` | file → top-level symbol; function/method → nested function | EXTRACTED | none; `namespace_export` for `export * as X` | every non-method symbol. Namespaces do **not** contain their members. |
| `method` | class/interface → method | EXTRACTED | — | every method |
| `case_of` | enum → member | EXTRACTED | — | |
| `inherits` / `implements` | class → base / interface | EXTRACTED | `type` | the base resolves to a same-file or imported symbol (generic args ignored). No edge and no stub for external (`React.Component`), qualified or call bases. |
| `imports_from` | file → module (the **function** for `require`/`import()` inside a function) | EXTRACTED | `import`; `re-export` | one per (source, target); first statement wins. `type_only: true` if that statement is `import type`. `deferred: true` for `import()` inside a function. |
| `imports` | file → imported symbol | EXTRACTED | `import` | each resolved named, default or destructured-require binding |
| `re_exports` | file → symbol | EXTRACTED | `re-export` | `export { a as b } from` (target is the original `a`) |
| `re_exports` | file → module | EXTRACTED | `export` | `export *`, `export * as`, `export {…} from`, `export type {…} from` |
| `dynamic_import` | file → module | EXTRACTED | none | every `import("…")`, including JSDoc `import('x')` types. No `source_location`, no `weight`. |
| `calls` | caller → callee | EXTRACTED / INFERRED 0.85 | `call` | §10.4 |
| `indirect_call` | enclosing function, **or the file** at top level → function | INFERRED 0.85 | `argument`, `collection`, `return` | a function identifier passed as an argument, placed in an object or array literal (`module.exports = { helper }`), or returned. Classes never qualify. |
| `references` | class (`field`) or **method** (`parameter_type`, `return_type`, `generic_arg`) → type | EXTRACTED | as listed | only for types **imported from another file** and only from class or interface members. Never from top-level functions, never to same-file types. |
| `references` | decorated class → decorator function | EXTRACTED | `decorator` | decorator line |
| `rationale_for` | rationale → **file** node | EXTRACTED | — | always the file |

There is no `uses` and no `AMBIGUOUS` in TS/JS (fixtures, excalidraw, monet, erpnext).

### 10.3 Import forms and resolution

| Form | Edges |
|---|---|
| `import "./polyfill"` | `imports_from` |
| `import X from "./m"` | `imports_from` + `imports` → **m's default export**, whatever the local name. Nothing if m has no resolvable default. |
| `import { a, b as c } from "./m"` | `imports_from` + `imports` → `m_a`, `m_b` |
| `import * as ns from "./m"` | `imports_from` only |
| `import type {…}` | as named; the whole-statement form sets `type_only` |
| `import("./m")` | top level: `dynamic_import` only. Inside a function: `imports_from` (`deferred: true`) from the function plus `dynamic_import`. |
| `const x = require("./m")` | `imports_from` + value node `x` |
| `const { a, b } = require("./m")` | `imports_from` + `imports` → `m_a`, `m_b` (stubs if missing) + value node `{ a, b }` |
| `export * from "./m"` | `imports_from` (`re-export`) + `re_exports` → m (`export`) |
| `export { a as b } from "./m"` | `imports_from` (`re-export`) + `re_exports` → `m_a` (`re-export`) + `re_exports` → m (`export`) |
| `export { a }`, `export default X`, `export { default } from` | nothing |

**Specifier resolution:**

- Relative specifiers try, in order:
  1. the exact path;
  2. the path plus each code extension;
  3. `.js`→`.ts` substitution;
  4. the directory index (`./models` → `models/index.ts`).
- `tsconfig.json` `paths` and `baseUrl` are honoured. With no tsconfig, `@/x` resolved to `src/x`
  (**Open**).

**Unresolved targets:**

- **Bare package** → external `ref_<slug(package)>`, where the package name is the first segment, or
  the first two for `@scope/…`: `react` → `ref_react`, `node:path` → `ref_node_path`,
  `@scope/other-pkg` → `ref_scope_other_pkg`.
- **Relative path to a missing file** → `ref_<slug(specifier without leading ./ and ../)>`.
- **Relative path to an existing non-code file** → `slug(resolved path without extension)`
  (`sub_style`; excalidraw has 332 font stubs).
- **Symbol stubs:** a named binding not defined in the resolved local module → stub
  `module_id_slug(name)` with an `imports` edge.

**Barrels (quirk):** importing a name that a barrel re-exports first creates the stub
(`src_models_index_user`). A later pass then adds the real `imports` edge
(`src_service -imports-> src_models_user_user`) but never removes the stub. When two `export *`
sources define the same name, the first wins.

**Dynamic-import phantoms (quirk):** an `import()` target that is not a local code file becomes a
`file_type: "code"` node with `confidence: "EXTRACTED"`, no `source_location`, `label` = specifier,
and `source_file` = specifier.

- The id is `slug(last path segment)` for bare specifiers (`@scope/pkg-types` → `pkg_types`).
- So `ref_lodash` and `lodash` can coexist.
- These nodes are lost on warm rebuilds (§8.4).

### 10.4 Calls

A call is attributed to the nearest enclosing **named** function or method. Anonymous callbacks are
transparent: `viaArrow() { [1].forEach(x => fnB(x)) }` → `viaArrow -calls-> fnB`. Top-level calls and
calls inside `describe`/`it` callbacks are **dropped**. There is one edge per (caller, callee), at the
first call line.

**Same-file pass (EXTRACTED, no score):**

- `name()`, `new Name()` and JSX `<Name/>` resolve to the same-file function or class. Lowercase
  intrinsic tags are ignored.
- `this.m()` resolves to the enclosing class's `m`.
- **Quirk:** `anything.m()` with a non-`this` receiver binds to the **last-defined method named `m` in
  the same file**. This makes self-loops such as `src_service_userservice_log -calls->` itself, from
  `console.log` inside `UserService.log`. The call is then consumed and is never typed-resolved.

**Import-scoped pass for top-level functions (EXTRACTED, no score):** `name()` where `name` is an
imported binding (named, aliased or default) resolves to the imported symbol.

**Cross-file pass (score present):**

- EXTRACTED 1.0:
  - calls from **methods** to imported functions;
  - `new X()` and `<X/>` of imported classes or components;
  - `name()` where `name` is defined in a module the file imports but is not imported by name.
- Names in modules the file does not import are never resolved.
- INFERRED 0.85: `recv.m()` where `recv`'s class is known from a parameter annotation of an imported
  class or from `const s = new Svc()`, and that class itself defines `m`. Inherited methods are not
  resolved.
- Dropped:
  - namespace-qualified calls (`ns.helper()`, `Storage.key()`);
  - calls on external modules (no `calls` edge to `ref_*`, unlike Python);
  - calls through destructured-require aliases;
  - `super.m()` and `super()`;
  - unknown names.

### 10.5 Rationale (TS and JS)

A rationale node needs a line that, after indentation, starts with **exactly** `// ` or `* ` (a
JSDoc continuation line), followed by `NOTE|WHY|TODO|FIXME|HACK|IMPORTANT|RATIONALE` and `:`.

- The label drops the prefix (`NOTE: …`) and uses the first line only, with 80-character
  truncation.
- JSDoc blocks are **not** docstrings.
- `rationale_for` always targets the file node.

### 10.6 Clustered consequences

- `dynamic_import` folds into `imports_from` when both join a pair. Alone, it survives with
  `confidence_score: 1.0` and no `context`, `source_location` or `weight`.
- A file-level `indirect_call` (`module.exports = { helper }`) replaces the file's `contains` edge to
  `helper`.
- A function calling its nested function keeps `contains`, with `context: "call"`.

---

## 11. Rust

Fixture: `tests/fixtures/rust/` (Cargo.toml, `src/{lib,main,model}.rs`, `src/util/{mod,fmt}.rs`,
`tests/it.rs`). Goldens: `rust.graphify.json` (raw, `--no-cluster`) and
`rust.clustered.graphify.json`. Confirmed at scale against the ripgrep corpus graph (3,622 `.rs`
nodes, 8,711 `.rs` edges). Every rule below was isolated with a dedicated probe repo; probe
names are not kept.

Notation: `F` = file-node id of the file being extracted (e.g. `src/util/mod.rs` → `src_util_mod`).
`slug` is defined in §4.1 (Unicode letters are kept: `fn üñí` → `src_lib_üñí`; `r#match` →
`r_match`).

### 11.1 Rust node kinds

All Rust nodes carry `file_type: "code"`, `_origin: "ast"`, `source_file`, `source_location`
(`"L<n>"`, 1-based line of the item keyword: attributes and doc comments above it do not count;
`#[derive] struct Circle` on L15/L16 gives `L16`). Rust nodes **never** carry `_callable` or
`_callable_class` (these are Python/JS-only).

| Construct | id | label | extra fields | parent edge |
|---|---|---|---|---|
| file | `F` | basename (`mod.rs`); raw output disambiguates duplicate basenames with a path suffix (`x/dupe.rs`) | – | – |
| `struct` (named, tuple, unit), `enum`, `trait` | `F_slug(name)` | `Name` | `_rust_declaration_count` | `contains` from `F` |
| `union`, `type` alias, trait associated `type` | **not emitted** | | | |
| enum variant | `F_enum_variant` (`src_model_kind_round`) | `Round` | – | `case_of` from enum |
| free `fn` (incl. `async`, `const`, `unsafe`, `pub(crate)`, `extern "C" { fn }`) | `F_slug(name)` | `name()` | – | `contains` from `F` |
| method / associated fn in `impl` or `trait` (incl. default methods) | `<owner>_slug(name)` | `.name()` | – | `method` from owner |
| associated `const` in impl/trait | `<owner>_slug(NAME)` (`src_model_circle_unit`) | `.UNIT` | – | `contains` from owner |
| module-level `const` / `static` | `F_slug(NAME)` (`src_lib_max_shapes`) | `MAX_SHAPES` | – | `contains` from `F` |
| `macro_rules! sq` | `F_macro_slug(name)` (`src_lib_macro_square`) | `square!` | – | `contains` from `F` |
| impl owner that is not an existing node (generic impl, foreign type) | `F_slug(type text)` | type text verbatim | `_rust_impl_key` (sometimes) | **none** (floating; see below) |
| `Cargo.toml` `[package]` | `pkg_slug(name)` (`pkg_shapes`, `pkg_b_crate`) | package name verbatim (`b-crate`) | `type:"package"`, `ecosystem:"cargo"`, `version` | none |

Not emitted at all: nested `fn`s inside fn bodies (`pick` in `classify`), items declared inside
fn bodies (local structs/enums/impls), closures, `mod` items themselves, `extern crate`,
`use` inside a fn body, doc comments (`///`, `//!`) and `// NOTE:` / `// WHY:` comments. **Rust
emits no `rationale` nodes** (ripgrep: all 56 rationale nodes come from a Python script).

Inline `mod` blocks (including `#[cfg(test)] mod tests`) are flattened into the file scope: their
items get `F_name` ids and a `contains` edge from `F` (`src_model_fixture`,
`src_model_area_is_positive`; `inner::helper` → `src_util_mod_helper`). Test files (`tests/it.rs`)
and `#[test]` fns are ordinary nodes; there is no test-specific handling.

**Duplicate ids: first definition wins.** Flattening makes `util::inner::helper` (L7) and
`util::inner::deeper::helper` (L13) both `src_util_mod_helper`; only the L7 node exists and every
call to either resolves to it. Same for two `#[cfg]` variants of `fn plat()` (one node, first
line), and for a method whose slug equals an enum variant's (`enum Plain { B(u8) }` +
`impl Plain { fn b() }` → one node `src_lib_plain_b`, label `B`, line of the variant, carrying both
`case_of` and `method` edges). *Quirk:* the raw output keeps the first duplicate edge's
attributes, the clustered output keeps the last (two `impl T { fn go }` blocks on L3/L5: node is
`L3` in both outputs, the `method` edge is `L3` raw and `L5` clustered).

`_rust_declaration_count` (int) appears on every struct/enum/trait node: the number of
same-name struct/enum/trait declarations in the file (`#[cfg(unix)] struct Platform;` +
`#[cfg(not(unix))] struct Platform;` → one node `src_model_platform`, count `2`, line of the first).
It does not count `impl` blocks (`Circle` with three impl blocks → `1`). ripgrep: 373 nodes, all `1`.

#### Impl owners

The owner id of an `impl` block is `F_slug(<type text after for/impl, generics included>)`.

- `impl Circle` in the file that declares `Circle` → `src_model_circle`, i.e. the struct node;
  methods attach to it. Multiple impl blocks merge.
- `impl<T: Shape + Clone> Wrapper<T>` → new node `src_model_wrapper_t`, label `Wrapper<T>`,
  line of the `impl`. Lifetimes are part of the text (`Life<'a>` → `src_lib_life_a`,
  `PreludeWriter<'a, M, W>` → `..._preludewriter_a_m_w`, `Box<dyn std::error::Error>` →
  `..._box_dyn_std_error_error`).
- `impl Shape for Circle` in a file that does not declare `Circle` → new node `src_b_circle`,
  label `Circle`, no `_rust_declaration_count`. The type now exists as two unrelated nodes.
- These created owner nodes get **no `contains` edge** and no link to the struct they extend.
  *Quirk worth not reproducing in dijkstral-native data; required for parity.*
- `_rust_impl_key: "<Name>/<n>"` is added to a created owner node only for an **inherent** impl
  whose generic list is plain type params with no bounds, no lifetimes and no `where` clause; `n`
  is the number of type params: `impl<T> Pair<T>` → `Pair/1`; `impl<T, U> C<T, U>` → `C/2`;
  `impl<X> H<X>` → `H/1`. None for `impl<T: Clone> B<T>`, `impl<'a, T> D<'a, T>`,
  `impl<T> E<T> where T: Clone`, `impl<T> Tr for G<T>`, `impl Gen<u8>`. No later stage merges
  keyed nodes into the type (ripgrep keeps `Match<T>` beside `Match`).

### 11.2 Stub nodes

Two different stub shapes exist; implementers must keep them apart.

**External import stub** (from `use`):
`{"id": slug(seg), "label": slug(seg), "file_type": "concept", "type": "external", "external": true, "source_file": ""}`
— no `source_location`, no `_origin`. Raw key order exactly as shown.

**Type-reference stub** (from a type name not declared in the same file):
`{"id": slug(Name), "label": "Name", "file_type": "code", "source_file": "", "source_location": "", "_origin": "ast"}`
(`registry`/`Registry`, `formatter`/`Formatter`, `t`/`T`, `f`/`F`).

Rules, all observed:

1. If a type-reference stub id and an external import stub id coincide, the type-reference shape
   wins regardless of file order (`use std::io::Read; fn a(c: &dyn Read)` → node `read`, label
   `Read`, `file_type: "code"`).
2. **File-scoping:** if the same type-reference stub id is produced by more than one file, each
   file gets its own stub `slug(<source_file path incl. extension>)_slug(Name)`:
   `Settings` used in `src/util/fmt.rs` and `src/util/mod.rs` → `src_util_fmt_rs_settings` and
   `src_util_mod_rs_settings`. Note `_rs_`: unlike file ids, the extension is kept. External
   import stubs are never file-scoped. ripgrep: `crates_cli_src_process_rs_read`.
3. **Resolution:** after file-scoping, a stub whose label equals the label of exactly one
   declared struct/enum/trait node is deleted and every edge pointing at its id is redirected to
   that node (EXTRACTED, no `confidence_score`). With 0 or ≥2 candidates the stub stays
   (`Match` in ripgrep has 5 declarations → stubs remain). Candidates are restricted by language:
   a Rust stub does not resolve to a Python class in a `.py` file, but *does* resolve to one in an
   extensionless script (`benchsuite/benchsuite`; ripgrep `use std::process::Command` →
   `benchsuite_benchsuite_command`). *Quirk.*
4. **Import rewiring is a side effect of rule 3.** Import edges point at `slug(last segment)`.
   When a *global* (non-file-scoped) type stub with that same id is resolved, the import edge is
   redirected with it; otherwise it stays on the external stub. The path is ignored entirely:
   `use std::fmt::Circle` resolves to the repo's `Circle` if any one other file mentions `Circle`
   as a type. Consequences in the fixture: `use shapes::{.., Shape}` in `main.rs` →
   `src_model_shape` (only `lib.rs` uses `Shape` as a type); `use shapes::model::{.., Circle}` →
   external `circle` (two files use `Circle` as a type → file-scoped → not shared). *Quirk:
   reproduce for parity, but this is name coincidence, not module resolution.*

**Clustered-build file-stem remap** (clustered/`update` output only; raw `--no-cluster` is never
remapped). An external stub whose label equals the filename stem of exactly one file node is
deleted. Incoming import edges from a file of the same language as that file are re-pointed to the
file node; incoming edges from other languages are **dropped**. With 0 or ≥2 matching files the
stub and its edges stay. Observed:

| import (importer) | files present | clustered result |
|---|---|---|
| `use std::fmt;` (`src/model.rs`) | `src/util/fmt.rs` | `src_model → src_util_fmt` (wrong: this is `std::fmt`) |
| `use super::model::*;` (`src/util/mod.rs`) | `src/model.rs` | `src_util_mod → src_model` |
| `use crate::helpers;` | `src/helpers.rs` | → `src_helpers` |
| `use std::io::Read;` (no `Read` type ref) | `src/read.rs` | → `src_read` |
| `use crate::dupe;` | `src/x/dupe.rs`, `src/y/dupe.rs` | external `dupe` kept |
| `use crate::util;` | `src/util/mod.rs` | external `util` kept (stem is `mod`) |
| `use crate::zork;` | only `py/sub/zork.py` | edge and stub dropped |
| `import httpx` (Python) | only `src/httpx.rs` | edge and stub dropped |
| `use crate::zork;` + Python `import zork` | `zork.py` and `zork.rs` | external `zork` kept, both edges kept |
| `use crate::h as hh` | `src/helpers.rs` | `h_as_hh` kept |

ripgrep: 21/21 such stubs remapped (`hostname`, `path` → `crates_printer_src_path` across crates,
`version`). *Quirk worth not reproducing natively.*

### 11.3 `use` forms → `imports_from` edges

Every Rust import edge is `relation: "imports_from"`, `context: "import"`, EXTRACTED, weight 1.0,
`source` = `F`, `source_location` = line of the imported leaf itself (in a multi-line group each
leaf reports its own line: `use std::{\n ffi::OsStr,\n io::{\n self,\n Read }}` → L2, L4, L5). Rust never emits `imports` or `re_exports`; `pub use` / `pub(crate) use` are treated
like `use`. One edge per imported leaf, target = stub id of the last path segment:

| form | target id(s) |
|---|---|
| `use std::borrow::Cow;` | `cow` |
| `use std::collections::{HashMap, HashSet};` | `hashmap`, `hashset` |
| `use std::{ffi::OsStr, io::{self, Read}, path::Path};` | `osstr`, `io` (`self` → parent segment), `read`, `path` |
| `use crate::model::{Shape, Circle as Round};` | `src_model_shape` (via rule 4), `circle_as_round` |
| `use self::fmt::render as render_shape;` | `render_as_render_shape` |
| `use a::b::{self as bee};` | `b_as_bee` |
| `use x::y::Z as _;` | `z_as` (trailing `_` trimmed) |
| `use super::model::*;` / `use super::*;` | `model` / `super` (segment before `*`) |
| `use q::{r::*, s::T as Tee};` | `r`, `t_as_tee` |
| `use crate::model;` | `model` |
| `use ::absolute::Thing;` | `thing` |
| `use Single;` | `single` |
| `use` inside an inline `mod` | emitted on `F` at its own line |
| `use` inside a fn body, `extern crate x [as y];`, `mod x;` | nothing |

There is no module-path resolution: `crate::`, `self::`, `super::`, the crate name (`shapes::`)
and external crates are all handled identically, and `mod model;` declarations produce no edge.

### 11.4 Edges

All Rust AST edges have `weight: 1.0` and `_origin: "ast"`. In-file edges have no
`confidence_score` in raw output; edges produced by the cross-file pass carry one. (The clustered
output adds `confidence_score` to every edge.)

| relation | direction | conf / score | context | source_location | emitted when |
|---|---|---|---|---|---|
| `contains` | file → top-level item; type → associated const | EXTRACTED | – | item line | always |
| `method` | owner → method | EXTRACTED | – | fn line | fn in `impl` / `trait` |
| `case_of` | enum → variant | EXTRACTED | – | variant line | each variant |
| `implements` | impl owner → trait | EXTRACTED | – | `impl` line | `impl Trait for T` where `Trait` is a bare identifier not on the std-trait denylist; path traits (`fmt::Display`, `std::fmt::Display`) give nothing; target resolved like a type stub (`ext`, `src_a_shape`) |
| `inherits` | trait → supertrait | EXTRACTED | – | trait line | `trait Named: Shape + Sized` → only `Shape` (same denylist; ripgrep `Flag: Debug + Send + Sync + UnwindSafe + RefUnwindSafe` → `unwindsafe`, `refunwindsafe`) |
| `references` | fn → type | EXTRACTED | `parameter_type` / `return_type` / `generic_arg` | **fn signature start line** for all of them | type name in params/return; `generic_arg` when nested inside `<...>` of another type (`Vec<X>`, `Result<Z, E>`, `Box<dyn Shape>`, `impl Into<Thing>`, `dyn Fn(A) -> B`); `&T`, `&mut T`, `*const T`, `[T; n]`, `&[T]`, tuples, `fn(A) -> B` keep `parameter_type`/`return_type` |
| `references` | struct/enum/const/static → type | EXTRACTED | `field` (or `generic_arg` when nested) | field / variant / item line | struct fields, tuple-struct fields, enum variant fields (source is the **enum**, not the variant), `const`/`static` type annotations |
| `references` | fn → variant / tuple struct | EXTRACTED | `constructor` | call line | in-file `Enum::Variant(..)` / `Self::Variant(..)` call or tuple-struct call `Meters(..)`. Unit variants (`Kind::Empty`), struct literals (`Circle { .. }`) and patterns give nothing |
| `references` | fn → tuple struct | INFERRED 0.85 | `constructor` | call line | bare `Meters(x)` where `Meters` is declared in another file and unique (`src_util_mod_widen → src_model_meters`) |
| `calls` | fn → fn/method/macro | EXTRACTED | `call` | call line | in-file resolution, below |
| `calls` | fn → fn/method | INFERRED 0.85 | `call` | call line | cross-file bare-name resolution, below |
| `depends_on` | package → package | EXTRACTED 1.0 (score present in raw) | `dependency` | `L1` | `Cargo.toml`, below |

Never emitted for Rust: `imports`, `re_exports`, `uses`, `indirect_call`, `rationale_for`.
ripgrep `.rs` histogram (clustered, after collapse): calls/EXTRACTED 2306, method 2055, contains
1145, imports_from 803, references parameter_type 767 / generic_arg 393 / return_type 322 /
field 259 / constructor 114 (+1 INFERRED), case_of 250, implements 167, calls/INFERRED 126,
inherits 2.

Type references skip: primitives, `Self`, generic params' bounds and `where` clauses, types used
only in fn bodies (`let v: T`, turbofish), and a std denylist. Observed **skipped**: `String Vec
Option Result Error Box HashMap HashSet BTreeMap BTreeSet VecDeque Cow Path PathBuf Arc Rc RefCell
Cell Weak Pin Duration Instant SystemTime OsStr OsString CString CStr Ordering PhantomData Range
Self Iterator Debug Display Clone Default Send Sync Hash Eq str Sized Fn FnMut FnOnce Future`.
Observed **kept** (become stubs): `Mutex RwLock File Formatter NonNull Regex Value Bytes Any Read
Write Stream Utf8Error ParseIntError Command Stdio Child`. Generic type parameters are **not**
filtered: `T`, `U`, `F` become global stubs `t`, `u`, `f` (*quirk*). Qualified paths use the last
segment (`std::fs::File` → `file`, `crate::m::Qualified` → `qualified`, `fmt::Formatter` →
`formatter`). The std-trait denylist used by `implements`/`inherits` is `Clone Copy Debug Default
Display From Into Iterator IntoIterator Drop PartialEq Eq Hash Error Send Sync AsRef AsMut Deref
DerefMut Ord PartialOrd Future Fn FnMut TryFrom TryInto Extend Sized` (kept: `Read Write FromStr
Serialize Deserialize Visitor Borrow ToString FromIterator Unpin Any UnwindSafe RefUnwindSafe Add
Sub Index BufRead Seek`). Both lists are partial: only these names were probed.

Per-file dedupe: one edge per `(source, target, relation)`, first occurrence wins (so
`diameter → Meters` keeps `return_type` and drops the `Meters(..)` constructor). Different
relations for the same pair are all kept in raw output (`Plain → plain_b` as both `case_of`
and `method`) and collapse later in the clustered build.

Edge emission order within a file (raw): structural edges (imports, contains, method, case_of,
implements, inherits, signature/field references) in source order, then all call and constructor
edges of the file; cross-file INFERRED edges come after all files.

### 11.5 Call resolution

Calls inside macro invocations are invisible (`println!("{}", render(&c))`,
`vec![helper()]`, `assert!(fixture().area() > 0.0)`, `assert_eq!(classify(..), ..)`). Calls to
closures, local `let f = twice; f()`, nested fns and fn pointers passed by name
(`apply(twice)`, `.map(twice)`, `spawn(worker)`) produce nothing. Unresolved calls produce no edge
and no stub (`missing_function()`). An edge is created once per `(caller, callee)`, at the first
call line.

**In-file (EXTRACTED):**

- Bare `name()` → the file's free fn `name` (inline-module fns included). Names on a builtin
  denylist never resolve in-file even when defined: observed `set len open print filter map next
  iter hash sum min max abs id` (these are Python builtins: *quirk*). `build`, `new`, `run`,
  `get` etc. do resolve in-file.
- Module-qualified `a::b::name()` (lowercase qualifier, incl. `crate::`, `super::`, unknown
  `xyz::`) → treated as bare `name()` against free fns of this file (`inner::deeper::helper()` →
  `src_util_mod_helper`); never cross-file (`shapes::util::run_all()`, `crate::esc::escape()` →
  dropped).
- Type-qualified `Type::name()` / `Self::name()` → method `name` of `Type` only if `Type` is
  declared in this file (`Circle::new` in `model.rs` → `src_model_circle_new`; `Other::make()`,
  `Local::helper()` with no such method, and cross-file `Circle::new` → dropped).
- Method syntax `recv.name()` (any receiver, `self` included) → the file's method `.name()` if the
  name is unique among the file's methods (`anything.twice_m()` resolves; `self.area()` in
  `model.rs` is dropped because `.area()` exists on `Shape`, `Circle` and `Wrapper<T>`).
- Macro invocation `name!(..)` → the same file's `macro_rules! name` node
  (`m1 → src_a_macro_sq`). Path-qualified (`crate::square!`) and cross-file macro invocations →
  nothing.

**Cross-file pass (INFERRED, `confidence_score: 0.85`):** only bare `name()` calls not resolved
in-file. The callee is the unique node anywhere in the repo, in a file of the same language
(Rust → `.py` was not resolved), whose label is `name()` or `.name()`: free fns and methods
compete (`gogo()` → `src_a_s_gogo`; a free `ff()` in one file plus a `.ff()` method elsewhere →
dropped). `use` statements are irrelevant (`unescape()` resolves without an import). Two or more
candidates → dropped (`render`, `helper`, `twin`). A denylist of common names is never resolved
cross-file; observed: `build new run get set init parse from into len default clone start open
read write print send push pop remove insert filter map next iter to_string is_empty contains
hash sum min max abs unwrap expect ok err some none id lock to_owned` (resolved fine: `main label
twice apply escape helper render update create handle load save find merge drop fmt` …; list
partial). Method-syntax and qualified calls are never resolved cross-file. Rust cross-file calls
are always INFERRED: ripgrep has 0 cross-file EXTRACTED calls and 124 cross-file INFERRED; its 2
same-file INFERRED calls are bare calls to a same-file *method* (`close → .drop()`), which only the
cross-file pass resolves.

Fixture results: `main → total_area`, `main → round_kind` (INFERRED); `build(..)` dropped
(denylist), `Circle::new`, `Round::new`, `r.area()` dropped (qualified/method cross-file),
`label`/`render` inside `println!` invisible, `shapes::model::classify(..)` dropped.

### 11.6 Cargo.toml (in `--code-only`)

`Cargo.toml` counts as a code file (the extractor prints "yielded no symbols" for it yet emits a
node). For each `Cargo.toml` with `[package] name`: node `pkg_slug(name)`, raw key order
`id, label, file_type, type, ecosystem, source_file, source_location, version, _origin`;
`version` is omitted unless it is a literal string (`version.workspace = true` → absent). A
workspace-only root (`[workspace]` without `[package]`) yields no node. No `contains` or other
edge links the package to its source files.

`depends_on` edges come only from `[dependencies]` (not `[dev-dependencies]`,
`[build-dependencies]`, `[workspace.dependencies]`), matched by **dependency key** against
package names in the repo, with or without `path`: `b-crate = { path = "../b-crate" }` and
`gamma = "0.1"` both link to the repo's packages; `serde`, `memchr` (not in the repo) produce
nothing. Raw edge: `{"source","target","relation":"depends_on","context":"dependency","confidence":"EXTRACTED","confidence_score":1.0,"source_file":"a/Cargo.toml","source_location":"L1","weight":1.0,"_origin":"ast"}`.
ripgrep: 11 package nodes, 16 `depends_on` edges.

### 11.7 Rust quirks summary (reproduce for parity, flag in dijkstral-native data)

1. Import resolution is name coincidence via type-stub ids (rule 4), not module resolution.
2. The clustered build's file-stem remap rewires `use std::fmt` to a repo file `fmt.rs` and drops
   cross-language import edges.
3. Generic-impl and foreign-impl owner nodes are disconnected duplicates of the real type.
4. Flattened inline modules plus first-wins dedupe merge distinct functions
   (`inner::helper`/`deeper::helper`) and methods with same-slug variants (`b`/`B`).
5. Generic type parameters become global stub nodes (`t`, `u`, `f`).
6. Python builtin names (`len`, `map`, `iter`, `min` …) are not resolved even for Rust in-file calls.
7. Raw and clustered outputs keep different duplicate edge attributes (first vs last).
8. Extensionless scripts are resolution candidates for Rust type stubs.

### 11.8 Open questions (Rust)

- Full contents of the three denylists (type references, std traits, cross-file call names).
  Only the names listed were probed.
- Whether stub resolution (rule 3) also accepts candidates from `.ts`/`.go`/`.java` files, and the
  exact "same language" grouping used by both resolvers.
- Exact `slug` behaviour for non-ASCII punctuation and for digits-only segments was not probed.

---

## 12. Go

Fixture: `tests/fixtures/go/` (module `example.com/shop`; packages `store/`, `service/`, `cmd/app/`).
Goldens: `go.graphify.json` (raw `--no-cluster`), `go.clustered.graphify.json`. Rules below were
isolated with extra probes; the probe snippets are quoted inline.

Notation: `slug` as in §4.1. `dir` = the **last path component of the directory containing the file**; for a file at
the scan root it is the **basename of the scan root directory itself** (probe: `root.go` with
`type R` scanned from `.../p1` gave `p1_r`). `fileid` = the file node id (`store/helpers.go` →
`store_helpers`).

### 12.1 Nodes

| Construct | id | label | source_location | extra fields |
|---|---|---|---|---|
| file | `fileid` | basename (`helpers.go`) | `L1` | — |
| `type X struct/interface/<named>` | `slug(dir)_slug(X)` → `store_id`, `service_user`, `store_entity` | `X` | line of the type spec | — |
| func `F` | `fileid_slug(F)` → `store_helpers_normalize`, `service_service_newuserservice` | `F()` | line of `func` | — |
| method `func (r T) M()` / `(r *T)` | `slug(dir)_slug(T)_slug(M)` → `service_userservice_register`, `store_model_identity` | `.M()` | line of `func` | — |
| interface method `M()` | `<interface id>_slug(M)` → `service_greeter_greet` | `.M()` | line of the method spec | — |
| generic receiver `func (s *Store[T]) Put` | method `store_store_t_put`; **plus** a separate receiver node `store_store_t` label `Store[T]` (see quirk G3) | | | |
| type parameter used as a type (`T`, `N`) | `slug(T)` → `t`, `n` (global, shared across files and languages) | `T` | `""` | `source_file: ""` |
| qualified external type `http.Request`, `testing.T` | `go_type_` + `slug(import path)` + `_` + `slug(Name)` → `go_type_net_http_request`, `go_type_testing_t` | `net/http.Request` (import path + `.` + Name) | `""` | `source_file: ""`, `file_type: "code"`, `_origin: "ast"` |
| external import | `go_pkg_` + `slug(import path)` → `go_pkg_fmt`, `go_pkg_github_com_google_uuid` | **same as id** (quirk G6) | absent | `file_type: "concept"`, `type: "external"`, `external: true`, `source_file: ""`, no `_origin` |
| `go.mod` | `pkg_` + `slug(module path)` → `pkg_example_com_shop` | module path `example.com/shop` | `L1` | `type: "package"`, `ecosystem: "go"`, `source_file: "go.mod"`; no edges; `require` lines produce nothing; stderr warns "yielded no symbols ... go.mod" |

Raw key order: `id, label, file_type, [type, ecosystem,] source_file, source_location, _origin`.
External: `id, label, file_type, type, external, source_file`.

**No Go node ever carries `_callable` or `_callable_class`** (contrast Python/Java). No
`metadata`, no `node_kind`.

Not emitted at all (verified): `const`, `var` (incl. `var ErrNotFound`), type aliases
`type Key = string` / grouped `G2 = int`, closures / func literals (`run := func(){}`), struct
fields, and **any comment** — Go produces **no rationale nodes** (package doc, `// NOTE:`,
`// WHY:`, `// TODO:`, `// HACK:`, `/* WHY: */` all ignored; probe `p6`). Grouped `type ( G1 struct{} )`
is emitted normally.

#### Cross-package type re-keying (quirk G1)

A type defined in package dir `p` that is referenced **as a type through a package qualifier
`p.T` from another package** (embed, struct field, parameter, return, generic arg) is re-keyed to
`slug(source_file of definition)` + `_` + `slug(dir)` + `_` + `slug(T)`:

```
p/types.go: type C struct{}       q/use.go: func Param(c p.C) {}
→ node id p_types_go_p_c (label C), q_use_param -references-> p_types_go_p_c
```

Types only used in composite literals (`p.K{}`), conversions or `var x p.T` keep `p_k`. Methods
of a re-keyed type keep the un-re-keyed prefix: `p_types_go_p_m -method-> p_m_hi`. Fixture:
`store_store_go_store_model` (embedded by `service.User`), `store_store_go_store_store`.

#### Id collisions (quirk G2)

Go ids are not disambiguated. When two constructs produce the same id the node is stored once, in
the **position of the first insertion** with the **attributes of the last** writer:

- file `x/x.go` (`x_x`) vs `type X` in package dir `x` (`x_x`): one node, label `X`,
  `source_location` of the type; the file's `contains` edges now originate from the type, and a
  self-loop `x_x -contains-> x_x` is emitted (it survives clustering). Fixture:
  `store_store_go_store_store` (file `store/store.go` + `type Store`, then re-keyed by G1) has
  self-loop `contains` at `L39` and every `contains`/`imports_from` of the file hangs off it; the
  file node `store/store.go` no longer exists as a file node.
- method `func (v X) Do` (`x_x_do`) vs func `Do` in file `x/x.go` (`x_x_do`): one node,
  label `Do()`, location of the later func; both `method` and `contains` edges kept in raw.

### 12.2 Edges

All Go edges have `weight: 1.0`, `_origin: "ast"`, `source_file` = file of the source construct.

| relation | source → target | when | confidence | `context` | source_location |
|---|---|---|---|---|---|
| `contains` | file → top-level type / func | always | EXTRACTED | none | definition line |
| `method` | receiver type id → method; interface → interface method | always (target type need not exist; id is computed) | EXTRACTED | none | method line |
| `embeds` | struct/interface → embedded type (`store.Model`, `*Base`, `io.Reader`, interface `R`) | embedded field / embedded interface | EXTRACTED | none | line of the embedded field |
| `references` | struct → field type | named field type resolvable to local type, type param, or `go_type_*` stub | EXTRACTED | `field` | field line |
| `references` | func/method → type | parameter / return / generic argument | EXTRACTED | `parameter_type` / `return_type` / `generic_arg` | the func line |
| `imports_from` | file → `go_pkg_*` external | stdlib or unresolved import path | EXTRACTED | `import` | line of that import spec |
| `imports_from` | file → **every file node** in the package dir | import path = `<go.mod module>/<dir path>` | EXTRACTED | `import` | import spec line |
| `calls` | func/method → func/method/type | see resolution | EXTRACTED / INFERRED | `call` | call line |

There is **no** `implements` edge (implicit interface satisfaction is not modelled), no
`indirect_call` (function values passed as arguments or stored in fields — `apply(names,
strings2upper)`, `log: logLine` — produce nothing), no `uses`, no `rationale_for`, no
`re_exports`, no `imports` (Go uses only `imports_from`).

References are deduplicated per (source,target) within a function (first context wins:
`service_userservice -references-> store_store_go_store_store` `field`, then `service_user`
`generic_arg` for `*store.Store[User]`). Unqualified builtin types (`string`, `int`, `error`,
`map`, slices) produce no reference. `references` raw key order puts `context` after `weight`
(`... source_location, weight, context, _origin`); `calls`/`imports_from` put it after `relation`.

#### Import forms (fixture `service/service.go`, `cmd/app/main.go`)

| form | result |
|---|---|
| `"fmt"`, grouped `"sort"` | `imports_from -> go_pkg_fmt` |
| `"example.com/shop/store"` (module-local) | `imports_from` to each file node in `store/`: `store_helpers`, `store_store_go_store_store` |
| `. "example.com/shop/store"` | same targets — deduplicated against the earlier plain import (no new edge) |
| `_ "github.com/google/uuid"` and `uuidlib "github.com/google/uuid"` | one `imports_from -> go_pkg_github_com_google_uuid` at the first spec's line (L10) |
| `str "strconv"` | `go_pkg_strconv` |
| module-local path without a matching dir (`ex.com/m/nope`) | `go_pkg_ex_com_m_nope` external |
| **no `go.mod`** | every non-stdlib import is external (`"foo/lib"` → `go_pkg_foo_lib`) and package-qualified calls into it are dropped |

Resolution is by module prefix + directory path (`ex.com/m/internal/store` → `internal/store/*`,
`ex.com/m/v2/thing` → `v2/thing/*`), not by package clause. `_test.go` files of the package are
included: `cmd_app_main -imports_from-> service_service_test` (quirk G4).

#### Call resolution

Observed (fixture + probe `p4`):

1. **Same file, bare call** `Helper()` → EXTRACTED, **no `confidence_score`** (`lib_a_caller -calls-> lib_a_helper`). Calls to a type name (conversion `ID(x)`) also produce `calls` to the type: `store_helpers_normalize -calls-> store_id`.
2. **Same file, method call on any receiver** `t.Only()` / `item.Identity()` → resolved by **bare method name among methods defined in the same file**, ignoring the receiver's type; when several types in the file define the name, the **last-defined** one wins (`t.Do()` with `t T1` → `lib_t2_do`; `item.Identity()` → `store_model_identity`, not the interface method). EXTRACTED, no score. Quirk G5.
3. **Same package, other file, bare call** `OtherFile()`, `Dup()` → INFERRED, `confidence_score: 0.85` (`service_service_test_testregister -calls-> service_service_newuserservice`). The candidate set includes **methods** of other files (`OnlyMethodInB()` → `lib_t3_onlymethodinb`, quirk). A same-package candidate is preferred over a same-named func elsewhere (`clean(...)` in `service_test.go` → `service_service_clean`, not `store_helpers_clean`). Dot-imported funcs resolve the same way: `Normalize(name)` → `store_helpers_normalize` INFERRED 0.85.
4. **Package-qualified** `pkg.F()` where `pkg` is the import alias or the **last import-path segment** (not the package clause: `mylib.Helper()` from dir `lib` is dropped) and the import resolves locally → EXTRACTED with `confidence_score: 1.0` (`cmd_app_main_main -calls-> store_store_sum`, `-> store_helpers_format`, `-> service_service_newuserservice`). Takes precedence over a same-named local func (`ml.Dup()` → `lib_a_dup` while bare `Dup()` → `app_main_dup`).
5. **Dropped**: method calls on values whose method lives in another file (`svc.Register()`, `t.Only()` in `b.go`, `s.users.Put`), calls through fields/vars/closures (`s.log(...)`, `cb()`, `run()`), unknown qualified names (`ml.Missing()`), all external package calls (`fmt.Println`, `strings.ToLower`, `sort.Slice`, `uuidlib.New`) — Go never emits `calls` to `go_pkg_*`, composite literals `&T{}` / `T{}` (no constructor edge), `go f()` / `defer f()` count as plain calls.
6. One `calls` edge per (caller, callee) pair; the first call site's line is kept.

### 12.3 Test files

`*_test.go` are ordinary code files: `service_service_test` / `service_service_test_testregister`,
`testing.T` param → `go_type_testing_t` stub. Nothing marks them as tests.

### 12.4 Quirks (do not reproduce unless chasing byte parity)

- **G1** cross-package type re-keying to `<slug(def file)>_<dir>_<name>`; method ids not re-keyed.
- **G2** id collisions merge file and type nodes (file node lost, self-loop `contains`), and func vs method.
- **G3** generic receivers create a phantom node `store_store_t` label `Store[T]` at the first
  method's line, owning `.Put()`/`.Get()` via `method`, with no `contains` from the file and no
  link to the real `Store` type.
- **G4** package imports link to `_test.go` files.
- **G5** method calls resolved by bare name, last definition in the file wins.
- **G6** external package nodes use the id as label (`go_pkg_fmt`), unlike Python (`json`).
- Root-level type ids depend on the name of the directory graphify was run in (`p1_r`), so the
  graph changes if the checkout directory is renamed.

---

## 13. Java

Fixture: `tests/fixtures/java/` (Maven layout; packages `com.shop.model`, `com.shop.service`, test
under `src/test/java`). Goldens: `java.graphify.json`, `java.clustered.graphify.json`. Ids below
abbreviate the common prefix `src_main_java_com_shop_` as `…`.

`fileid` = slug of the path without extension (`src/main/java/com/shop/model/User.java` →
`…model_user`; root-level `A.java` → `a`). Java does **not** use the package clause for ids.

### 13.1 Nodes

| Construct | id | label | extra |
|---|---|---|---|
| file | `fileid` | basename (`User.java`) | — |
| any type declaration — class, abstract class, interface, enum, record, `@interface`, and **nested/inner/static-nested types at any depth** | `fileid_slug(Name)` — flat, the outer type is not part of the id: `…model_user_user`, `…model_user_address` (record nested in `User`), `…model_user_badge`, `…model_model_status` (enum nested in `Model`), `…service_userservice_repository` (2nd top-level type in file) | `Name` | `_callable: true`, `_callable_class: true` |
| method, constructor, interface/abstract method (with or without body), annotation element | `<type id>_slug(name)`: `…model_user_user_getname`; constructor `…model_user_user_user` | `.name()`; constructor `.User()` | `_callable: true`; optional `metadata.unresolved_calls` |
| enum constant | `<enum id>_slug(CONST)`: `…model_model_status_active` | `ACTIVE` | none (no `_callable`) |
| anonymous class `new Runnable() {…}` | `<enclosing method id>_object_slug(Type)_l<line>`: `…service_userservice_userservice_notify_object_runnable_l38` | `Runnable` | `_callable`, `_callable_class`; its methods `…_l38_run` label `.run()` |
| imported FQ type that is referenced (field/param/annotation) but not local | `slug(FQN)`: `org_junit_jupiter_api_test`, `java_lang_annotation_retention`, `org_lib_thing` | the FQN `org.junit.jupiter.api.Test` | `file_type: code`, `source_file: ""`, `source_location: ""`, `_origin: ast`, no `external` |
| annotation used but not imported (`@Override`) | **per file**: `slug(path incl. extension)_slug(Name)`: `…model_model_java_override` | `Override` | same stub shape as above |
| import that stays unresolved | `slug(last segment)` (see Imports) | same as id: `instant`, `util`, `requirenonnull` | `file_type: concept`, `type: external`, `external: true`, `source_file: ""` |

Not emitted: fields, record components, local variables, lambdas, method references, generic
type parameters (`K`, `T` produce no stub, unlike Go/Python), and **no rationale nodes** — Javadoc
(`/** … */`) and `// NOTE:` / `// WHY:` comments are ignored.

**Overloads merge**: overloaded methods and constructors share one id; the node keeps the
**first** declaration's `source_location` (`.register()` L15, `.User()` L10), and edges from all
overloads attach to it (`…userservice_register -references-> …user_address` `parameter_type` at
L25 comes from the 2nd overload).

`metadata.unresolved_calls` (raw: after `source_location`, before `_callable`): a list of
`{"callee", "receiver_type", "lang": "java", "line": "L<n>"}` in source order, one per
`Type.method(...)` call whose receiver is a capitalised identifier that is not a local class:
`Objects.requireNonNull` and `Instant.now` on `…model_model_model_model`, `Collections.sort` on
`…userservice_names`, `Math.max` in probe. Not recorded: `System.out.println` (field receiver),
lowercase receivers, ambiguous local types (`Formatter.shout`, see below).

### 13.2 Edges

All: `weight: 1.0`, `_origin: "ast"`, `source_file` = file of the source construct.

| relation | source → target | when | confidence | context | location |
|---|---|---|---|---|---|
| `contains` | file → top-level type; outer type → nested type; method → anonymous class | always | EXTRACTED | none | decl line |
| `method` | type → method/constructor (incl. enum and anonymous class methods) | always | EXTRACTED | none | method line |
| `case_of` | enum → constant | always | EXTRACTED | none | constant line |
| `inherits` | class → superclass (`User extends Model`) | local superclass only | EXTRACTED | none | class line |
| `implements` | class → interface (`Model implements Entity<String>`, `UserService implements Repository<User>`) | local interface only; `Comparable<Model>` dropped | EXTRACTED | none | class line |
| `references` | type → field type | local type or imported FQ stub | EXTRACTED | `field` | field line |
| `references` | type → generic arg in `implements`/`extends` (`Repository<User>` → `…model_user_user`) or field generic (`List<Address>`) | | EXTRACTED | `generic_arg` | that line |
| `references` | method → param / return type | | EXTRACTED | `parameter_type` / `return_type` | method line |
| `references` | method/type → annotation (`@Override`, `@Audited`, `@Test`, `@Retention`) | every annotation | EXTRACTED | `attribute` | annotated decl line |
| `imports` | file → class / external stub | every import declaration | EXTRACTED | `import` | import line |
| `imports` (2nd pass) | file → local class | see Imports | EXTRACTED, `confidence_score: 1.0` | **none** | import line |
| `calls` | method → method/class | see Calls | EXTRACTED / INFERRED | `call` | call line |

No `inherits`/`implements` to external supertypes, no `uses`, no `indirect_call` (method references
`User::getName`, `out::add` and lambdas produce nothing), no `rationale_for`. Bounds
(`<T extends Entity<String>>`) produce no edge. JDK types (`String`, `List`, `Map`, `Instant`
field) produce no `references` edge and no stub. References dedupe per (source,target): `make(Dup
d)` returning `Dup` keeps only `parameter_type`.

#### Imports

First pass, per import declaration, target id = `slug(last segment)`:

| form | target |
|---|---|
| `import java.time.Instant;` | `instant` (external) |
| `import java.util.*;` | `util` (the package's last segment) |
| `import java.util.function.Function;` | `function` |
| `import static java.util.Objects.requireNonNull;` | `requirenonnull` (member name) |
| `import static org.junit…Assertions.assertEquals;` | `assertequals` |
| `import java.util.Map.Entry;` / `import java.io.*;` | `entry` / `io` |
| imported type used as a type or annotation in the file | `slug(FQN)` stub (`org_junit_jupiter_api_test`, `java_lang_annotation_retention`, `org_lib_thing`) |

Then a global pass rewrites targets: if a local class is referenced anywhere in the repo **as a
type** under that simple name, a stub with id `slug(Name)` is merged into the class node and every
edge pointing at it (including other files' imports) is redirected. Consequences:

- `import com.shop.model.User;` → `…model_user_user` directly (ctx `import`, no score).
- `import com.shop.model.User.Address;` → `…model_user_address`.
- `import com.x.util.Tools;` with `Tools` never used as a type anywhere → `tools` **external**
  node plus a 2nd-pass edge `imports -> com_x_util_tools_tools` (`confidence_score: 1.0`, no
  context). Add `class Other { Tools t; }` anywhere and the external node disappears and the
  first-pass edge targets the class instead (probe `j5`).
- **Quirk J1** — `import com.shop.model.*;` targets `…model_model_model` (class `Model`) because
  the package's last segment `model` equals the stub id of the referenced class `Model`.
- **Quirk J2** — the 2nd pass resolves `import static com.shop.service.Formatter.slug;` to
  **every** class named `Formatter`: `…service_userservice_formatter` and the test's
  `src_test_java_com_shop_service_userservicetest_formatter` (both EXTRACTED 1.0), in addition to
  the external `slug`.
- `import a.Dup;` with `a/Dup.java` and `b/Dup.java` both defining `Dup` → resolved by FQN to
  `a_dup_dup`; types used in that file resolve the same way.

#### Calls

1. **First pass (same file, no `confidence_score`)**: bare calls `name(...)` resolve to a
   same-file method **with a body**, searching every type in the file: `save(user)` →
   `…userservice_save`; `slug(name)` → `…formatter_slug`; inner class `Badge.text()` →
   outer `…user_user_getname`; `new User(name)` inside `User.java` → constructor
   `…user_user_user`; `new Runnable(){…}` → `calls` to the anonymous class node. Bare calls to
   body-less (interface/abstract) methods are dropped (`default String d() { … id(); }`).
2. **Second pass** (`confidence_score` present):
   - `this.m()` → EXTRACTED 1.0 (`…register -calls-> …userservice_notify`; `I.e()` → `I.id()`).
   - `Type.m()` with `Type` a unique local class name → EXTRACTED 1.0, needs no import
     (`User.of(...)` → `…user_user_of`; `Util.helper()` from default package → `b_util_util_helper`).
   - `var.m()` / chained → INFERRED **0.85** when `m` names exactly one method-with-body in the
     repo, regardless of receiver type and even in the same file (`user.getName()` →
     `…user_user_getname`; `user.id()` → `…model_model_id` because `Entity.id()` has no body;
     `service.names()` same file; test `service.register(…)`).
   - `new X()` cross-file → INFERRED 0.85 to the **class** node when `X` is unique and has **no
     explicit constructor**; dropped when `X` declares a constructor (the name then matches both
     class and `.X()` → ambiguous): `new NoCtor()` edge, `new WithCtor()`/`new UserService()`/`new
     User(name, address)` none.
   - Ambiguous names drop silently: `formatter.format(...)` (two `.format()`), `d.run()` (two
     `Dup.run`), `Formatter.shout(...)` (two `Formatter` classes, not even recorded as unresolved).
3. Never emitted: `super(...)`, `this(...)`, `super.m()`, external calls, method references,
   lambda bodies' calls are attributed to the enclosing method (`f.apply(u)` → nothing resolvable).
4. One edge per (caller, callee) pair; first call line kept.

### 13.3 Test files

`src/test/java/...` files are ordinary code: `…userservicetest_userservicetest_registersuser`,
annotation stubs `org_junit_jupiter_api_test` / `…_beforeeach`. The test's nested `Formatter` is a
normal duplicate that poisons resolution in main code (J2, `formatter.format` dropped).

### 13.4 Quirks (do not reproduce unless chasing byte parity)

- **J1** wildcard import resolves to a class whose lowercase name equals the package's last segment.
- **J2** static-import 2nd pass links to all same-named classes across packages.
- **J3** whether an import edge targets the class or an external stub depends on whether that
  class is used as a type anywhere else in the repo (non-local effect).
- **J4** `@Override` (and any un-imported annotation) gets one stub node per file.
- **J5** overloads collapse into one node at the first overload's line.
- **J6** nested types are not namespaced by their outer type (verified, probe `j7`): `F.Inner`
  and `G.Inner` in `F.java` merge into one node `f_inner` (first declaration's line, L2) with both
  `f_f -contains->` and `f_g -contains->` edges and both methods `.a()` and `.b()`.

---

## 14. JSON, TOML, shell and markdown in a code-only build

Fixture `mixed/`; goldens `mixed.graphify.json` and `mixed.clustered.graphify.json` (31 nodes, 31 edges each).

### 14.1 JSON (`.json`)

#### Which JSON files produce nodes

A `.json` file produces nodes only if (a) its basename is in a manifest allowlist, or (b) its
top-level object has a "manifest" key. Otherwise it produces **nothing — not even a file node**
(and does not participate in file-label disambiguation).

- (a) observed allowlist: `package.json tsconfig.json jsconfig.json composer.json app.json
  manifest.json deno.json .eslintrc.json .babelrc.json .prettierrc.json babel.config.json
  angular.json biome.json bower.json renovate.json vercel.json`.
  Observed NOT in it: `turbo nx lerna project firebase schema openapi swagger settings launch
  tsconfig.app tsconfig.node tsconfig.base cargo pyright jest eslint .stylelintrc vite.config`.
- (b) trigger keys observed: `dependencies devDependencies peerDependencies compilerOptions
  extends` (and presumably `optionalDependencies`). Non-triggers: `name version scripts main type
  private workspaces include files exports module bin description author license keywords
  repository engines`.
- Top-level array (`[1,2]`) or unparsable JSON → no nodes. JSONC comments and trailing commas in
  tsconfig.json are tolerated.

#### Shape

```
package.json                     {"name": ..., "scripts": {"build": ...}, "dependencies": {"react": ...}}
N package                 label "package.json"   (file node, L1)
N package_name            label "name"           L2   contains from package
N package_scripts         label "scripts"        L4
N package_scripts_build   label "build"          L4   contains from package_scripts
N package_dependencies_react  label "react"      L5   contains from package_dependencies
N ref_react               label "react"  file_type "concept"  source_file "package.json" L5
E package -imports-> ref_react   EXTRACTED ctx "import"  L5
```

- Every object key becomes a node (file_type `code`, `_origin` `ast`, location = line of the key),
  recursively through nested objects; array elements are never descended.
- Key node id: depth 1 → `<fileid>_<slug(key)>`; depth ≥ 2 → `<fileid>_<slug(parentkey)>_<slug(key)>`
  — **only the immediate parent key**, not the full path (quirk): `deep.a.b.c.d` gives
  `package_deep`, `package_deep_a`, `package_a_b`, `package_b_c`, `package_c_d`.
- Keys whose slug is empty are dropped (`"@/*"` under `paths`).
- Edges: `contains` parent→child (no context, EXTRACTED).
- Dependency keys inside `dependencies|devDependencies|peerDependencies|optionalDependencies`
  whose value is a **string** additionally create a concept node `ref_<slug(name)>` and an edge
  `file -imports-> ref_*` (context `import`). `"@types/node"` → `ref_types_node`,
  `"lodash.merge"` → `ref_lodash_merge`. Non-string values (`{"a": 1}`) → no ref.
  `bundledDependencies` (array) → no refs.
- `"extends": "<string>"` → concept node `ref_<slug(value)>` (`./base.json` → `ref_base_json`,
  label `./base.json`) and edge `file -extends-> ref` with context `import`.
- Values are never stored (no package name/version node for package.json, unlike TOML).
- Ref node shape (raw): `{"id","label","file_type":"concept","source_file","source_location","_origin":"ast"}`
  — no `external` flag.
- Ref nodes are per-file: when two JSON files reference `react`, both ids collide and are
  rewritten to `package_json_ref_react`, `sub_package_json_ref_react`,
  `x_package_json_ref_react` (collision rule, §4.5).
- Unicode keys keep letters (`"Ünïcode"` → `package_ünïcode`).


### 14.2 TOML (`Cargo.toml`, `pyproject.toml`)

- No file node, no key nodes. The file always appears in the "yielded no symbols" warning even when
  it produces a package node (quirk).
- One package node when a name is found: `[package] name` (Cargo), `[project] name` or
  `[tool.poetry] name` (pyproject):
  `{"id":"pkg_<slug(name)>","label":<name>,"file_type":"code","type":"package","ecosystem":"cargo"|"python","source_file","source_location":"L1","version":<str>,"_origin":"ast"}`
  (key order as shown). `version` omitted when not a literal string (`version.workspace = true`).
  Workspace-root Cargo.toml without `[package]` → nothing.
- Edges: `depends_on` (context `dependency`, EXTRACTED, raw carries `confidence_score: 1.0`,
  location `L1`) between package nodes only when the dependency name resolves to another package
  node in the scan (name slug match: `crate-b` dep → `pkg_crate_b` whose name is `crate_b`).
  External crates/pypi deps (`serde`, `requests`) produce nothing.
- Collision: two packages named `mixed-fixture` (Cargo.toml + pyproject.toml) →
  `cargo_toml_pkg_mixed_fixture`, `pyproject_toml_pkg_mixed_fixture`; `x/Cargo.toml` and
  `y/Cargo.toml` both `same` → `x_cargo_toml_pkg_same`, `y_cargo_toml_pkg_same`.
- `--cargo` (not used by our goldens) additionally adds nodes `crate:<name>` with only
  `id,label,source_file,source_location` (no file_type/_origin) and logs "Cargo: N nodes, 0 edges".
- Corpus: 20 `.toml` package nodes (15 with version), 19 `depends_on` edges, all in monet.


### 14.3 Shell (`.sh`, `.bash`, sh/bash/zsh shebang)

```
N scripts_build             "build.sh"          metadata {"language":"bash","kind":"file"}
N scripts_build_sh__entry   "build.sh script"   metadata {"language":"bash","kind":"bash_entrypoint"}  L1
N scripts_build_clean       "clean()"           metadata {"language":"bash","kind":"bash_function"}    L6
E scripts_build -contains-> scripts_build_sh__entry   L1
E scripts_build -defines->  scripts_build_clean       L6     (functions use `defines`, not `contains`)
E scripts_build -imports_from-> scripts_common   ctx import  L4    (from `. ./common.sh`)
E scripts_build_sh__entry -calls-> scripts_build_build  ctx call  L21  (top-level call)
E scripts_build_build -calls-> scripts_build_clean       ctx call  L12
E scripts_build_clean -calls-> scripts_common_log        ctx call  L7  (cross-file via sourced file; raw has confidence_score 1.0)
```

- Every shell file gets a file node (with `metadata.kind = "file"`) and an entry node
  `<slug(path incl. extension)>__entry` (double underscore kept), label `<basename> script`, even if
  the script has no top-level calls (`plain.sh`).
- Shell nodes have `metadata` and **no** `_callable`.
- Function ids are `<fileid>_<name>`; nested functions are NOT parent-prefixed
  (`outer(){ inner(){..} }` → `main_inner`) and the edge is `outer -defines-> inner`.
- `source X` / `. X` → `imports_from` file edge only when X is a literal path resolving relative to
  the sourcing file's directory (`./lib.sh` from the same dir). `"$(dirname "$0")/common.sh"` and
  `./scripts/common.sh` (repo-root relative) do not resolve → no edge.
- Calls resolve to functions in the same file or in sourced files (EXTRACTED). A same-named
  function in a non-sourced file is not chosen (`lib2.bash:shared` ignored). External commands
  (`ls`, `echo`, `rm`) produce nothing. Repeated calls to the same target from the same caller
  yield one edge at the first call site.
- Corpus: 65 `.sh` nodes, kinds `file`/`bash_entrypoint`/`bash_function` plus 3 `kind:"code"` (quirk,
  unexplained); relations `defines` 27, `contains` 19, `calls` 14.


### 14.4 Markdown in a code-only build

`README.md` is skipped entirely in a code-only build: it is a doc. Markdown nodes appear only
through `graphify update`, which is how every corpus graph was built. Their shape, briefly:

- **Page node:** `{"id":"crates_core_readme","label":"core/README.md","file_type":"document","node_kind":"page",…,"source_location":"L1"}`.
  An optional `frontmatter` object holds parsed YAML front matter.
- **Heading node:** id `<fileid>_<slug(heading text)>`, label = the heading text verbatim
  (including link markup), `node_kind: "heading"`, located at the heading line.
- **Edges:**
  - `page -contains-> top heading` and `heading -contains-> subheading`, nested by level.
  - A backticked identifier produces `references` to a code node: either EXTRACTED with no context,
    or INFERRED 0.95 with `context` = the identifier text (`readme_sec -references-> a_f`,
    context `f`).
- Markdown nodes take part in file-label disambiguation (§5.1) and in id collisions (§4.5).

### 14.5 Discovery

#### Classification in `--code-only`

Every file falls into one of: code (extracted), doc/paper/image (counted then skipped with
`--code-only: skipping N non-code file(s)`), or "not classified" (skipped with
`N file(s) not classified (no supported extension or shebang)`).

Code (by extension, observed): `.py .ts .tsx .mts .cts .js .jsx .mjs .cjs .go .java .rs .json
.toml* .sh .bash .rb .rake .c .h .cpp .cc .cxx .hpp .cu .cs .kt .kts .swift .php .scala .groovy
.gradle .dart .lua .zig .ex .exs .erl .ml .jl .m .mm .v .sol .f90 .vb .vue .svelte .ps1 .r .sql .tf`,
and also `.astro .psm1 .psd1 .cls .vbproj` (§38–§42).
`.d.ts` and `.min.js` are code (not filtered).

- `.r`, `.sql`, `.tf` count as code but contribute nothing (stderr: optional grammar missing).
- Some code files "yield no symbols" (warning lists them): e.g. `fun kt() {}` in `.kt` (the
  function's id `<file>_kt` is the file's internal id, so it merges into the file node, §21.1),
  `fn = () => 1` in `.jsx`, any `.vue`/`.svelte`/`.lua`/`.zig` stub, root workspace `Cargo.toml`.
  Such files still get a **file node** unless noted (JSON/TOML below are exceptions).
- `*.toml`: only files named `Cargo.toml` and `pyproject.toml` are code; `config.toml` is
  "not classified".

Doc (skipped in code-only): `.md .mdx .rst .txt .html .yaml .yml`, and `CMakeLists.txt` (as .txt).
Paper: `.pdf`. Image: `.png .svg`.

Not classified: `.css .scss .less .graphql .proto .xml .ini .cfg .conf .csv .jsonc .json5 .geojson
.pyi .ipynb (even a valid notebook) .hh .hs .clj .nim .fish .zsh .bat .cmd .pl .pm .fs .elm .ino
.cmake .lock .tsbuildinfo .markdown .toml.example`, and extensionless `Dockerfile Makefile Gemfile
Rakefile`.

Extensionless files with a shebang are code: `#!/usr/bin/env python3`, `#!/usr/bin/python`,
`#!/bin/sh`, `#!/bin/bash`/`env bash`, `#!/bin/zsh` (shell extractor; note `.zsh` *extension* is
not classified — quirk), `#!/usr/bin/env node`, `#!/usr/bin/env ruby`, `#!/usr/bin/env perl`
(counted as code, zero nodes). `#!/usr/bin/env -S deno run` is not classified.
Example in corpus: ripgrep `benchsuite/benchsuite` (python shebang) → file node id
`benchsuite_benchsuite`, label `benchsuite`.

#### Skipped directories (default, any depth, case-sensitive name match)

Skipped: `.git node_modules target dist build __pycache__ .venv venv site-packages .tox .nox
.mypy_cache .pytest_cache .ruff_cache .cache .idea .vscode .next .nuxt .svelte-kit .angular
.turbo .parcel-cache .serverless .terraform .claude __snapshots__ storybook-static .eggs
*.egg-info` and the output dir `graphify-out`. Nested occurrences (`src/build/`,
`src/node_modules/`, `a/dist/`) are skipped too.

**Not** skipped (quirk-worthy): hidden dirs in general (`.github .gradle .hidden .yarn .svn .hg
.direnv .expo .vercel .nx .output .docusaurus .dart_tool .ipynb_checkpoints .hypothesis
.pnpm-store .sass-cache .stack-work .devenv .pixi`), `vendor third_party external bower_components
jspm_packages Pods bin obj out tmp gen generated coverage htmlcov _build deps elm-stuff zig-cache
zig-out cmake-build-debug DerivedData env conda-env log logs docs examples fixtures test tests
migrations lib src pkg dijkstral-out`, and case variants `Build Dist NODE_MODULES`.
Hidden *files* (`.hidden.py`) are extracted. Generated-looking files (`foo_pb2.py`,
`a.generated.ts`) are extracted.

#### Ignore files

- `.gitignore` is honoured **even when the directory is not a git repo**, including nested
  `.gitignore` files scoped to their directory (`sub/.gitignore` with `deep/` drops `sub/deep/`).
- `.graphifyignore` is honoured always, including under `--no-gitignore`.
- `--no-gitignore` disables `.gitignore` only.

#### Lockfiles and sensitive files

- `package-lock.json` is silently dropped (not counted as code, not reported). `yarn.lock`,
  `Cargo.lock`, `composer.lock`, `bun.lock` are `.lock` → not classified. `pnpm-lock.yaml` is
  yaml (doc).
- Sensitive filter (reported as `N file(s) skipped as potentially sensitive`): `.env`,
  `.env.local`, `*.pem`. Not flagged: `credentials.py`, `secrets.py`, `id_rsa.py` (extracted).

#### `built_at_commit`

- Raw `--no-cluster` output never has it (keys: `nodes, edges, hyperedges, input_tokens,
  output_tokens`).
- Clustered output: key present (last top-level key) **only** when the scan root is inside a git
  work tree with at least one commit; value = `git rev-parse HEAD` of the enclosing repo (also when
  the scan root is a subdirectory of the repo). Not a repo, or a repo with no commits → key
  **absent** (not null).
- `source_file` paths are always relative to the scan root, not the repo root.

---

## 15. Quirks summary and open questions

### 15.1 Quirks (do not reproduce in dijkstral-native output; expected parity differences)

| # | Quirk | Where |
|---|---|---|
| Q1 | Raw output deduplicates `(source, target, relation)` keeping the first edge, while the build keeps the last. The raw and clustered outputs disagree on context and location for duplicates. | §2.4, §8.2 |
| Q2 | In an import cycle through a re-exporting `__init__.py`, the collapse keeps the edge with the lexicographically larger source. | §8.2 |
| Q3 | Warm rebuilds lose every `dynamic_import` edge and phantom node (excalidraw: −12 nodes, −13 edges). | §8.4 |
| Q4 | Cross-file id collisions rename **all** emitters, including a file node that collides with an unresolved type stub from another file (`base.py` → `base_py_base`), and the rename sticks after the stub is resolved. | §4.5 |
| Q5 | Same-file id collisions merge distinct definitions: Python redefinitions, TS `function foo` vs `class Foo`, Rust flattened inline modules (`inner::helper` vs `deeper::helper`) and variant/method slugs, Go file vs type (file node lost, `contains` self-loop), Java overloads and same-named nested types. | §4.5, §10.1, §11, §12, §13 |
| Q6 | The clustered build remaps external stubs to a same-stem file (`use std::fmt` → `src/util/fmt.rs`, `path` → another crate's `path.rs`) and drops cross-language import edges. | §8.1, §11 |
| Q7 | Rust import edges resolve by name coincidence through type-stub ids, not by module path. | §11 |
| Q8 | TS/JS `anything.m()` binds to the last same-file method named `m`, which creates self-loops (`console.log` inside `.log()`). Go has the same last-definition rule for same-file method names. | §10.4, §12 |
| Q9 | TS barrel imports leave a dangling stub (`src_models_index_user`) beside the real edge. Dynamic-import phantoms carry `confidence` and no `source_location`, and can coexist with a `ref_*` stub for the same package. | §10.3 |
| Q10 | Go: cross-package type re-keying (`store_store_go_store_model`); root-level type ids depend on the checkout directory name; generic receivers create phantom `Store[T]` nodes; package imports link to `_test.go` files; external labels equal ids. | §12 |
| Q11 | Java: a wildcard import resolves to a class named like the package; static imports link to every same-named class; whether an import targets a class depends on its use elsewhere in the repo; `@Override` gets one stub per file. | §13 |
| Q12 | Python: calls to external modules are emitted only from top-level functions and only through `import x` / `import x as y` aliases. Calls qualified by a local module are dropped. | §9.4 |
| Q13 | Collapse artefacts: `contains`/`method` links carrying `context: "call"` (935 in the corpus); `uses` links carrying `parameter_type`; an EXTRACTED link with `confidence_score: 0.85`. | §8.2 |
| Q14 | Generic type parameters become global stub nodes (`t`, `u`, `f`) shared across unrelated files. | §9, §11, §12 |
| Q15 | Rust in-file and cross-file call resolution skips Python builtin names (`len`, `map`, `iter`, `min`, …). Rust type stubs can resolve to classes in extensionless Python scripts. | §11 |
| Q16 | JSON key ids use only the immediate parent key (`package_c_d` for `deep.a.b.c.d`). TOML package files are reported as "yielded no symbols" yet emit a node. | §14 |
| Q17 | `dynamic_import` edges have no `weight` and no `source_location`. | §7.1 |
| Q18 | Python: `from . import x` inside `pkg/__init__.py` produces an `imports_from` self-loop, which the build then drops. | §8.3 |
| Q19 | Kotlin/Scala: a definition named like the file's extension (`fun kt()` in `a.kt`, `def scala()` in `Main.scala`) merges into the file node, which gains `_callable` and a `contains` self-loop. | §21.1, §22.1 |
| Q20 | Kotlin: declarations found inside `init` blocks, secondary constructors, misparsed headers and anonymous objects outside functions become top-level functions of the file; Scala does the same for `def`s in expression blocks of a class body. | §21.1, §22.1 |
| Q21 | Kotlin: references to a qualified type use its **first** segment (`Interceptor.Chain` → `Interceptor`, `java.io.File` → `java`). | §21.2 |
| Q22 | Scala: imports link to the first path segment (`import shop.model.User` → `shop`). | §22.2 |

### 15.2 Open questions

1. **Mechanism of Q2.** It is consistent across 5 of 5 cases, but no simple sequential or sorted
   rule explains it together with the 23 equal-rank reversed pairs that keep the smaller source.
2. **Rank table completeness.** The ranks of `embeds`, `extends`, `depends_on`, `defines` and
   `dynamic_import`-versus-`calls` are not pinned by any observed collision.
   - `case_of` < `method` rests on one pattern (Rust and TS `case_of + method` → `method`).
   - `contains` < `imports` rests on one Java case.
   - The special handling of `contains`/`method` (whole replacement as winner; only a `calls`
     context absorbed as survivor) fits all data, but it is a description, not a known mechanism.
3. **Scope of the stub-to-file remap (§8.1).**
   - Rust externals are remapped (21 of 21 in ripgrep).
   - Python externals with a same-language, same-stem file are not (fastapi `http`, `types`,
     `gzip`).
   - Unknown: whether the trigger is the importing language, the stub's origin (`use` versus
     `import`), or the stem file's language.
4. **Community assignment.** Resolved in §17: reproduced exactly on every fixture and corpus graph.
5. **`AMBIGUOUS`** never appears in code-only output. It may exist only in semantic or LLM mode.
6. **Rationale truncation** when the first 80 characters contain no space. Python docstring
   threshold behaviour exactly at 20/21 characters after whitespace collapse with non-ASCII text.
7. **Rust denylists** (type references, std traits, cross-file call names) are known only for the
   names probed (§11).
8. **TS/JS.**
   - Is the `@/` → `src/` mapping without a tsconfig a fixed heuristic?
   - Do INFERRED member calls walk inheritance?
   - `.vue` stem collisions follow §4.5 (`c.vue` + `c.ts` → `c_vue_c`, `c_ts_c`); the merged
     `c_vue_c` node with a self-loop was not reproduced by any probe (§38).
9. **Python.**
   - Does the unique-global-name fallback (`lonely_b()` → INFERRED) also consider methods
     (labels `.name()`), as Rust's does?
   - Are imports inside functions ever emitted as file-level edges? They are masked by
     deduplication in the probes.
10. **Go and Java gaps.**
    - Go: `package x_test` external test packages, cgo, `vendor/`.
    - Java: `package-info.java`; `var.m()` when the only definition is a body-less interface
      method; enum constants with bodies.
11. **JSON allowlist** and manifest trigger keys are only partly enumerated (§14.1).
12. **`update` versus `extract`.** `update` adds markdown nodes (§14.4) and `community_name`. Its
    warm path was reported to lose nodes on the graphify and excalidraw repos. For extract, Q3
    explains it; for `update`, this was not re-measured.

---

## 16. Addenda from implementation (2026-10-12)

The language implementations probed Graphify further while reaching golden parity. Where this
section disagrees with an earlier one, this section wins; the module docs of each
`crates/core/src/{extract,resolve}/<lang>.rs` carry the full detail.

- **Rust (§11).**
  - `recv.m()` binds to the **last** same-file method named `m`, not a unique one. Inside an `impl`,
    `self.m()` prefers that impl's own method.
  - Cross-file bare calls are resolved by **path proximity** (longest shared directory prefix,
    then same directory), not by global uniqueness; a tie resolves nothing.
  - `self.m()` with no `m` in the file resolves EXTRACTED 1.0 to the same type's method in another
    file when the type is declared exactly once (this is what `_rust_impl_key` serves).
  - A struct/enum/trait wins stub resolution over other same-label nodes. A type the file declares
    overrides the type denylist (`struct Error`). References, `implements` and `inherits` never
    point at their own source. Tuple-struct constructors bind to the last node with that label and
    have their own denylist. The three Rust denylists are now complete in the module.
- **Python (§9).** Same-file clashes between plain functions or methods get `_sha1(name)[:6]`;
  `indirect_call` is skipped when the function was already called; module-level collections and
  assignments use the file node as the source; `uses` also counts class-body and function-level
  imports. Excluded type and decorator names are listed in the module. `uses` is never emitted
  for an imported name whose original name starts with `_` (`from m import _Priv`, also
  `_Priv as Q`; `Pub as _P` still counts). An absolute `from models import X` with no root-level
  `models` resolves to a same-stem file elsewhere in the repository (a sibling in the probe,
  `lib/helpers.py` for `from helpers import …` in `other/x.py`); `uses` follows it only when the
  stem is unique.
- **TS/JS (§10).** Module resolution also follows the nearest `tsconfig`/`jsconfig` `extends`,
  workspace packages by `package.json` name and `exports`/`main`; `@/` resolves relative to the
  importing directory. Class-expression methods, object-literal methods passed to module-level
  calls, and declarations inside top-level `describe`/`it` callbacks become module-level
  functions (needed for frappe-style codebases).
- **Go (§12).** Type-parameter and unresolved stubs are per file, not global. Same-file names that
  differ only in case get `_sha1(name)[:6]`. Stubs bind globally by label (unique type, else unique
  function). In-file calls bind against all of the file's nodes, last wins, skipping a builtin
  denylist; bare cross-file calls bind by path proximity. Generic receivers create a node for any
  receiver type not declared earlier in the file.
- **Java (§13).** Scoped stubs use the last identifier as their label, except annotations, which
  keep the full name. Graphify counts external import nodes as emitters in the §4.5 collision
  rename; dijkstral exempts externals in every language (a few Java stub ids differ).
- **JSON (§14.1).** The allowlist also has `nest-cli.json` and `now.json` (case-insensitive);
  `$schema`, `bundledDependencies` and `bundleDependencies` also trigger manifests; `$schema` plus
  `definitions`/`$defs`/`$id` marks a JSON Schema document and yields nothing; at most 500 key
  nodes per file in pre-order; a `dependencies` object at any depth creates refs; array `extends`
  edges come from the `extends` key node. Cross-file `ref_*` nodes with the same label (case
  folded) merge into the shortest id when the label's Shannon entropy is at least 2.5 bits per
  character.
- **TOML (§14.2).** Cargo dependencies also come from `[dependencies.x]` and
  `[target.*.dependencies]`; in pyproject, `[project]` wins over `[tool.poetry]` field by field;
  dependency names match package names in any ecosystem; self-dependencies are skipped.
- **Shell (§14.3).** Top-level `export`/`declare`/`readonly`/`local`/`typeset` assignments become
  `kind:"code"` nodes (the "unexplained" quirk). Sourcing resolves `./`/`../` paths EXTRACTED,
  existing bare paths INFERRED, missing bare paths to an external, script-directory variables
  INFERRED; running another script creates `calls` with context `script_invocation`.
- **`norm_label` (§5.4).** "Combining marks" are characters with a non-zero canonical combining
  class (`unicodedata.combining`), not the general category M: Devanagari spacing vowel signs
  (`ि`, class 0) stay, the virama (`्`, class 9) goes. Found on a graphify-corpus rationale label.

---

## 17. Clustering, analysis and `GRAPH_REPORT.md` (2026-10-13)

Derived black-box like the rest of this document: `graphify extract`, `update` and
`cluster-only --no-label --no-viz` were run on scratch copies of the fixtures and the corpus, on
hand-written `graph.json` files fed to `cluster-only`, and through `graphify god-nodes`. The Leiden
engine was read from its own source (Microsoft's MIT-licensed `graspologic-native` 1.3.1, crate
`network_partitions`, used by dijkstral at the same tag); Graphify's source was not.

**Where each output comes from.** `extract` writes `graph.json` (with `community`) and
`.graphify_analysis.json`, but **no report**. `update` (no model) writes `GRAPH_REPORT.md` with
hub-named communities and corpus statistics; `cluster-only --no-label` re-clusters an existing
`graph.json` and writes the report with `Community N` names. Dijkstral's `extract` writes all three:
the analysis sidecar as `extract` does and the report as `update` would for the same graph.
`dijkstral cluster-only` mirrors `graphify cluster-only --no-label`.

**Verification.** `dijkstral cluster-only` on Graphify's own `graph.json` reproduces Graphify's
`cluster-only` output: `graph.json` byte-identical for all 7 fixtures and all 7 corpus repos
(Kubernetes included: 256,424 nodes, 9,417 communities); the analysis sidecar byte-identical for 6
of 7 fixtures and for monet, ripgrep, excalidraw and Kubernetes (sampled betweenness included);
the report byte-identical for 6 of 7 fixtures and monet. The residual differences are in §17.7.

### 17.1 Hypotheses (as briefed) and outcome

| Hypothesis | Outcome |
|---|---|
| Leiden via graspologic-native 1.3.1: resolution 1.0, randomness 0.001, 1 iteration, modularity, seed 42, 1 trial | **Held** |
| Input graph rebuilt in sorted order | **Held, refined**: the edge list is `sorted((min(u,v), max(u,v), w))` over all links, self-loops included; nodes are numbered by first appearance in that list |
| AMBIGUOUS weights × 0.5 | **Not observable** in code-only graphs (no AMBIGUOUS links); implemented as stated. Link `weight` is the Leiden weight (1.0 when absent) — verified: unweighted runs diverge |
| Isolates become singletons | **Held** |
| Communities > 25% of nodes and ≥ 10 nodes are split again | **Refined**: > 25% **and > 10** (a 10-node community at 32% of the `mixed` fixture is not split; 11 is) |
| Communities ≥ 50 nodes with cohesion < 0.05 re-split | **Held** (one pass, after the size split; boundaries not pinned by any corpus) |
| Final ids by size descending, ties by sorted members | **Held** (Python list order of the sorted member ids) |
| Label = highest-degree member, ties smaller id, trailing `()` stripped | **Held** (100% of 2,800 corpus communities) |
| God nodes: top 10 by degree excluding files, stubs/concepts and noise labels | **Refined**: §17.4 |
| Betweenness sampled with k=100 | **Refined**: exact up to 1,000 nodes; above, `k=100` sources from `random.Random(42).sample(list(G.nodes()), 100)` |
| Edge betweenness skipped above 5k nodes | **Not observable**: no output uses edge betweenness |

### 17.2 Community detection

1. Links → `(lo, hi, weight)` with `lo < hi` by id (Python string order), `weight` from the link
   (absent → 1.0; AMBIGUOUS × 0.5). Sort the list. Self-loops stay.
2. graspologic-native `leiden(edges, resolution=1.0, randomness=0.001, iterations=1,
   use_modularity=True, seed=42, trials=1)`: nodes numbered by first appearance in the list; a
   repeated pair keeps its first weight; a self-loop only adds to the total self-link weight.
3. Nodes without links: one singleton community each.
4. Each community with `len > 0.25 · N` and `len > 10` is replaced by a Leiden run (same
   parameters, same edge-list recipe) on its induced subgraph, plus singletons for its isolates.
5. Then, once, each community with `len ≥ 50` and cohesion `< 0.05` is re-partitioned the same way.
6. Number communities by `(-len, sorted member ids)`.

Cohesion (reported and in step 5) is `internal links without self-loops / (n·(n−1)/2)`, `1.0`
for fewer than two members (`go` and `typescript` fixtures pin the self-loop exclusion).

### 17.3 `.graphify_analysis.json`

`json.dump(indent=2)`, no trailing newline, floats in Python `repr` (`0.059940059940059943`,
`1e-05`). `extract` escapes non-ASCII text (`ensure_ascii=True`: `"Caf\u00e9"`); `cluster-only`
writes it as UTF-8 (a Kubernetes id with `ω`). Keys in order:

```json
{
  "communities": {"0": ["lib_store_store", "..."], "...": []},
  "cohesion": {"0": 0.2857142857142857},
  "gods": [{"id": "lib_store_store", "label": "Store", "degree": 9}],
  "surprises": [{"source": "Row()", "target": "formatLabel()",
                 "source_files": ["components/App.jsx", "lib/format.mjs"],
                 "confidence": "EXTRACTED", "relation": "calls",
                 "why": "connects across different repos/directories; bridges separate communities"}],
  "tokens": {"input": 0, "output": 0}
}
```

Members are sorted by id. `cluster-only` writes `questions` (`[{type, question, why}]`, §17.5)
instead of `tokens`.

### 17.4 Rankings

All rankings run over the NetworkX graph in **build order** (nodes as extracted; each node's
neighbours in the order their links were added), so ties keep that order. Under `cluster-only`
the order is the loaded `graph.json` order, which is why `extract` and `cluster-only` can name
different tenth god nodes for the same graph. Degree counts a self-loop twice.

- **Symbol**: `source_file` has a suffix (`PurePath.suffix`; `benchsuite/benchsuite` does not) and
  the node is not a file node (label is a `/`-suffix of `source_file`). Stubs and externals
  (empty `source_file`) are not symbols; concepts with a `source_file` (JSON `ref_*`) are.
- **Hub candidate** = symbol, label not a method (`.x()`), label not in the noise list:
  `Any AsyncMock Bool Callable ClassVar Color Combine Comparable Counter Data Date Dict Double Enum
  Equatable Error False Final Float Font Foundation Hashable Identifiable Int List Literal MagicMock
  Mock Optional OrderedDict Path Protocol Sendable Set String True Tuple Type URL UUID Union View abc
  bool bytearray bytes complex datetime defaultdict float int io json object os patch re sentinel
  str sys typing` (case-sensitive; found by feeding ~593k candidate labels to `graphify god-nodes`).
- **God nodes**: hub candidates that are also not generic JSON keys (in a `.json` file,
  case-insensitively: `bundleddependencies bundledependencies data dependencies description
  devdependencies end id items key name optionaldependencies peerdependencies properties start
  title type value version`); top 10 by degree, stable.
- **Surprising connections**: links in `G.edges()` order between two hub candidates (neither a
  concept nor rationale) in different files. Score = confidence (EXTRACTED 1, INFERRED 2,
  AMBIGUOUS 3) + 2 if the top-level directories differ (`main.py` → `""`) + 1 if the communities
  differ + 1 if one end has degree ≤ 2 and the other ≥ 5. Stable sort by score, top 5. `why` joins,
  in this order: `inferred connection - not explicitly stated in source`, `connects across
  different repos/directories`, `bridges separate communities`, ``peripheral node `p` unexpectedly
  reaches hub `h` ``; with none of them, `cross-file semantic connection`.

### 17.5 `GRAPH_REPORT.md`

No trailing newline. Sections in order, each preceded by a blank line:

```text
# Graph Report - <dir name>  (<YYYY-MM-DD>)
## Corpus Check            - N files · ~W,WWW words | Verdict: corpus is large enough that graph structure adds value.
                           (cluster-only: "- cluster-only mode — file stats not available")
## Summary                 - N nodes · E edges · C communities (S shown, T thin omitted)
                           - Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS[ · INFERRED: n edges (avg confidence: 0.91)]
                           - Token cost: 0 input · 0 output
## Graph Freshness         only with built_at_commit: commit[:8] and two fixed hint lines
## Community Hubs (Navigation)   "- <label>" for every community with at least one listed member
## God Nodes (most connected - your core abstractions)   "1. `User` - 17 edges"
## Surprising Connections (you probably didn't know these)
                           "- `a` --rel--> `b`  [CONF]" + "  src/a.py → src/b.py";
                           none: "- None detected - all connections are within the same source files."
## Import Cycles           "- k-file cycle: `a -> b -> a`" (≤ 20) or "- None detected."
## Communities (C total, T thin omitted)
                           per non-thin community: ### Community i - "label" / Cohesion: 0.24 / Nodes (k): a, b, … (+n more)
## Knowledge Gaps          - **n isolated node(s):** `a`, … (+n more) + explanation line; - **T thin communities (<3 nodes) omitted from report** — …
## Suggested Questions     _Questions this graph is uniquely positioned to answer:_ then "- **q**" / "  _why_"
```

Rules:

- **Words**: `len(text.split())` per file, thousands separators. Files/words count every file
  `update` indexes (code and markdown; dijkstral counts the code files it indexes). An
  `Unclassified: N file(s) not represented in the graph (top: …)` line appears when the walk saw
  unsupported files; dijkstral does not write it.
- **Percentages**: `round(100·count/links)`; the INFERRED clause only when there is one, with the
  mean INFERRED `confidence_score` to two decimals.
- **Listed members** (Community sections, hubs, thin): symbols, not rationale, not methods, not
  functions (`…()`) with degree ≤ 1; in id order, first 8 shown. *Thin* = fewer than 3 listed.
- **Isolated nodes**: symbols, not rationale, not functions, degree ≤ 1, unless their one link is a
  `contains`; build order; the explanation adds `or undocumented components` when
  the graph has `document` nodes; the total counts every node with degree ≤ 1.
- **Import cycles**: a file digraph from `imports_from` links and module-level `re_exports` (target
  is a file node), skipping `type_only` and `deferred` imports (erpnext's `TYPE_CHECKING` cycle,
  excalidraw's dynamic imports); simple cycles of at most 5 files, shortest first, each rotated to
  start at its smallest path, at most 20.
- **Suggested questions**, interleaved one per kind per round (bridge, verify, isolated, low
  cohesion), at most 7:
  - *bridge_node*: the 5 hub candidates with the highest betweenness; those touching other
    communities, at most 3. ``Why does `X` connect `<own>` to `<a>`, `<b>`?`` — other communities
    in the iteration order of a CPython `set` of their ids. Why: `High betweenness centrality
    (0.304) - this node is a cross-community bridge.`
  - *verify_inferred*: god nodes among the first 5 with ≥ 2 INFERRED links. ``Are the n inferred
    relationships involving `X` (e.g. with `a` and `b`) actually correct?`` (first two INFERRED
    neighbours by id).
  - *isolated_nodes*: ``What connects `a`, `b`, `c` to the rest of the system?`` / `n
    weakly-connected nodes found - possible documentation gaps or missing edges.`
  - *low_cohesion*: communities with cohesion < 0.15 (a connected community needs 14+ members), in id
    order: ``Should `label` be split into smaller, more focused modules?`` / `Cohesion score
    <repr> - nodes in this community are weakly interconnected.`
  - None at all: `_Not enough signal to generate questions. …_`.

### 17.6 Goldens

`tests/fixtures/<lang>.graphify_analysis.json` (from `extract`), `<lang>.graphify_report.md`
(from `update`; `mixed` from a copy without `README.md`, which `update` would index) and
`<lang>.cluster_only.graphify_{analysis.json,report.md}` (from `cluster-only --no-label` on
`<lang>.clustered.graphify.json`). Reports are compared after replacing the title line (directory
name and date) and `graphify update`/`graphify query` in hints with `dijkstral …`; analysis files
need no normalisation.

### 17.7 Remaining differences

- **Extraction**: wherever dijkstral's node or link set differs from Graphify's (the `ALLOWED`
  quirks, stage-1 parity), Leiden sees a different graph and community ids move globally
  (NMI 0.83–0.95 on the corpus); god-node and surprise ties follow build order, which differs where
  the cross-file pass emits links in another order (`go` fixture: `main() → Sum()` vs `Format()`).
- **Surprise ties**: Graphify orders two equal-score surprises differently from `G.edges()` in the
  `java` fixture and in fastapi. Not explained.
- **verify_inferred**: on fastapi (`HTTPException`) and erpnext (`make_stock_entry()`,
  `make_sales_order()`) Graphify asks no question for a top-5 god node with ≥ 2 INFERRED links,
  although it does for the same node in an ego graph. Not explained.
- **Bridge community order**: the `set` iteration order matches for most bridges; for a few
  (graphify `dispatch_command()`, erpnext `make_stock_entry()`) Graphify's insertion order differs
  from sorted-neighbour order.
- **Import cycles**: the same cycles are found where Graphify lists all of them (fastapi, monet,
  erpnext, graphify); the order of equal-length cycles, and therefore which 20 are listed in
  ripgrep, excalidraw and Kubernetes, differs. Graphify's enumeration order is not
  `networkx.simple_cycles` over the arcs in link or `G.edges()` order (tested).
- **Words** count the code files dijkstral indexes; `update` also counts markdown and other
  documents, and lists unclassified files, which dijkstral does not.

---

## 18. C# (2026-10-14)

Fixture `tests/fixtures/csharp/` (file-scoped and block namespaces `Shop.Models`,
`Shop.Services`, `Shop.Tests`; a `partial class User` split over `User.cs` and
`User.Validation.cs`). Goldens: `csharp.graphify.json`, `csharp.clustered.graphify.json` and the
§17.6 report goldens. Probes `p1`–`p6` (namespaces, members, call forms, `new`, `.csproj` and
`.razor`) and the serilog corpus (Graphify's `graphify-out/cache/ast/*.json` per-file facts,
`raw_calls` and `member_field_tables`, were read as observable output). Implementation:
`crates/core/src/{extract,resolve}/csharp.rs`.

### 18.1 Nodes

| Construct | id | label | extra |
|---|---|---|---|
| file | `fileid` (§4.2) | basename | — |
| namespace (every declaration, nested ones joined: `Outer.Inner`) | `csharp_namespace:` + `sha1(name)[:16]` — one node per name for the whole repo | the full name | `type: "namespace"`, `metadata {"kind":"csharp_namespace","namespace":…}`; `source_file`/`source_location` of the first declaring file in path order |
| class, struct, interface, enum, record, `record struct` (**nested types flat**) | `fileid_slug(ns.Name)`, plus `_<arity>` for generic types (`…_ientity_1`, `…_c3_2`); global namespace `fileid_slug(Name)` | `Name` | `_callable`, `_callable_class` |
| method (also interface/abstract/partial, explicit `IX.Do()`); overloads merge, first line kept | `typeid_slug(name)` | `.Name()` | `_callable` |
| property (interface properties too) | `typeid_slug(Name)` | `Name` | — |
| enum member | `enumid_slug(Name)` | `Name` | — |
| method outside any type (tree-sitter error recovery, e.g. serilog `ILogger.cs` broken by `#if`) | `fileid_slug(name)` (no namespace) | `name()` | `_callable` |
| unresolved type (field, parameter, return, base, generic argument, attribute) | `slug(Name)`, **one global node** (no §4.5 renaming) | `Name` | stub shape |
| `using N;` of a non-local namespace, `using static`, alias to a non-local type | `slug(text as written)` (`system_collections_generic`, `system_math`, `system_datetime`) | = id | external |

Every symbol node in a namespace has `metadata` (keys sorted): `is_nested_type: true` (nested
types), `is_partial: true` (a `partial` declaration), `namespace`, `scope_chain` — the
`s<row>:<col>` (0-based) of every enclosing namespace declaration, outermost first (a file-scoped
namespace on line 15 is `["s14:0"]`) — and, cross-file, `unresolved_calls`. Symbols in the global
namespace have no `metadata` unless they are nested, partial or have unresolved calls.

Not emitted: constructors (static ones too), fields, events, indexers, operators, conversion
operators, destructors, delegates, local functions, lambdas, top-level statements and their local
functions, type parameters (no stub).

**Partial types**: every `partial` declaration with the same namespace, enclosing types, name
and arity merges into the declaration of the first file in path order (`User.Validation.cs`
before `User.cs`); its node keeps that file's line and `is_partial`. Members keep their own
file's ids (`src_shop_models_user_shop_models_user_getname` hangs off
`src_shop_models_user_validation_shop_models_user`), and the other parts' `contains` and in-file
edges are redirected to the merged node.

### 18.2 Edges

| relation | source → target | when | context / metadata | location |
|---|---|---|---|---|
| `imports` | file → namespace node / external / local type | every `using` (`global using` too; also inside `#if`) | `import`; `metadata {"using_kind": namespace\|static\|alias, ["alias"], "target_fqn" (as written, HTML-escaped: `IRepository&lt;Shop.Models.User&gt;`), "scope_kind": file\|namespace, ["scope_id": "s1:0"]}` | directive line |
| `contains` | file → namespace node | every namespace declaration | — | declaration line |
| `contains` | file → top-level type; outer type → nested type; file → recovered function | always | — | declaration line (attributes included) |
| `inherits` / `implements` | type → base-list entry | `implements` when the entry resolves to a local interface **or** its name is `I` + capital (`IKlass` naming a class, unresolved `IStubby`); `inherits` otherwise; interfaces always `inherits` | `metadata {"ref_token": Name[, "qualified": true, "ref_qualifier": "System"]}` | type line |
| `references` | type → base generic argument (`IComparable<User>` → `User`, a self-loop is kept) | | `generic_arg` | type line |
| `references` | type → field, property, record / primary-constructor parameter type | not its own type (`static readonly T Instance`) | `field` | member line |
| `references` | method → parameter, then return type; generic arguments of each | | `parameter_type` / `return_type`, `generic_arg` | method line |
| `references` | method → attribute (`[Fact]` → `fact`, `[Obsolete]` → `obsolete`) | method attributes only, not inside `#if` | `attribute` | method line |
| `references` | caller → type argument of a generic method call (`Assert.Throws<AggregateException>()`) | | `generic_arg` | call line |
| `defines` | type → property | always | `field` | property line |
| `method` | type → method | always | — | method line |
| `case_of` | enum → member | always | — | member line |
| `calls` | see 18.4 | | `call` | call line |
| `dispatches_to` | interface method → implementing method | see 18.4 | `call`, INFERRED 0.85 | implementing class line, its file |

All `references`, `inherits` and `implements` carry `metadata.ref_token` (the simple name as
written: `Repo` for an alias); a qualified name adds `qualified: true` and `ref_qualifier` (the
qualifier's raw text). Predefined types (`string`, `int`…), `var`, type parameters, arrays'
brackets, nullability and tuples' parentheses produce nothing themselves (their element types
do). Constructor parameters, property accessors, field initializers, `typeof`, casts, `is`
patterns and `new List<T>()` produce no reference.

### 18.3 Type lookup

**Rule.** A type name resolves as C# would from the namespace declaration enclosing the reference:
its namespace, that declaration's `using` namespaces and aliases, then each enclosing namespace
(`namespace A.B` opens `B` then `A`) with its usings, the global namespace and the file's usings.
`global using` directives in **other** files do not count (serilog's `GlobalUsings.cs`: most test
files reference `LogEvent` as the stub `logevent`). Nested types are found only through their
outer type (`Address` inside `User` → stub `address`; `new User.Address()` binds). Generic types
are looked up by name and arity. No unique-name fallback for references.

### 18.4 Calls

Per file (no `confidence_score`): a bare call `M()`, and a call on any receiver that is not a
name, `this` or `base` (`f().M()`, `a.b.M()`, `kvp.Key.Render()`), binds to a method named `M`
of the enclosing type, else of any type in the file; `new T()` binds to a type `T` declared in the
file (constructors or not). Calls named like a predefined type (`Int()`, `String()`,
`Decimal()`) are ignored. Constructors' calls come from their type; lambdas' and local functions'
from the enclosing method.

Cross-file (unbound bare and `new` calls of the file first, then the rest in source order):

| form | binds to | confidence |
|---|---|---|
| `M()` | the enclosing type's or a base type's `M`, else the only method named `M` | INFERRED 0.85 |
| `this.M()`, `base.M()` | own type and bases / bases | EXTRACTED 1.0 |
| `X.M()`, `X?.M()` with a capitalized `X`; `(x as T).M()`, `((T)x).M()` | `X` as a type (lookup, else the only type named `X`), else the declared type of field/property/local `X` | EXTRACTED 1.0 |
| `x.M()` on a lower-case parameter, local (`T x`, `var x = new T()`, `is T x`), field or property (also `this.x`) of non-predefined declared type | that type (lookup, else unique name) and its bases | INFERRED 0.85 |
| `new T()`, `T` not used as a type in the file | lookup, else the method or type named `T` nearest the caller's directory (`new LogEventProperty(…)` in a test → `Some.LogEventProperty()`) | INFERRED 0.85 |
| `new T()`, `T` used as a type in the file | lookup only, and not when the caller already references `T` | INFERRED 0.85 |

`foreach (T x in …)` and `var x = f()` give `x` no type; an untyped lower-case receiver never
binds. **`metadata.unresolved_calls`** lists, per caller and in source order, one
`{"callee","receiver_type","lang":"csharp","line"}` per (callee, type) for a capitalized or typed
receiver whose type names no local type: `Console`, `Assert`, `Guid`, `List` (from
`List<Address> _addresses`), `T`, `TextWriter` (a property `Output` of that type), and `Street` /
`Renamed` (a property of predefined type, an event: the receiver's own name).

**`dispatches_to`**: for each class listing a local interface in its base list, each interface
method with a same-named method in the class; emitted only when that interface method has exactly
one such implementation in the repo (serilog: 2 of 7 interface methods) and no call already joins
the pair (a call outranks it in the collapse).

### 18.5 `.csproj`, `.sln`, `.razor` in a code-only build

**Open** (not implemented): `.csproj` files get a node (`source_location: null`) with
`framework_<tfm>` nodes (`file_type: "concept"`, per file when the TFM is shared: `…_csproj_framework_net8_0`)
and `…_csproj_sdk_<sdk>` nodes (`references`), `nuget_<package>` nodes labelled `Name (version)`
(`imports`) and `imports` to referenced projects; `.sln` files a bare node; `.razor` files a node,
`route:<path>` nodes, `@using` imports, `@inject` field references and `calls` to components.
Parity runs use `--ext .cs`.

### 18.6 Quirks (not reproduced)

- **CS1** Graphify merges partial types after folding links (dict-update semantics), so a
  `method` link to a member of a non-first part absorbs the reversed `references`' `context` and
  `metadata` (fixture: `User -method-> .CompareTo()` has `context: parameter_type`), and a nested
  class's `inherits` to its outer class can survive as `contains` with the `inherits` metadata
  (serilog `TimeProvider`). dijkstral redirects before the fold.
- **CS2** A generic reference picks a same-named type of another arity (`Gen<T> Self()` inside
  `Gen<T>` → `Gen<T, U>`); dijkstral matches the arity.
- **CS3** A call on an element access (`Receivers[group.Key](…)`) is recorded in
  `unresolved_calls` with text fragments (`callee: "Key]"`); dijkstral records none.
- **CS4** Some typed receivers are not recorded (`ReadOnlySpan<char> context` → `StartsWith`) or
  recorded with the element type (`HashSet<Type>` field → `Type`) in Graphify; not explained.
- Surprises: Graphify lists one surprise per (source label, target label, source files); a C#
  property named like a type (`User -defines-> Status` beside `-references-> Status`) appears once.
  dijkstral applies the same rule (`analyze.rs`); the tie order among equal scores is §17.7's.
## 19. PHP

Fixture `php/`; goldens `php.graphify.json` and `php.clustered.graphify.json` (50 nodes, 71
links). Derived from the fixture, about forty probe repositories and guzzle. Graphify parses
`.php` with tree-sitter-php's PHP-with-HTML grammar (text before `<?php` is skipped, a file of
only HTML or `<?= ?>` keeps its file node); `.phtml .php5 .phps .php8` are not classified, `.inc`
is code that yields nothing.

### 19.1 Nodes

| Construct | Id | Label | Fields |
|---|---|---|---|
| class, interface, trait, enum (any namespace, any depth) | `file_name` | `User` | `_callable`, `_callable_class` |
| method (abstract and interface methods included) | `type_name` (`__construct` → `construct`) | `.__construct()` | `_callable` |
| function at any depth (in `if (!function_exists())`, nested in a function, in a braced namespace) | `file_name` | `slugify()` | `_callable` |
| method of an anonymous class | `file_name` (a function) | `am()` | `_callable` |
| closure / arrow function | `file_closure_<n>`, numbered per file in pre-order | `{closure#n}()` | `_callable` |
| enum case | `enum_case` | `Active` | — |
| per-file type stub (§19.3) | `slug(last segment)` | as written, without namespace | stub shape |
| shared import / supertype stub (§19.3) | `slug(fqn)` / `slug(name)` | the FQN / the name | stub shape |
| `use function A\f;` / `use const A\C;` | external `slug(last segment)` | same as id | external shape |

Not emitted (**Rule**): namespaces, properties, class constants, `require`/`include` (literal or
not), PHPDoc and `// NOTE:` comments (no rationale nodes at all), anonymous classes themselves.

### 19.2 Edges

| Relation | Source → target | When | Confidence | Context | Location |
|---|---|---|---|---|---|
| `contains` | file → class, function, closure | always (closures and nested functions too) | EXTRACTED | — | definition line |
| `method` | type → method | always | EXTRACTED | — | method line |
| `case_of` | enum → case | always | EXTRACTED | — | case line |
| `inherits` | class → `extends`; interface → each `extends` | always | EXTRACTED | — | class line |
| `implements` | class or enum → each interface | always | EXTRACTED | — | class line |
| `mixes_in` | class or trait → each trait of `use T1, T2 { … }` | always | EXTRACTED | — | `use` line |
| `references` | function / method / closure → type | parameter and return types (union, intersection, nullable, DNF members each) | EXTRACTED | `parameter_type`, `return_type` | the function's line |
| `references` | class → type | typed properties and promoted constructor parameters (each member: parameter ref, then field ref) | EXTRACTED | `field` | property line; constructor line for promotion |
| `imports` | file → class or import stub | `use A\B [as C]`, grouped `use A\{B, C}` | EXTRACTED | `import` | statement line |
| `imports` | file → external | `use function` / `use const` (never resolved to a function) | EXTRACTED | `import` | statement line |
| `calls` | caller → callable / class | §19.4 | EXTRACTED (in file), EXTRACTED 1.0 or INFERRED 0.85 | `call` | first call line |
| `references_constant` | caller → class | `X::CONST`, `X::class` inside a function (§19.4) | EXTRACTED 1.0 | — | first use |
| `uses_static_prop` | caller → class | `X::$prop` inside a function | EXTRACTED 1.0 | — | first use |

- **Rule.** Skipped types are tree-sitter's `primitive_type` (`array bool callable false float
  int iterable mixed null object string true void`) and the `never` return type. `self`,
  `static`, `parent`, `never` as a parameter type, `resource`, `integer`, `boolean`, `double`,
  `list`, `Closure` all become stubs. `catch (X $e)`, `instanceof X` and PHPDoc types produce
  nothing.
- **Rule (fold).** `references_constant` ranks above `calls` and below `uses_static_prop`
  (dijkstral: 35 and 36, §8.2): `L::X; new L();` keeps `references_constant` with the call's
  context.

### 19.3 Name resolution

- **Imports.** `use A\B` targets the class whose FQN (namespace + name) is `A\B`, else the
  shared stub `slug("A\B")` labelled `A\B`; never renamed per file. `use A\B as C` maps `C` for
  types (not for calls, below). A qualified type `Psr7\Stream` under a non-aliased
  `use GuzzleHttp\Psr7` names `GuzzleHttp\Psr7\Stream`; an aliased namespace (`use A as P;
  P\X`) is not expanded.
- **Types** (parameters, returns, properties): an imported name → the import target; a class of
  the file → that class; anything else (also `\Fully\Qualified\X`) → a per-file stub of the last
  segment, which the build binds to the only class of that label in the repository.
- **Supertypes and traits** not imported or declared in the file → a stub shared by every file
  (`countable`, `unknown`, never collision-renamed), bound the same way.
- **Quirk (stub reservation).** Every name a file writes in a signature or supertype list
  reserves the per-file id `slug(name)` even when nothing links to it, so two files writing
  `StreamInterface` rename a linked stub to `src_authmiddleware_php_streaminterface` (§4.5)
  although the other file emits no such node.

### 19.4 Calls and constant uses

Calls are attributed to the innermost function, method, closure or anonymous-class method;
top-level code and class bodies have no caller. Deduplicated per (caller, target), first line.

| Form | Result |
|---|---|
| `f()`, `$x->m()`, `$this->m()` with a definition named `f`/`m` in the file | EXTRACTED to the **last** function or method of that name in the file (any class, quirk like Q8) |
| `$x?->m()`, `self::m()`, `static::m()`, `parent::m()`, `\Ns\f()` | dropped |
| `new X` / `X::m()` with class `X` in the file | EXTRACTED to the class (never to the method) |
| `new X` / `X::m()` where `X`'s last segment is a non-aliased import of a class | EXTRACTED 1.0 to the class |
| otherwise (also `new A\B`, `new \X`), unless the file writes `X` as a type | INFERRED 0.85 to the nearest class `X` (§19.5) |
| `X::m()` with a qualified `X`; an aliased `new C` | dropped |
| bare `f()` not in the file | INFERRED 0.85 to the nearest function or method `f` |
| a builtin-like name (`Proxy`, `Promise`, `Map`, `len`, `sorted`, … the Q15 list) as `f()`, `new X` or `X::` | dropped |
| a cross-file call whose caller already links to the target in the file (type reference, constant use) | dropped |

`X::CONST`/`X::class`/`X::$p` link only when the file declares `X` or writes it in a signature
(→ its per-file stub) or a supertype list (→ the shared supertype stub); unqualified names only;
`self::`/`static::`/`parent::` and class-body initialisers produce nothing.

### 19.5 Nearest candidate (PHP and Ruby)

A name bound across files (INFERRED) picks the candidate with the greatest key
(**the candidate is a test file exactly when the caller is**, shared leading directories, same
directory, same file); an equal best binds nothing. Test files: a `test`, `tests`, `spec`, `specs`
or `__tests__` directory, or `*_test`, `*_spec`, `test_*`, `*.test.*`, `*.spec.*`. Measured
pairwise from `rack-protection/spec/support/x.rb`: `test/` beats `rack-protection/lib/rack/`,
`rack-protection/spec/` beats `test/`, `test/` ties `spec/`.

### 19.6 Quirks and open questions (PHP)

- **Quirk.** A class `extends`/`implements` link and a property `field` reference on the same pair
  keep no context in Graphify; dijkstral's fold keeps `field` (guzzle: 2 links).
- **Open.** `use GuzzleHttp\Server\Server;` inside a braced namespace becomes the external
  `server` when a later namespace block of the file defines a function (guzzle
  `tests/bootstrap.php`); dijkstral keeps the class import.
- **Parity (2026-10-14, guzzle, `--ext .php`).** Nodes 4151/4152 (the `server` external), links
  9634/9635 recall and precision; def nodes 100%.

---

## 20. Ruby

Fixture `ruby/`; goldens `ruby.graphify.json` and `ruby.clustered.graphify.json` (56 nodes, 71
links). `.rb`, `.rake` and `ruby` shebang scripts are code (`Gemfile`, `Rakefile`, `.gemspec` are
not). Derived from the fixture, about thirty probes and sinatra.

### 20.1 Nodes

| Construct | Id | Label | Fields |
|---|---|---|---|
| file | `file` | as §5.1 | `metadata.ruby_resolution_schema: 1`, then `ruby_lookup_unsafe`, `ruby_external_method_owners` when set |
| module / class | `file_<slug(qualified name)>` | qualified name: enclosing labels + `::` + the name as written (`Shop::Models::User`, `M::A::B`, `::Rooted`, `HelpersTest::::HelperTwo`) | `_callable`, `_callable_class`; `metadata.ruby_reopened`, `ruby_lookup_unsafe` |
| `X = Class.new(…) [do … end]`, `X = Struct.new(…) [do … end]` | as a class | qualified `M::X` | as a class, always `ruby_lookup_unsafe` |
| method directly in a module/class body or `class << self` | `owner_<method slug>` | `.name()` | `_callable`, `metadata.ruby_method_kind` |
| any other `def` (top level, in a block, under `if`/`begin`, `private def x`, `def obj.x` at top level, in `Module.new do`) | `file_<method slug>` | `name()` | `_callable` |
| per-file superclass stub | `slug(last segment)` | last segment | stub shape |

- **Method slug (Rule).** A trailing `?`, `!`, `=` becomes `_pred`, `_bang`, `_eq` before
  slugging: `valid?` → `valid_pred`, `save!` → `save_bang`, `name=` → `name_eq`, `==`/`===`/`!=`/
  `[]=` → `eq` (so they merge), `!` → `bang`; names that slug to nothing (`<=>`, `+`, `[]`, `<<`,
  `=~`, `-@`) produce no node.
- **`ruby_method_kind`.** `instance`; `singleton` for `def self.x` and defs in `class << self`;
  `ambiguous` for `def Const.x` / `def obj.x` in a body and for any id defined twice (`def a` and
  `def self.a`, `===` and `!=`).
- **Merging (Rule).** One node per id per file, first label and location kept, flags and
  metadata merged: a class reopened in the file gets `ruby_reopened: true`; a module that takes a
  method's id (`def stream` then `module Stream`) keeps `.stream()` with `_callable_class`
  added and gets no `contains` link; a method taking a class's id adds `ruby_method_kind`.
- **`ruby_lookup_unsafe`** is set on the innermost module/class (or the file, at top level)
  whose body, outside any `def` and nested module/class, contains `include`, `extend`,
  `prepend`, `attr`, `attr_reader`, `attr_writer`, `attr_accessor`, `define_method`,
  `define_singleton_method`, `alias_method`, `module_function` with arguments, an `alias`, or a
  `def` that is not a direct method (in a block, `if`, call argument, `Class.new` block). Also
  on `::Rooted` modules and constant-assigned classes. Not: `private`, `delegate`,
  `def_delegators`, bare `module_function`, `method_missing`.
- **`ruby_external_method_owners`** (file, sorted): every module/class name written with `::`
  (`Shop::Service`, `::HelperOne`), every `def Const.m` receiver, and `X::*` for every constant
  assignment `X = …` (plus `Y::*` when the value is a constant `Y`).
- Not emitted: `require`/`require_relative` (no import edges at all), comments and `=begin`
  blocks (no rationale), `attr_*`, `alias`, `define_method`, constants, a `def` inside a method
  body.

### 20.2 Edges

| Relation | Source → target | When | Confidence | Context |
|---|---|---|---|---|
| `contains` | file or enclosing module/class → module/class; file → function, constant class, a module/class defined in a block | always | EXTRACTED | — |
| `method` | owner → method | always | EXTRACTED | — |
| `inherits` | class → superclass | constant superclass: the file's top-level class labelled like its last segment, else the per-file stub (`class User < Base` → `base` even with `Shop::Models::Base` in the repo); `metadata: {"ruby_superclass_ref": written, "ruby_lexical_scopes": [enclosing labels, innermost first]}` | EXTRACTED | — |
| `mixes_in` | module/class → module | `include`/`extend`/`prepend X` directly in the body, `X` resolved lexically (enclosing scopes innermost first, then as below) | EXTRACTED 1.0 | `mixin` |
| `calls` | method/function → method/function/class | §20.3 | EXTRACTED, EXTRACTED 1.0, INFERRED 0.85 | `call` |

### 20.3 Calls

Attributed to the enclosing `def`; class bodies and top-level code have no caller. A bare
identifier that is not a parameter or assigned local is a call; an identifier used as a
receiver (`settings.x`) is not. Builtin-like names (the Q15 list: `print`, `filter`, `hash`,
`len`, `sorted`, …) never bind, `puts` does. Names match the merged node labels (a def whose id
an earlier node took answers to that node's label only).

| Form | Result |
|---|---|
| `m`, `m()`, `self.m` in a method | EXTRACTED to the owner's method `m`, else a function `m` of the file |
| `x.m`, `@x.m`, `::IO.m`, `call.m` | EXTRACTED to the **last** definition named `m` in the file (quirk like Q8: `admin.promote` → the caller itself) |
| any call inside a function | EXTRACTED to the last definition of that name in the file |
| bare `m` not bound in the file | INFERRED 0.85 to the nearest method or function `m` (§19.5) |
| `Foo.new`, `Foo::Bar.new` | EXTRACTED 1.0 to the only module/class labelled `Foo` or `…::Foo` (no lexical lookup: `Request` is ambiguous with `Sinatra::WebDAV::Request` from `Sinatra::Base`) |
| `Foo.m` | EXTRACTED 1.0 to `Foo`'s method `m` of any kind (`module_function` or not), else to `Foo` itself |
| `self.m` / `x.m` not bound in the file, `super` | dropped |

### 20.4 Quirks and open questions (Ruby)

- **Quirk.** `def self.run(server, port)` merged with a later `def run` loses its parameters as
  locals, so `port` becomes a bare call (sinatra: 1 link); dijkstral keeps the parameters.
- **Open.** The location of a deduplicated self-call (`call` → `call`) differs in 3 sinatra
  links.
- **Parity (2026-10-14, sinatra, `--ext .rb`).** Nodes 1343/1343, links 2054/2055 recall and
  precision, identical communities (NMI 1.0), no node field differences.

## 21. Kotlin

Fixture `tests/fixtures/kotlin/` (Gradle layout, packages `com.shop.{model,service,util}`, a test
under `src/test/kotlin`, a root `Main.kt` and `build.gradle.kts`). Goldens `kotlin.graphify.json`,
`kotlin.clustered.graphify.json` and the §17.6 report goldens; Graphify's per-file cache
(`graphify-out/cache/ast/<version>/*.json`, which also lists each file's unresolved `raw_calls`
and its `kotlin_package`) was the main oracle. Grammar: `tree-sitter-kotlin` 1.1.0 (the crate
`tree-sitter-kotlin-ng` 1.1.0, same parse trees). `.kt` and `.kts` are both Kotlin; file ids drop
only the final extension (`build.gradle.kts` → `build_gradle`). Validated on okhttp (§21.5).

### 21.1 Nodes

| Construct | id | label | extra |
|---|---|---|---|
| class, interface, `object`, `data`/`sealed`/`value`/`enum`/`annotation` class, `fun interface`, `data object` — nested at any depth | `file_slug(Name)`, flat | `Name` | `_callable`, `_callable_class` |
| companion object (named or not) | none: its members belong to the enclosing class | | |
| top-level function, extension function (`fun User.displayName()`) | `file_slug(name)` | `name()` | `_callable` |
| method (class, object, companion, interface with or without body, enum entry) | `owner_slug(name)`; overloads merge (first line) | `.name()` | `_callable` |
| enum entry | `enum_slug(NAME)`; its body's methods `entry_m` | `NAME` | none |
| anonymous object in a function body | `<function>_object_slug(Type)_l<line>` (first supertype, last segment) | `Type` | `_callable`, `_callable_class` |
| type referenced but not declared earlier in the file | per-file stub `slug(Name)` | `Name` | §4.4 |
| unresolved import | external `slug(last segment)` | | §4.4 |

- Not emitted: properties (top-level or member), type aliases, constructors (primary or
  secondary), `init` blocks, accessors, local functions and classes, script statements of `.kts`
  files, and rationale (KDoc, `// NOTE:` comments).
- **Rule (walk).** Declarations are found through any non-declaration node, except function
  bodies: functions inside a misparsed region (a class whose header failed to parse, an
  `annotated_expression` swallowing a class) become top-level functions. Inside a class body,
  declarations nested in other nodes (`init { body = object : Sink { override fun close() } }`,
  local `val`s in a secondary constructor) are taken as **top-level** declarations of the file:
  `close()` is `file_close`, `contains` from the file (Q20).
- **Quirk Q19.** A definition whose id equals the file's internal id `slug(path with
  extension)` merges into the file node: `fun kt() {}` in `Main.kt` (`src_main_kotlin_main_kt`)
  adds `_callable` to the file node and a `contains` self-loop; `class Kt` adds
  `_callable_class` too, and its methods keep the unmerged prefix (`a_kt_kt`). This is §14.5's
  "yields no symbols".

### 21.2 Edges

All in-file edges: EXTRACTED, no score, weight 1.0, `source_file` of the source construct.

| relation | source → target | when | context | location |
|---|---|---|---|---|
| `contains` | file → top-level class/function; class → nested class; function → anonymous object | always | none | declaration line (annotations included) |
| `method` | class/object/entry/anonymous object → method | always | none | method line |
| `case_of` | enum → entry | always | none | entry line |
| `inherits` | class → supertype written as a constructor call (`Base()`, `Result()`) | | none | class line |
| `implements` | class → any other supertype (`Iface`, `Comparable<User>`, `Iface by impl`) | | none | class line |
| `references` | class → type of a `val`/`var` constructor parameter | plain parameters excluded | `field` | the parameter's line |
| `references` | class → type of a member property (class body, companion; anonymous object properties to the object) | | `field` | property line |
| `references` | function → parameter types / return type | receiver types excluded | `parameter_type` / `return_type` | function line |
| `references` | any of the above → type arguments, recursively (`Map<String, List<Deep>>` → `deep`), and supertypes' arguments (not for anonymous objects) | | `generic_arg` | as above |
| `references` | class/function/property owner → every annotation (`@Throws`, `@get:JvmName`, `@field:Json`; constructor-parameter annotations to the class); class annotations also → their `X::class` arguments (`@RunWith(AndroidJUnit4::class)`) | | `attribute` | annotated declaration's line |
| `imports` | file → import target (§21.3), `metadata: {"target_fqn": …[, "alias": …]}` | every non-wildcard import | `import` | import line |
| `calls` | function/method → function/method/class/anonymous object; class or file → … from property initializers | §21.4 | `call` | call line |

- **Type names.** A reference to a qualified type uses its **first** segment
  (`Interceptor.Chain` → `Interceptor`, `java.io.File` → `java`; quirk Q21); supertypes and
  annotations use the last (`b.Base<T>` → `Base`). Nullable types count; function types
  (`(Foo) -> Bar`, `Foo.() -> Unit`), star projections, bounds and `where` clauses produce
  nothing. Type parameters are stubs (`T` → `t`, per file).
- **Denied types.** About 100 common Kotlin and JDK names never produce a reference or stub
  (`String`, `Int`, `List`, `MutableMap`, `Exception`, `IOException`, `Instant`, `File`,
  `TimeUnit`, `Runnable`, `Random`, `Executor`…; the list is in `extract/kotlin.rs`). The list
  does **not** apply to supertypes (`: Closeable`, `object : Runnable`) or annotations.
- **Local types** bind by name to a class declared **earlier** in the file (or the class being
  declared): a nested `Builder` referenced before its declaration is a stub.
- References dedupe per (source, target, context, line).
- No `uses`, `indirect_call` or `rationale_for`.

### 21.3 Imports

The per-file target is `slug(last segment)` (aliases ignored; `import a.b.*` produces nothing).
Graphify then binds it, in order:

1. by **FQN** to the only definition whose `package + "." + label` equals it, over every
   non-method node of Kotlin and Java files: classes, objects, nested classes by their flat
   label, anonymous objects, **enum entries** and top-level functions. `okhttp3.Dns` stays
   unbound because a test enum in package `okhttp3` has an entry `Dns`; nested FQNs
   (`okhttp3.CallEvent.CallStart`) never match.
2. to the file's own stub of that name (a type the file uses), which then binds like any stub;
3. to any other node with that id (another file's unrenamed stub);
4. to the only file whose stem slugs to it (§8.1): `import okhttp3.Request` with
   `Request` ambiguous → `Request.kt`; the edge is dropped when that file is not Kotlin;
5. else the external node `slug(last)`.

### 21.4 Calls

In the file, calls bind by name to the **last** function, method, class or anonymous object of
that name in the file, whatever the receiver (`repo.save(u)` → the file's last `save`,
`name.compareTo(…)` inside `compareTo` → itself). Enum entries do not bind. One edge per
(caller, callee), first line kept. Names in the call denylist never bind: the Python builtins
`abs all any bool bytes callable delattr dict dir enumerate filter float getattr hasattr hash id
int isinstance iter len list map max min next object open print range repr reversed round set
setattr sorted str sum super tuple type vars zip`, the JS globals `isNaN isFinite parseInt
parseFloat decodeURI decodeURIComponent encodeURI encodeURIComponent`, and the constructor names
`String Error Date JSON Array Map Set Object Promise Math Number Boolean Symbol URL Int Float
Bool Int32 Int64 RegExp Proxy Reflect File Response Request Headers Color Font UUID Uuid Data
BigInt WeakMap WeakSet URLSearchParams FormData Blob Intl Character Double Decimal EvalError
TypeError RangeError SyntaxError Locale` (probed one by one).

Walked: function bodies first (pre-order: `a.b().c()` visits `c`, `b`), then property
initializers (caller: the class, or the file at the top level; `by lazy {}` included). Not
walked: local functions, accessors, `init` blocks, constructors, default arguments, `.kts`
statements other than property declarations.

Across files (`confidence_score` set), skipping any (caller, target) pair that already has an
edge (`f(x: Foo) { Foo() }` keeps only the reference):

1. **Bare calls** `name(…)`: an import whose last segment is `name` and whose FQN binds (§21.3)
   → EXTRACTED 1.0 (`UserService(…)` imported in `Main.kt`). Otherwise, unless the file uses a
   type of that name (`via_stub` in Graphify's cache), the nearest definition of that name
   (enum entries and anonymous objects included) → INFERRED 0.85: most shared leading
   directories, then same directory; a tie binds nothing, a sole candidate binds even with no
   shared directory. When that fails, a member import `import okhttp3.TestUtil.assumeNetwork`
   binds `assumeNetwork()` to that object's method (INFERRED 0.85).
2. **Object receivers** `Type.m(…)` (a capitalised identifier receiver): when `Type` names exactly
   one Kotlin class and that class (companion included) has exactly one method `m` → EXTRACTED
   1.0. Overloaded `m`, `Named.Factory.m()` and Java receivers bind nothing; the denylist does
   not apply (`Adapters.any(…)` binds).
3. Calls on any other receiver (`x.m()`, `this.m()`, `super.m()`) never bind across files.

### 21.5 Parity

Fixture: 100% of nodes and links, communities and array order exact, report goldens exact.
Corpus okhttp (`.kt`/`.kts`, frozen reference): file and def nodes 100%/100%; all nodes 99.9%
recall / 100.0% precision; links 99.8%/99.8% (`calls` 99.7/99.8, `imports` 99.8/99.8,
`references` 99.9/99.7, all other relations ≥ 99.6). Remaining: in-file call targets where
Graphify prefers a cross-file import over a same-file anonymous object of the same name, a few
proximity ties, and Java-side effects (Graphify's Java pass sees Kotlin nodes, dijkstral's does
not; §16 externals as collision emitters).

---

## 22. Scala

Fixture `tests/fixtures/scala/` (packages `shop.{model,service,util}`, Scala 3 top-level `def`s,
`enum`, `extension`, a self type, a munit suite). Grammar `tree-sitter-scala` 0.26.2. Only
`.scala` is code: `.sc` and `.sbt` are "not classified" (probe `s0`). Validated on munit (§22.4).

### 22.1 Nodes

| Construct | id | label | extra |
|---|---|---|---|
| `class`, `case class`, `trait`, `object`, `case object`, `enum`, named `given` — nested at any depth | `file_slug(Name)`, flat; a companion object shares its class's id (second `contains`) | `Name` (the given's name) | `_callable`, `_callable_class` |
| `def` (abstract or not, `def this(…)` too) in a class/trait/object/enum | `owner_slug(name)`; overloads merge | `.name()` | `_callable` |
| top-level `def`, `def` in a package object or `extension` | `file_slug(name)` | `name()` | `_callable` |
| enum case | `enum_slug(Case)` | `Case` | none |
| any type name not declared in the file | per-file stub `slug(Name)` — **no denylist**: `String`, `Int`, `Unit`, `List`, `Option` are stubs | `Name` | §4.4 |
| import | external `slug(first path segment)` | | §22.2 |

Not emitted: `val`/`var`, type aliases, anonymous classes (`new Runnable { … }`), local `def`s,
annotations, rationale. Q19 applies (`def scala()` in `Main.scala`). Declarations found inside
expressions of a class body or in a `val`'s value are top-level declarations of the file
(`test("…") { val user: User = … }` gives `file -references-> user field`; `val anon = new
Base { def m() }` gives the function `file_m`).

### 22.2 Edges

| relation | source → target | context | location |
|---|---|---|---|
| `contains` | file → top-level class/def; class → nested class/given | none | declaration line |
| `method`, `case_of` | class → `def`; enum → case | none | its line |
| `inherits` | class → first type of `extends` (trait or class alike) | none | that type's line |
| `mixes_in` | class → each `with` type | none | that type's line |
| `requires` | trait → every type named in its self type (`this: Base[Int] with M =>` → `base`, `int`, `m`) | none | self-type line |
| `references` | class → every primary-constructor parameter type (all parameter lists, `val` or not); typed `val`/`var` and type alias of a class body (file for top-level ones) | `field` | parameter / declaration line |
| `references` | `def` → first parameter list's types / return type | `parameter_type` / `return_type` | `def` line |
| `references` | → type arguments, recursively | `generic_arg` | as above |
| `references` | `def` → context bound (`[T: Type]`) | `type_bound` | `def` line |
| `imports` | file → `slug(first segment)` (`import shop.model.{A, B => C}` → `shop`; one edge per import, also inside class bodies) | `import` | import line |
| `calls` | `def` → `def`/class/case | `call` | call line |

- Supertypes carry no type-argument references, and a supertype naming the class itself
  (`object Assertions extends Assertions`) is dropped; references never point at their source.
- Type forms: tuple and compound types reference each part, a projection `A#B` both names, a
  function type only its result (`() => Future[T]` → `future`, `t`), by-name (`=> T`) and
  repeated (`T*`) parameters nothing; a qualified type (`mutable.Map[K, V]`) references only its
  arguments; supertypes use the last segment (`scala.annotation.StaticAnnotation`).
- **Local types** bind when `file_slug(Name)` is a node emitted earlier in the file — any class,
  object, `given` or top-level `def` (`Ordering[Foo]` binds to `given ordering`; a parameter
  `context: AfterEach` binds to an earlier top-level `def afterEach`).
- Import targets bind to an existing node with that id (another file's stub `diff` → its class),
  else to the only file with that stem (§8.1), else an external; an import of the file itself
  (`import JUnitReporter._` in `JUnitReporter.scala`) is dropped.

### 22.3 Calls

In-file binding as Kotlin (§21.4: last definition of that name, same denylist), over `def` bodies
only: `val` initializers, object bodies and anonymous class bodies are not walked, and
`new X(…)` and generic calls `f[T](…)` are not calls. Across files only bare calls bind,
INFERRED 0.85 by the §21.4 proximity rule (no import-based binding, no object receivers).

### 22.4 Parity

Fixture: 100% nodes and links, communities and order exact, analysis and cluster-only goldens
exact (the `update` report lists `build.sbt` as unclassified, §17.7). Corpus munit (`.scala`):
nodes 99.9%/99.9%, links 99.9%/99.9% (calls, imports, references, method, mixes_in, requires
100%). Remaining: two stubs Graphify renames because a Java file's external import node of the
same id counts as a collision emitter (§16), and one self-`contains` of nested same-slug
classes (`object a { case class A() }`).

## 23. Swift

Derived black-box (2026-10-14) from the fixture `tests/fixtures/swift/`, small probe repositories
and a scratch copy of the Alamofire corpus. Graphify parses `.swift` with tree-sitter-swift 0.7.4
(dijkstral uses the same grammar version). Its per-file cache (`graphify-out/cache/ast/…/*.json`)
also records, for Swift, `raw_calls` (calls the file could not bind), `swift_type_table` (the
variable-type table and `factory`) and `swift_extensions`; dijkstral reproduces those facts per file
exactly on Alamofire (134 files), and they are the oracle for the rules below. Implementation:
`crates/core/src/{extract,resolve}/swift.rs`.

### 23.1 Nodes

| Construct | id | label | extra |
|---|---|---|---|
| file | `fileid` | unique suffix (§5.1) | — |
| `class`, `struct`, `enum`, `actor`, `protocol`, at any depth (nested types are flat) | `fileid_slug(Name)` | `Name` | `_callable`, `_callable_class` |
| `extension T` | `fileid_slug(text of T)` (`extension Swift.Array` → `…_swift_array`, `extension [String]` → `…_string`) | the type text (`Swift.Array`, `[String]`, `AFError.ServerTrustFailureReason`) | `_callable`, `_callable_class` |
| method, `init`, `deinit`, `subscript`, protocol requirement (with or without body) | `typeid_slug(name)` (`…_init`, `…_subscript`) | `.name()` | `_callable` |
| computed property (`{ get … }`, `{ … }`) and observed property (`willSet`/`didSet`) | `typeid_slug(name)` | `.name` | none |
| top-level function | `fileid_slug(name)` | `name()` | `_callable` |
| enum case, one per name (`case a, b(Int)`) | `enumid_slug(name)`, line of the name | `name` | none |
| `import M` / `import struct M.T` | `slug(M)`, shared, never collision-renamed | `M` as written (`Foundation.Date`) | `type: "module"`, `source_file`/`source_location` of the importer whose **basename** sorts first (then path), **Rule** |
| undeclared type in a supertype list | `slug(Name)`, shared by every file (no collision rename) | `Name` | stub shape (§4.4) |
| undeclared type anywhere else | `slug(Name)`, per file (§4.5 rename applies) | `Name` | stub shape |

Not emitted: stored properties, protocol property requirements, typealiases, associated types,
operators (`static func ==`), nested functions, closures, global variables and top-level
statements, types declared inside function bodies, and **no rationale nodes** (`///` doc comments
and `// NOTE:`/`// WHY:` are ignored). Generic parameters are ordinary undeclared types (`T` stub).

- **Rule: same-id definitions merge, first node wins, later ones only add missing fields.** An
  overloaded `init` keeps the first line; a case `convertToSnakeCase` merged with a later
  `static func convertToSnakeCase` keeps the case's label and gains `_callable`; a method
  `singleValueContainer()` and a later extension `_URLEncodedFormEncoder.SingleValueContainer`
  share one id and the node gains `_callable_class`.
- **Rule: one pass.** A type name refers to a node of the file defined *so far*: its id is
  `fileid_slug(Name)`. Otherwise the first use makes a stub — shared when that use is a supertype,
  per file otherwise — and later uses reuse it until the file declares or extends that name, from
  which point they target the file's node. Forward references therefore make stubs, which the
  cross-file pass usually binds (23.4). `Swift.Error` names `Swift`, whose id
  `fileid_swift` is Graphify's internal id of the file node itself: such references and
  supertypes target the **file node** (**Quirk S1**).

### 23.2 Edges

All `EXTRACTED`, `weight 1.0`, `_origin ast`, `source_file` = the file of the source construct.
Structural edges carry no `context` key.

| relation | source → target | context | line |
|---|---|---|---|
| `imports` | file → module node | `import` | import line |
| `contains` | file → top-level type, extension or function; type → nested type | none | declaration (start of the node, attributes included) |
| `method` | type or extension → method, `init`, `subscript`, `deinit`, computed/observed property | none | member line |
| `case_of` | enum → case | none | line of the case name |
| `inherits` | protocol → each supertype; class/actor → first supertype unless it is a protocol the file defined so far; later supertypes when they are classes; struct → a class of the file | none | declaration |
| `implements` | enum, extension → every supertype; class/actor → later supertypes and a known protocol first; struct → non-class supertypes | none | declaration |
| `references` | type → stored or computed property type | `field` | property line |
| `references` | function → parameter type / return type | `parameter_type` / `return_type` | function line |
| `references` | enum → associated value types | `type` | case line |
| `references` | any of the above → type arguments (also a supertype's: `WKHostingController<ContentView>`) | `generic_arg` | as above |
| `calls` | function, property or type (stored property value) → member, function or type | `call` | call line |

- **Rule: type names.** A `user_type` names its *first* identifier (`Outer.Inner` → `Outer`,
  `T.Type` → `T`); type arguments become `generic_arg`; attributes in types (`@Sendable`) are
  types too, `@escaping`/`@autoclosure` parameter modifiers are not. A parameter written with
  attributes (`h: @escaping @Sendable (Int) -> Void`) names **only** the attributes: Graphify reads
  the grammar's `type` field, which holds just the `type_modifiers`. Self-references are skipped
  (`indirect case nested(EI)`). References dedupe per declaration by (target, context).
- **Rule: `metadata.swift_plain_return`.** On the first `return_type` reference when the return
  type is written as a plain named type (`-> User`, `-> Result<A, B>`, `-> Outer.Kind`,
  `-> Self`, `-> Base!`), not for `T?`, `[T]`, tuples, functions or `some P`. The cross-file pass
  uses it to type `let x = Type.make()`.
- **Rule: per-file edge order.** Structure in pre-order, then every in-file `calls` edge of
  function bodies in walk order, then those of stored-property values.

### 23.3 Calls inside the file

Members are found by the kept node's label (`.name()`/`.name` stripped): `_response()` merged
into `.response()` is no member `_response`. Cases are no members (they have no `method` edge).

| call | binds to |
|---|---|
| `name(…)` | a member of the caller's type or its superclasses in the file, else a type of the file with exactly that label (its initializer → the type node), else the file's last definition of `name` when that is a top-level function; else recorded |
| `super.m(…)` | the superclass's member (superclass declared in the file), else recorded with no receiver |
| `self.m(…)` | the own type's member, else recorded with no receiver |
| `x.m(…)`, `T.m(…)`, any receiver | the file's **last** function, member or type named `m`, regardless of the receiver (**Quirk S2**, like Q8), else recorded |

- Builtin-like names never bind in the file: Python builtins and JavaScript globals as for Go
  (but `pow` binds), plus `Bundle Calendar Data DateFormatter DispatchGroup DispatchQueue Double
  JSONDecoder JSONEncoder NSError NSLock NumberFormatter OperationQueue TimeZone UInt32 UInt64
  UInt8 URLComponents URLRequest URLSession UUID` (observed). Bare calls with them are dropped;
  member calls are recorded.
- One edge per (caller, target), first line. Self-loops are kept (`cancel()` inside `cancel()`).
- Scanned: function, initializer and computed/observed property bodies, closures inside them and
  types declared inside them. Not scanned: nested functions, subscript bodies. A stored
  property's value is scanned only when it is a call (`= Cache()`, `= { … }()`), with the type as
  the caller; a closure or literal value is not.
- Recorded calls carry the receiver `x` of `x.m()`, `x.y.m()` and `self.x.m()`; longer chains,
  calls and literals give none. `via_stub` marks a bare call whose name the file uses as an
  undeclared type.

**Variable-type table** (one flat map per file, by variable name): parameters (any function,
protocol requirements included) and typed properties, last write wins; over locals initialised
by `Type(...)` or `Type.member` (capitalised identifier), first write wins. The value is the
first type name of the annotation in the 18.2 sense. `let x = Type.m(...)` records
`factory[x] = (Type, m)` when `x` has no entry yet. Locals with only a type annotation, closure
parameters, `for`/`if let`/`guard let` bindings and top-level statements are not recorded.

### 23.4 Cross-file pass

1. **Extensions merge** into the one declaration of their label (a redirect; methods keep their
   extension-scoped ids). An extension the declaring file writes itself shares the declaration's
   id and makes that id an extension too, so a type extended in its own file is **no merge target**
   (**Quirk S3**: Alamofire's `Request`, `AFError`, `Protected` keep separate extension nodes).
2. **Stubs** (per-file and shared) bind to the one class-like node of their label, declaration or
   extension, after merging. Unbound shared stubs become one node each.
3. **Bare calls** (all files, before member calls): the definition of that name — function,
   member, property, case or type, by stripped label — nearest to the caller's file (same file,
   then most shared leading directories); a tie binds nothing. INFERRED 0.85. A `via_stub` call is
   dropped.
4. **Member calls**: the receiver's type comes from the table, or for a `factory` variable from
   the plain return type of `Type.m`. It must name exactly one class-like node that is a
   declaration or lies in the caller's file; then the member `m` → `calls` INFERRED 0.85, or, when
   the type has no such member, `references` to the type INFERRED 0.85 (context `call`). Without
   a table entry, a capitalised receiver is the type itself (`User.make()`) and the same edges are
   EXTRACTED 1.0. A type that no class-like node has is recorded once per (caller, callee, type) in
   the caller's `metadata.unresolved_calls` (`lang: "swift"`), except builtin-like types (the
   list above, `Int`, `Set`, `Sendable`, `Locale`). Ambiguous or extension-only types elsewhere
   give nothing.
5. A cross-file edge is skipped when the caller already links to the target (any relation), which
   is why an INFERRED edge can shadow a later EXTRACTED one.
6. **`dispatches_to`**: a protocol with exactly one conforming type (`implements`) links each
   requirement to that type's member of the same name, INFERRED 0.85, context `call`, at the
   conforming type's node (`source_file`, `source_location`).

### 23.5 Build-stage observations

- Swift structural edges lack a `context` key, so a folded edge keeps the other edge's context:
  `contains` + a later `references field` on the same pair gives `contains` with
  `context: "field"`, and in the opposite orientation `implements` + `references field` keeps
  `implements` with `context: "field"`; a `calls` edge folded over a `references return_type` loses
  `swift_plain_return`. dijkstral's fold (§8.2) merges no keys into a surviving `contains`/`method`
  and none across orientations (**Quirk S4**; 61 link fields on Alamofire).
- Opposite-orientation ties compare Graphify's internal ids, where a file node is
  `fileid_swift`: for `extension Alamofire.HTTPHeader: Swift.Decodable` the extension's
  `implements → file` beats the file's `contains` (dijkstral keeps `contains`; 4 links).
- A per-file stub whose id equals a shared supertype stub of another file is collision-renamed
  by Graphify (`…_swift_codingkey`); dijkstral exempts shared stubs from §4.5 counting and merges
  them (3 nodes, ~12 links). A renamed stub that collides with an extension id
  (`…_swift_bool` vs `extension Swift.Bool`) gets no sha1 suffix in Graphify (1 node).

### 23.6 Quirks

| # | Quirk | dijkstral |
|---|---|---|
| S1 | `Swift.X` resolves to the file node | reproduced |
| S2 | `x.m()` binds to the file's last `m` whatever `x` is | reproduced |
| S3 | a type extended in its own file is no merge target for other files' extensions | reproduced |
| S4 | key merging on folded Swift edges (23.5) | not reproduced (shared fold) |
| S5 | module nodes take the location of the importer with the smallest basename | reproduced |

### 23.7 Parity (Alamofire, Graphify's frozen reference, `.swift` only)

Nodes: def 2635/2635 both ways, stubs 648 shared (+1/−3); links: calls, case_of, dispatches_to,
imports, inherits and method 100%/100%, references 99.4%/99.4%, implements 97.8% recall,
contains 99.2% precision. Per-file facts (nodes, edges in order, raw calls, tables) match
Graphify's cache for all 134 files.

### 23.8 Open

- Whether an extension constructing its own type from its body binds in general: the only such
  call Alamofire has outside the denylist (`CharacterSet(...)` in a property closure of
  `extension CharacterSet`) binds; `DispatchQueue(...)` and `Bundle(...)` are denylisted.
- The full builtin-type denylists (calls and `unresolved_calls`) are known only for the names
  Alamofire exercises.

### 23.9 Analysis

The god-node noise labels (§17.4) keep `String` out of the god nodes, but Graphify still lets an
`extension String` symbol be a surprise endpoint and a bridge (fixture `swift`, `cluster-only`).
dijkstral's analysis applies the noise labels to all three, so `dijkstral cluster-only` on the
Swift golden differs in surprises and bridge questions (verified by making `String` eligible: only
the god list then differs). The test `swift_cluster_only_matches_graphify` is ignored until the
analysis distinguishes the two lists. Equal-score surprise ties also follow insertion order, where
Graphify inserts module nodes at their first import and shared stubs at first use; dijkstral adds
both after all files.
## 24. C

Fixture `tests/fixtures/c/`; goldens `c.graphify.json`, `c.clustered.graphify.json` and the report
goldens of §17.6. Extensions `.c` and `.h`; tree-sitter-c 0.24.2 (Graphify's version). Rules were
isolated with probe repositories run through `graphify extract --code-only`; jq (`.c`/`.h`) is the
corpus check. Implementation: `crates/core/src/{extract,resolve}/c.rs` (shared with C++).

### 24.1 Which language a file is

- **Rule.** `.c` is C. `.h` is C++ when its bytes contain any of `class `, `namespace `,
  `template`, `::`, `public:`, `private:`, `protected:` anywhere, comments included (case
  sensitive, plain substrings: `// subclass Foo` makes a header C++, `// class` followed by a
  newline does not, `extern "C"` and `__cplusplus` do not). Every other `.h` is C. Probed up to
  200 KB into a file.
- C headers therefore get no class, struct or field nodes even in C++ projects (jq's headers,
  fmt's C++ headers are C++).

### 24.2 Nodes

| Construct | id | label | extra |
|---|---|---|---|
| file | file id | §5.1 | — |
| function definition | `file_slug(name)` | `name()` | `_callable: true` |
| type name used in a signature (`Shape`, `AreaFn`, `FILE`) | `slug(name)`, per file (§4.5) | name | stub |
| `#include` with no local file | §24.4 | | external |

- **Rule.** Function definitions are found anywhere outside function bodies: in preprocessor
  branches (`#if`/`#else` both), `extern "C"` blocks, and the `ERROR` nodes of error recovery
  (jq's `util.c`, `jv_dtoa.c`, decNumber). A definition the parse nested inside another
  function's body is not a node.
- Not emitted: prototypes, `struct`/`union`/`enum` (with or without body), typedefs, macros
  (`#define`), globals, and comments: C has no rationale nodes (`NOTE:`/`WHY:`/`HACK:` ignored).
- Same-file duplicates (two `static` definitions under `#if` branches) keep the first (§8.1).

### 24.3 Edges

| relation | source → target | context | location |
|---|---|---|---|
| `contains` | file → function | — | definition line |
| `references` | function → type stub; return type first, then each parameter | `return_type` / `parameter_type` | function line |
| `imports` | file → file / stub / external (§24.4) | `import` | directive line |
| `calls` | function → function (§24.5) | `call` | call line |

- Only `type_identifier` types produce references: primitive types (including `size_t`,
  `uint8_t`, `bool`), `struct X` / `enum E` and types inside function-pointer parameters do not.
- Stubs never bind to anything in a pure C repository (there are no type nodes to bind to); in a
  mixed repository they bind like C++ stubs (§25.4).

### 24.4 Includes

`#include "p"` resolves to the indexed file at `p` relative to the including file's directory
(`../include/util.h` works; a path relative to the repository root or to an include directory does
not). `<p>` never resolves that way. Otherwise the target is the external id
`E = slug(stem of p)` (`<sys/types.h>` → `types`), and Graphify then lands the edge, in this
order, on:

1. the stub of the first header (`.h`/`.hpp`, by path) that has a renamed (§4.5) stub `E`
   (`#include "jv.h"` from `tests/` → `src_jv_h_jv`; fmt's `<vector>` →
   `test_gtest_gmock_gmock_h_vector`);
2. a node of the including file with local id `E` (its own stub `Shape` next to
   `#include "shape.h"`), renamed and bound like that stub;
3. any node whose id is `E` (an unrenamed stub elsewhere);
4. the only definition labelled exactly `E` (a class, field or enumerator `E`, or a function
   `E()`; not a method): fmt's `<ostream>` → class `ostream`, `"fmt/printf.h"` → `printf()`;
5. the only file whose stem is `E`, in any language (§8.1; jq's `"version.h"` →
   `scripts/version`);
6. the external node `E` (label = id, §4.4).

A macro include (`#include FOO_H`) emits nothing. Each directive gives an edge; duplicates fold
as usual.

### 24.5 Calls

- **In the file**: a call `f(...)`, and also `x.f()` / `x->f()` whatever `x` is, binds to the file's
  node answering to `f` (call name: label without `.` and `()`; functions, and in C++ methods,
  classes, fields, enumerators). Among several, the **node created last** wins; a later
  definition that merged into an existing id (§8.1) does not count. EXTRACTED, no score.
  Recursion gives a self-loop.
- **Across files**: a bare `f()` with no `f` in the file binds by path proximity, exactly as Go's
  bare calls (§16): the candidate sharing the longest directory prefix with the caller, a tie
  binding nothing, a candidate sharing no directory only when it is the only one. INFERRED 0.85.
  Graphify adds a file's cross-file calls in callee-name order (observable in surprise ties).
- Builtin-like names never bind by name (in-file, cross-file or as a qualified fallback): the
  Python builtins and JavaScript globals of Go's list minus the names Graphify does bind
  (`format`, `compile`, `slice`, `eval`, `exec`, `chr`, `hex`, `oct`, `ord`, `pow`, `input`, `help`,
  `globals`, `locals`, `property`, `staticmethod`, `classmethod`, `frozenset`, `bytearray`,
  `memoryview`, `ascii`, `bin`, `breakpoint`, `complex`, `divmod`, `issubclass`, `require`, typed
  arrays, `Function`, `DataView`, `ArrayBuffer`); 82 names, listed in `extract/c.rs`.
- Calls through pointers (`fp(x)`, `(*fp)(x)`), macros and calls at file scope bind nothing; one
  edge per (caller, callee), first line kept.

### 24.6 Quirks (C)

- **C1** The C/C++ split of `.h` is a substring test that also reads comments.
- **C2** Unresolved includes land on unrelated stubs and definitions that share the name (§24.4
  steps 1–4): an `imports` edge to a type stub or to a function.
- **C3** `s->free(x)` binds to a function `free` of the same file (member calls bind by name).
- **C4** Header and source with the same directory and stem (`jv.h`, `jv.c`) collide on their
  file ids and get renamed (`src_jv_h_src_jv`), as do their shared stubs.

---

## 25. C++

Fixture `tests/fixtures/cpp/` (also `.cu` and a C++ `.h`); goldens `cpp.*`. Extensions `.cpp .cc
.cxx .hpp .cu` and C++ headers (§24.1); tree-sitter-cpp 0.23.4. Corpus check: fmt. Everything in
§24 applies; this section lists what C++ adds.

### 25.1 Nodes

| Construct | id | label | extra |
|---|---|---|---|
| class / struct / union / enum with a name | `file_slug(Name)`, **flat** (namespaces and outer classes are not part of it) | `Name`; a specialization keeps its arguments (`Arr<bool, 1>`) | `_callable`, `_callable_class` |
| enumerator | `enum_slug(NAME)` | `NAME` | — |
| field | `class_slug(name)` | `name` (`x_`, `kind_`) | — |
| method defined or declared in the class body | `class_slug(name)` | `.name()`; `.~Shape()`, `.operator==()` | `_callable` |
| function, and function template | `file_slug(name)` | `name()` | `_callable` |
| out-of-line definition `R A::f(...)`, `ns::f` | `file_slug(A::f)` | `A::f()` as written (`Registry<T>::first()`, `geo::detail::round_up()`) | `_callable` |
| base or type stub | §25.3 | | stub |

- **Rule.** A named class specifier anywhere outside a function body is a class contained by the
  file, including forward declarations (`struct Fwd;`), elaborated types in declarations
  (`struct S *gs;`) and in prototype parameters. Not: inside function bodies (local classes),
  in a field's type (`struct Node *next;`), in friend declarations, or anonymous ones (their
  enumerators and fields are dropped too).
- A class or enum defined as a field type in a class body (`class Inner {...};`) is contained by
  the outer class; its id is still flat (`file_inner`).
- Class bodies: field declarations give fields (`int *p, **q;` two fields; array and bit-field
  declarators count; reference declarators and function-pointer fields give nothing) or, with a
  function declarator, a method declaration; function definitions give methods (with
  `= default`/`= delete`). Constructor declarations (`explicit Shape(Kind);`, a `declaration`
  node) give nothing. Every other construct of the body is walked like file scope: a template
  member (`template <class U> U conv()`) becomes a file-level function `conv()`, and anything
  inside `#if` in a class body becomes file level (methods turn into functions `get()`, fields
  vanish).
- Destructor and constructor share an id (`~Outer` slugs to `outer`); overloads merge (§8.1).
  Operator names slug to `operator`; conversion operators (`operator bool`) and declarators
  behind a reference (`Outer &operator=(...)`, out-of-line too) give nothing.
- Namespaces, `using`, typedefs, aliases, macros and comments give nothing (no rationale).
- **Misread classes.** `class MACRO Name { ... }` (fmt's `FMT_API`, gtest's `GTEST_API_`) parses
  as a function definition; Graphify still emits class `Name`. With a base list the name is in an
  `ERROR` node and the base is the declarator (an `inherits` edge). In the body (a compound
  statement) declarations with a function declarator, also one moved into an `ERROR` child or a
  `void f()` read as a variable initialised with `()`, are methods; other declarations without
  an `ERROR` child are fields; function definitions are methods; labels (`public:`) are walked;
  everything else, including classes and functions the recovery swallowed after the class, is
  file level.

### 25.2 Edges

| relation | source → target | context | location |
|---|---|---|---|
| `contains` | file → class / function / out-of-line definition; outer class → nested class | — | definition line |
| `method` | class → method | — (`field` when a field shares the id, either order) | method line |
| `defines` | class → field | `field` | field line |
| `case_of` | enum → enumerator | — | enumerator line |
| `inherits` | class → base | — | class line |
| `references` | class → field type (before the `defines`) | `field` | field line |
| `references` | function or method **definition** → return / parameter type | `return_type` / `parameter_type` | function line |
| `references` | the same → template arguments (`std::vector<Inner>` → `Inner`) | `generic_arg` | |

- Method declarations carry no references; qualified types reference their last name
  (`std::string` → stub `string`, `geo::Point` → `Point`); `auto`, primitives and `decltype` give
  nothing.

### 25.3 Type targets and bases

- **Rule.** A type name binds in its file to whatever node has the id a file-level definition of
  that name would have (`file_slug(name)`), a class but also a function or a field (fmt binds base
  `testing::Test` to the macro function `TEST()`); otherwise to a per-file stub.
- A base names its stub after the base text minus its first qualifier: `Tmpl<int>` → `Tmpl`,
  `::testing::Test` → `testing::Test`, `detail::buffer<char>` → `buffer<char>` (id
  `buffer_char`); the in-file lookup uses the innermost name (`buffer`) and never the class
  itself (`struct is_contiguous : detail::is_contiguous<T>` → stub `is_contiguous<T>`).
- Stubs bind across files to the only definition labelled like them (classes, enumerators,
  fields): `Rect` with a class `Rect` and an enumerator `Rect` stays a stub.

### 25.4 Calls

In-file binding is §24.5, with two additions:

- In a method defined in a class body, `f()` and `this->f()` first take the class's own member `f`.
- Template calls `f<T>(...)` and `obj.template f<T>()` bind nothing.

Qualified calls `A::...::f(...)` (scope = outermost qualifier, name = the rest):

1. scope names exactly one class (not templated: `formatter<std::tm, Char>::parse` skips this)
   → its member (method, field or enumerator) `f`, else the only out-of-line definition
   `A::f()`; EXTRACTED 1.0; a method never calls itself this way;
2. otherwise the file's node answering to the rest of the name (`ns::helper()` → `helper()`,
   `std::f()` → a local `f()`, `internal::FilePath::MakeFileName` → the out-of-line
   `FilePath::MakeFileName()`), EXTRACTED; never across files.

Typed calls: `x.m()` / `x->m()` with no `m` in the file use `x`'s type, taken from the file's
**first** local declaration of `x` (any function, earlier or later) that has a single declarator
and a capitalised type name (`Shape s;`, `Shape *p = ...;`, `const Shape &r = ...;`,
`geo::Shape g;` → `Shape`, condition declarations; not parameters, `auto`, templates
`Box<int> b`, `Shape a, b;`, range-for). The class's member `m` binds INFERRED 0.85; no
inheritance walk, out-of-line definitions are not members here.

`metadata.unresolved_calls` (§6), `{"callee","receiver_type","lang":"cpp","line"}` in source
order, once per (callee, type) and caller: typed calls whose type names no class
(`BiggestConvertible parsed; parsed.push_back(...)`), and qualified calls whose scope starts with a
capital and names no class (`L::lm()`, `TypeParameterizedTest<...>::Register`).

### 25.5 Declaration and definition merge

**Rule.** A header and its implementation file in the same directory with the same stem share a
file id prefix, so an out-of-line definition `K::a()` gets the id of the header's method `.a()`.
Graphify does not rename this pair apart (§4.5): the method node stays (its label, line and
fields) and gains `definition_file` and `definition_location` (the first definition's), the
definition's `contains` and calls attach to it. Other ids the two files share are renamed as
usual (their file nodes: `x_h_x`, `x_cpp_x`).

### 25.6 Quirks (C++)

- **CPP1** Flat class ids: same-named nested classes of one file merge (`F::Inner`, `G::Inner`).
- **CPP2** `#if` inside a class body moves its members to file level.
- **CPP3** Typed calls take the first capitalised declaration of the variable name anywhere in
  the file; parameters are ignored.
- **CPP4** Type names bind to any same-id node (a base binding to a `TEST()` function).
- **CPP5** In-file binding ignores receivers (`g.go()` on a `G1` binds `G2::go` when defined later).
- **CPP6** `#include <x>` can land on a stub, a field or a function named `x` (§24.4).

### 25.7 Open (C and C++)

- **Self-calls in large files.** fmt shows no self-loop for about 120 calls dijkstral resolves to
  the caller itself by the rules above (`fmt_.begin()` in `parse_context::begin`, `detail::compare`
  in `compare`), while every probe keeps such self-loops (`k(n-1)`, `d::f(x)`, `items_.size()`).
  Not explained; possibly a parse difference between py-tree-sitter 0.25.2 and the Rust runtime
  0.25.10 on error recovery.
- **Header capture order.** §24.4 step 1 picks the first header by path; only the jq, fmt and
  probe cases pin it.
- **Error recovery.** A few fmt and gtest regions (methods attached to `buffer`, iterator buffer
  fields) come out of the parse differently, which moves `contains`/`references` lines and a
  handful of nodes.

---

## 29. Elixir

Fixture `tests/fixtures/elixir/`; goldens `elixir.*` (53 nodes, 67 links, §17.6 report goldens).
Extensions `.ex` and `.exs`; tree-sitter-elixir 0.3.5 (Graphify's version). Derived from about
fifteen probe repositories, Graphify's per-file cache (`raw_calls` with `elixir_module` and
`elixir_call_scope`) and ecto. Implementation: `crates/core/src/{extract,resolve}/elixir.rs`.

### 29.1 Nodes

| Construct | id | label | fields |
|---|---|---|---|
| `defmodule A.B` / `defprotocol A.B` not inside another module | `file_slug(A.B)` | `A.B` | `_elixir_module: true` |
| the same nested in a module (any depth of its body) | `file_slug(name as written)` (not prefixed by the parent) | name as written (`Stats`) | — |
| `defimpl P, for: X` | `file_defimpl_slug(P)_slug(X)` | `P (for X)` | — |
| `defimpl P` (no `for:`, or `for:` a list or `__MODULE__`) | `file_defimpl_slug(P)` | `P` | — |
| `def`, `defp`, `defmacro`, `defmacrop`, `defguard`, `defguardp`, `defdelegate` | `owner_slug(name)` | `name()` (`valid?()`, `promote!()`, `__using__()`) | — |
| the same at file level (no module) | `slug(path WITH extension)_slug(name)` (`lib_a_ex_top`, quirk EX1) | `name()` | — |

- **Rule.** Definitions are found at any depth of a module body outside function bodies: inside
  `if`/`else`, `for`, `quote`, `describe`, `Enum.each(fn -> ... end)`. A `defmodule` inside a
  top-level `if` is still top level (`_elixir_module`). Nothing inside a function body is a node
  (`defmodule`, `def` and `quote do def ... end` in a `defmacro __using__`).
- The name of a `def` is the head's call target (`f(x)`, `f(x) when g`), a bare identifier
  (`def eight, do: ...`) or the left operand of an operator head (`def a + b` → `a()`).
  `def unquote(name)(x)` gives nothing. Every clause of a multi-clause function merges into the
  first (§8.1); `?`/`!` vanish from ids, so `dump` and `dump!` merge and keep the first label.
- A `defmodule`, `defprotocol` or `defimpl` whose name is not an alias (`defimpl encoder, for:
  X` in ecto's `json.ex`) yields nothing, its body included.
- Not emitted: `defstruct`, `defexception`, `@callback`/`@spec`/`@type`, `@behaviour`, module
  attributes, `test`/`describe`/`setup` blocks, comments and `@moduledoc`/`@doc` (no rationale;
  `NOTE:`/`WHY:` ignored).

### 29.2 Edges

| relation | source → target | when | location |
|---|---|---|---|
| `contains` | file → `defmodule` (nested ones too); enclosing module → nested `defprotocol` or `defimpl`; file → top-level `defprotocol`/`defimpl`; file → file-level function | always | definition line |
| `implements` | defimpl → protocol | a `defprotocol` of the same file, defined earlier, named like the defimpl's protocol (as written or after alias expansion); never across files | defimpl line |
| `method` | module, protocol or defimpl → function | one per clause (the build keeps the last clause's line) | clause line |
| `imports` | file → module, file or external | §29.4 | statement line |
| `calls` | function → function | §29.3 | call line |

### 29.3 Calls

Only calls inside function bodies count: the `do` block, or the value of a `do:` keyword written
on the keyword's line (anonymous functions and `quote` blocks inside included). A `do:` followed
by a line break reads no body (EX7: ecto's `defp escape_bind(...), do:` with `error!(...)` on
the next line calls nothing). Module bodies, file-level code, heads, guards and default
arguments give nothing.

- **Call sites.** A `call` node whose target is an identifier is a bare call, unless the name is
  `def defp defmacro defmacrop defguard defdelegate defstruct defmodule defimpl defprotocol case
  cond if unless with for import alias use` or in the builtin denylist (C's list of §24.5
  without `object`, plus `require`: `map filter max min len print hash id ...`); `raise`, `quote`,
  `unquote`, `try`, `receive`, `send`, `apply`, `inspect` and `@attr value` (`doc`, `impl`) are
  calls. `x |> f()`, `&(&1 + f(&1))` and string interpolation call `f`; `&f/1` does not;
  `&Mod.f/2` does. A target `Mod.f` is a member call: `Mod` is alias-expanded (§29.5), `__MODULE__`
  and `__MODULE__.X` are the current module; a variable, call or atom receiver (`x.f`, `:ets.f`,
  `f.()`) never binds.
- **In the file** (no score): a bare `f` binds to the **last** function of the file labelled
  `f()`, whatever module defines it (EX2); `Mod.f` binds when the expanded `Mod` is the full
  name of a module of the file (a `defimpl P, for: X` is the module `P.X`), to its function
  with id `module_slug(f)`, so by id rather than label (EX8: `Ecto.Schema.__field__` reaches
  `field()`). One edge per (caller, callee), first line; a call that would bind to the caller
  itself stays unbound.
- **Across files.** `Mod.f` binds to the function labelled `f()` (any kind, `defp` included; by
  label here, so `Ecto.primary_key!` misses the merged `primary_key()`) of the only
  **top-level** module labelled `Mod` (EXTRACTED 1.0); nested modules are invisible here
  (`Shop.Stats.count` from another file binds nothing, EX3). A bare `f` binds to the only
  function labelled `f()` in the files that define the caller's scope modules (INFERRED 0.85):
  the `import`/`use` targets of the enclosing modules, innermost first, then the file level, as
  written (`require` and `alias` are not scope). No scope, two candidates, or a candidate equal
  to the caller bind nothing. Self-loops never appear.
- Graphify adds the cross-file calls in target-id order (`f → B.c` before `f → B.y`), which
  decides surprise ties (§17.4).

### 29.4 Imports

`alias`, `import`, `use` and `require` anywhere outside function bodies (module body, file level,
nested modules) give one edge per target from the **file** node: `alias A.B` / `as:` → `A.B`;
`alias A.{B, C.D}` → `A.B`, `A.C.D`; `alias __MODULE__.Sub` → `__MODULE__.Sub` (external
`module_sub`); atoms (`import :timer`) give nothing. The target `E = slug(text)` lands on, in
order:

1. the only top-level module whose `slug(label)` is `E`, when it is in another file (a same-file
   match, or two matches, skip this step: `use Twice` with `Twice` in two files stays external);
2. an existing node whose id is `E` (`alias Lib.A` → file `lib_a`);
3. the only module, protocol or file of the graph keyed `E`, where a file's keys are the slugs
   of its path suffixes without extension (`v`, `w_v`, `lib_w_v` for `lib/w/v.ex`) and a
   module's are those keys plus `_` plus its id's local part (`q_r` for nested `R` in `lib/q.ex`);
   in another file it is linked only if the importing file has no import edge to that node yet
   (Graphify adds these after the others); in the importing file itself the edge is **dropped**
   (`import Config` in `config/config.exs`, `use Supervisor` in `supervisor.ex`, EX4);
4. otherwise the external node `E` (label = id).

### 29.5 Module names and aliases

A top-level module's full name is its label. A nested module's full name is its parent's plus
`.` plus the name as written, and its first segment becomes an alias in the parent for the whole
parent body (`defmodule Shop.Other` inside `Shop.A` makes `Shop` mean `Shop.A.Shop`, so
`Shop.B.run` there names `Shop.A.Shop.B`, EX5). `alias` in a module body applies to the whole
body, before or after it, and to nested modules; in a function body only to that function.
Expansion replaces the first segment (`F.deep` with `alias Shop.{E.F}` → `Shop.E.F`).

### 29.6 Quirks (Elixir)

- **EX1** File-level functions keep the extension in their id prefix (`lib_a_ex_top`).
- **EX2** A bare call binds to the file's last same-named function, even in another module.
- **EX3** Nested modules are reachable only from their own file.
- **EX4** Import targets are matched by id slugs and path suffixes, not module names; an import
  that lands in its own file is dropped.
- **EX5** A nested `defmodule A.B` re-aliases `A` for its whole parent, retargeting unrelated
  `A.X` calls.
- **EX6 (not reproduced)** An import matched by step 3 of §29.4 to a node of the importing file
  other than the file itself: Graphify keeps either the `contains` link (with `context:
  "import"`) or the import (probes disagree); dijkstral drops the import.
- **EX7** A `do:` keyword body that starts on the next line is not read.
- **EX8** In-file `Mod.f` matches by id, cross-file `Mod.f` and bare calls by label.

### 29.7 Parity (2026-10-14)

ecto (`--ext .ex,.exs`): nodes 1954/1954 and links 3943/3943 (recall and precision 100%), no field
differences, identical communities (NMI 1.0) and array order. The Graphify repository's
`sample.ex` matches the same way on a scratch copy.

---

## 30. Erlang

`.erl` and `.hrl` are code files (counted in the report's corpus check, §17.5) but Graphify
0.9.86 extracts nothing from them: their grammar comes from the optional extra `graphifyy[erlang]`
(`tree-sitter-language-pack`), which the pinned install lacks, and the build logs "contributed
nothing to the graph because a dependency is missing". No file node, no edges. A repository with
only Erlang files yields an empty graph and no clustered output. Dijkstral classifies the files
(`Lang::Erlang`), parses nothing and emits nothing. Fixture `erlang/` (Python tools carry the
graph; `rebar.config` is not code and is left out so the report compares exactly).

## 31. OCaml

The same as §30 for `.ml` and `.mli` (optional extra `graphifyy[ocaml]`, `tree-sitter-ocaml`):
code files with no nodes. Fixture `ocaml/`; dune files are not classified.

## 32. Julia

Fixture `tests/fixtures/julia/`; goldens `julia.*` (32 nodes, 42 links). Extension `.jl`;
tree-sitter-julia 0.23.1 (Graphify's version). Implementation:
`crates/core/src/{extract,resolve}/julia.rs`. There is no cross-file call resolution.

### 32.1 Nodes

| Construct | id | label |
|---|---|---|
| `module M` (nested too) | `file_slug(M)` | `M` |
| `abstract type T`, `struct T`, `mutable struct T{P}` | `file_slug(T)` | `T` |
| `function f(...)`, `f(...) = ...` (also with `where`, `::R`) | `file_slug(f)` | `f()` |
| `function Base.show(...)` | `file_slug(show)` | `Base.show()` |
| `Base.:+(a, b) = ...`, `function +(a, b)` | the file id (the name slugs to nothing, JL1) | `Base.:+()` |
| `(c::Circle)(x)` | `file_circle_call` | `(Circle)()` |
| `macro m(...)` | `file_slug(m)` | `@m` |
| `using A, B.C: x`, `import ..P`, `using .Q` | `slug(path)` (no file prefix; per file, so two files importing `Base` both get renamed, §4.5) | `A`, `B.C`, `P`, `Q` (leading dots dropped) |
| type name in a field or `<:` not defined earlier in the file | stub `slug(T)` | `T` |

- Every id hangs off the file id, whatever module encloses the definition. Methods of one
  function merge (§8.1); a struct and a function of the same slug merge too.
- Definitions are found at any depth outside function bodies (`begin`, `let`, macro calls such as
  `@inline f(x) = x`). Not nodes: `function f end`, `primitive type`, `const`, globals,
  `export`, `import X as Y`, `include`, docstrings and comments (no rationale), functions nested
  in a function body.

### 32.2 Edges

| relation | source → target | notes |
|---|---|---|
| `defines` | file → every module (nested ones too); enclosing module (or file) → type, function, macro | one per method, at its line |
| `imports` | enclosing module (or file) → import node | context `import`, statement line |
| `inherits` | type → parent | `<: P` or `<: P{...}`; qualified parents give nothing |
| `references` | struct → field type | context `field`; only plain names (`x::T`), not `Vector{T}`, `Union{...}`, `Main.T`, nor the struct's own parameters |
| `calls` | function → `file_slug(callee)` | §32.3 |

A type name binds in the file only to a node already created with id `file_slug(T)`; otherwise
a stub, which the build binds to the only Julia node labelled `T` (type, module or import node),
else to the only function labelled `T()`; with two candidates it stays a stub.

### 32.3 Calls

Every call in a function body (calls in the signature and default arguments excluded) targets
`file_slug(callee)` where the callee is the identifier, the last name of `a.b.c(...)` or
`x.m(...)`, or the operator of `Base.:+(...)` (the file id, JL1); broadcast `f.(x)`, `x |> f`,
calls in comprehensions, `do` blocks, lambdas, `begin`/`let` and macro arguments count; a nested
short-form definition counts as two calls. The edge survives only when that id exists, so calls
bind to the file's own definitions (constructors of its structs included) and never across files;
self-calls stay. No denylist, no dedup (the build keeps the last line).

### 32.4 Quirks (Julia)

- **JL1** Operator methods (`Base.:+`, `function +`) take the file's id and merge into the file
  node; calls to them link to the file. Graphify keeps `file -defines-> module` over the reverse
  `module -defines-> file` (smaller source on an equal-rank tie), while dijkstral's fold keeps the
  larger source in that tier (§8.2): a shared-rule difference, not observed outside probes.
- **JL2** Calls are bound by synthesized id, so a call named like another file's id slug would
  link there; cross-file calls are lost.

### 32.5 Parity

No Julia corpus repo. The Graphify repository's `sample.jl` matches exactly on a scratch copy
(23 nodes, 27 links with `sample.ex`). In the whole repository its ids are collision-renamed
(§4.5) against `sample.ps1` and `sample.f90`, languages dijkstral does not extract yet, so five
definitions there differ by id until those languages land.
## 38. Vue (2026-10-15)

Fixture `tests/fixtures/vue/` (a Vue 3 todo app: `<script setup lang="ts">`, a dual
`<script>` + `<script setup>` component, an Options API component, a template-only component and
an HTML comment holding a `<script>` tag) with the usual goldens; about thirty probes. Graphify
counts `.vue` (and `.svelte`, `.astro`) as code and has **no component grammar**: it finds the
script by text and runs its TypeScript/JavaScript extractor (§10) on it. Implementation:
`crates/core/src/extract/sfc.rs`, which blanks the rest of the file and calls the existing
`extract::script::extract` unchanged; resolution is `crate::resolve::script`.

### 38.1 Script regions and grammar

- **Rule.** Every `<script …>…</script>` pair, case-insensitive (`<SCRIPT>`, attributes over
  several lines), **including ones inside HTML comments** (`<!-- <script>function f(){}</script> -->`
  yields `f()`), is code; everything else is ignored, so lines and ids are those of the outer
  file. All blocks form **one module**: a function in `<script>` calls, or is called through an
  import made in, `<script setup>` as if they were one file.
- **Grammar** (Rule): the first `lang` attribute on any `<script>` tag (case-insensitive, quoted or
  bare; `lang` on `<style>`/`<template>` does not count): `tsx` → TSX; `js`, `jsx` → JavaScript;
  anything else (`ts`, `typescript`, `javascript`) or none → **TypeScript**. So JSX in a block
  without `lang` swallows the declarations after it (error recovery nests them: `src_s_j_ok2`).
- **Default exports** (Rule, observed): which declaration is a module's default export is decided
  on a **TypeScript** parse whatever the grammar: the first `export default` function, generator
  or class declaration at any depth (a default declared after JSX still counts), or an
  identifier. `export default function F() { return <div/> }` in a `lang="tsx"` block becomes a
  function expression under TypeScript, so `import F from './F.vue'` links nothing.
- A file with no script (template only) keeps its file node and "yields no symbols" (§14.5).

### 38.2 What differs from a `.ts` file

- **VU1** No rationale nodes: `// NOTE:` and `* WHY:` lines inside the script are ignored.
- Imports resolve like TS (§10.3): `./Foo.vue` by its exact path; an extensionless `./Foo` never
  tries `.vue` (it becomes `ref_foo`); `@/x/Foo.vue` from `src/x/` is `ref_x`.
- Calls, import-scoped calls (no `confidence_score` in raw output, also across blocks), object
  literal methods (`data()`, `methods: { … }` → module-level functions), heritage, type references,
  `type_only` and default imports behave as in TS.
- **Self imports.** A module importing itself (`import { k } from './K.vue'`) keeps its `imports`
  links, but the clustered build drops the `imports_from` self-loop; this holds for `.ts` too, so
  `resolve::script` skips it for every module (shared rule change).

### 38.3 INFERRED member calls across component kinds

Raw output links `x.m()` (typed receiver, §10.4) into and out of components, but the clustered
build keeps an INFERRED member call that touches a component only for Vue → Vue, Vue → Svelte and
Svelte → Svelte (probes: TS ↔ Vue, TS ↔ Svelte, TS → Astro dropped; Svelte → Vue, anything from
an Astro file never linked). TS/JS among themselves are unaffected (`.ts` → `.js`, `.tsx` kept).
`resolve::script` applies the same table (shared rule change).

### 38.4 Stem collisions

`c.vue` beside `c.ts` collide like any other files (§4.5): `c_vue_c`, `c_ts_c`; symbol ids do not
collide unless the names do.

### 38.5 Golden and quirks

- Golden: nodes 44/44, links 84/84 after the one allowed Q8 link (`.mount()` binding to the
  `mount()` function in `main.ts`); communities identical.
- TS behaviours seen through Vue and not reproduced, as for `.ts`: Q8 (`this.other()` inside an
  options-API method binds to the flattened `other()`), and Graphify emits uninitialised
  `export let x` late without a `contains` link (dijkstral links it).

### 38.6 Analysis (Open)

`dijkstral cluster-only` on the Vue golden asks one more bridge question than Graphify:
`formatTitle()`, ranked fourth by betweenness, whose only neighbour in another community is its
own file node. Graphify's two bridges are consistent with not counting same-file neighbours when
listing the communities a bridge connects; unverified on other graphs, so `report.rs` is unchanged
and `vue_cluster_only_matches_graphify` is ignored.

## 39. Svelte (2026-10-15)

Fixture `tests/fixtures/svelte/` (SvelteKit layout: `<script context="module">` plus instance
script, props, `$:` statements, `$lib/…` imports, a markup-only component, `+page.ts`). Script
regions as §38.1; the grammar is JavaScript for `lang="js"`/`"jsx"` and TypeScript otherwise
(`tsx` included).

Svelte goes through a reduced TS path. Compared with §38 (Rule, all observed):

- No import-scoped call pass: calls from module-level functions to imported functions carry
  `confidence_score` 1.0, like calls from methods (only the edge order changes once clustered).
- No `type_only`, no `imports` link for a **default** import (`import Cart from …`), no
  `inherits`/`implements`, no type `references` (`field`, `parameter_type`, `return_type`).
  Named imports, decorators (`references`, `decorator`) and `indirect_call` remain.
- An exported variable without a value (`export let item: Item`, the props idiom) has no node.
- No rationale.

Quirks (not reproduced):

- **SV1** With two `<script>` blocks, a later pass reads the second block at shifted offsets:
  stray nodes named after neighbouring tokens appear without `contains` (`export let n: number`
  gives a node `number`), and once a default import was linked (`imports` to `Counter`).
- **SV2** (Q9 family) Every import whose specifier does not resolve to a local file also gets a
  phantom code node (id `slug(last segment)`, `$lib/Cart.svelte` → `cart_svelte`; a missing
  relative path → `slug(resolved path without extension)`), labelled with the specifier,
  `source_file` the specifier or resolved path, `confidence` and no location, plus an
  `imports_from` link without context, line or weight, beside the normal link to `ref_lib`. Some
  of these links disappear in the clustered build (`pkg/C.svelte`). Dijkstral keeps the external
  only.

Golden: nodes 27/27 and links 45/45 after the two phantom nodes and links (SV2); NMI 1.0.

## 40. Astro (2026-10-15)

Fixture `tests/fixtures/astro/` (pages, a layout, components with frontmatter, a client
`<script>`, a React island `.tsx`). The frontmatter body (the file's first line is `---`, up to the
next `---` line) and every `<script>` block form one TypeScript module; a `---` later in the
markup is not a fence. Everything else is §39: no import-scoped pass, no default-import links,
no heritage or type references, no `type_only`, no props without a value, no rationale, and the
SV2 phantoms for unresolved imports (`@/H.astro` → `h_astro`). INFERRED member calls involving an
Astro file are never kept (§38.3).

Golden: nodes 22/22, links 32/32, identical communities, analysis and report.

## 41. PowerShell (2026-10-15)

Fixture `tests/fixtures/powershell/` (`build.ps1`, a dot-sourced `scripts/Common.ps1`, a module
`Modules/Shop/` with manifest `Shop.psd1`, `Shop.psm1` with classes and an enum, a nested
`Private/Pricing.psm1`, a Pester test). `.ps1`, `.psm1` and `.psd1` are code, parsed with
tree-sitter-powershell 0.26.4 (Graphify's version; the Rust crate of the same version is used).
Probes `ps1`–`ps5` and the graphify corpus fixtures. Implementation:
`crates/core/src/{extract,resolve}/powershell.rs`. Nothing carries `_callable`; there is no
rationale.

### 41.1 Nodes

| Construct | Id | Label |
|---|---|---|
| `function`, `filter`, `workflow`, at any depth outside class methods (nested, in script blocks) | `file_slug(name)` | `name()` (`global:Scoped()`, id `…_global_scoped`) |
| `class` | `file_slug(Name)` | `Name` |
| method or constructor (overloads merge, first line kept) | `class_slug(name)` | `.Name()` |
| `enum`, member | `file_slug(Name)`, `enum_slug(Member)` | bare |
| unresolved type | `slug(Name)`, per file (§4.5 applies) | `Name` as written, last segment |
| import target | external `slug(stem)` | = id |

Functions, classes and imports inside a class method body are ignored (only its calls count).

### 41.2 Edges

| Relation | Source → target | Context | Line |
|---|---|---|---|
| `contains` | file → function (nested ones too), class, enum | — | statement |
| `method` | class → method, once per definition (overloads repeat; the build keeps the last) | — | method |
| `case_of` | enum → member | — | member |
| `inherits` / `implements` | class → first base / each further base | — | base name |
| `references` | class → property type (`field`; never its own class); method → return type (`return_type`), then each typed parameter (`parameter_type`) | as listed | the type literal's line |
| `imports_from` | file → external or file | none; `import` in a manifest | statement / manifest key |
| `calls` | function or method → function or method | none in-file; `call` + INFERRED 0.85 across files | call |

**Types** (Rule): `[T]` names the last segment of a plain type name (`[System.IO.FileInfo]` →
`FileInfo`); array (`[string[]]`) and generic (`[List[string]]`) types give nothing; function
`param()` blocks, `[OutputType()]` and attributes give nothing. A type binds to a class or enum of
the file, else to the file's stub, which the cross-file pass binds to the only PowerShell class or
enum with that name **unless that class's id was renamed** by §4.5 (then the stub stays). Classes
in other languages never count (`[UniquePy]` stays a stub).

**Imports** (Rule):

- `Import-Module` (any case) with a bare word (the first `generic_token`, parameters skipped) →
  stem of its last `/` or `\` component (`Az.Accounts` → `az`, `C:\abs\Win.psm1` → `win`,
  `./b.c.psm1` → `b_c`).
- Dot-sourcing a bare path (`. ./Helpers.ps1`, `. .\Utils.ps1`) → its stem.
- `using namespace|module|assembly X` → the last `.` segment of `X` (`System.IO` → `io`,
  `using module ./Shop.psm1` → `psm1`).
- Nothing for quoted or variable paths (`Import-Module "$PSScriptRoot/x.psm1"`,
  `. $PSScriptRoot\x.ps1`), `&` invocation, aliases (`ipmo`), `#Requires -Modules`, or imports
  inside class methods. Imports inside functions come from the file.
- `.psd1` manifests: in the top-level hashtable, unquoted keys exactly `RootModule` and
  `NestedModules` (every string inside, in order, nested hashtables included: `ModuleVersion =
  '1.0'` gives `1`) and `RequiredModules` (strings and `ModuleName` entries, **sorted**), each the
  stem of the string, context `import`, line of the key.
- An external whose id equals `slug(stem)` of exactly one file of the repository, **in any
  language**, is replaced by that file (`. .\Utils.ps1` → `worked/httpx/raw/utils.py` in the
  graphify corpus; `Private/Pricing.psm1` → the module file); a file importing itself loses the
  link.

**Calls** (Rule): callers are functions and methods (top-level code has none; a nested function
owns its own calls; calls in script blocks belong to the enclosing function). In the file, a
command name binds case-insensitively to a function of the file, `$this.M()` to the class's method
and `[Class]::M()` to a method of a class of the file, EXTRACTED without context, once per pair,
grouped by caller in definition order. Other command names go to the function **or method** of
exactly that (case-sensitive) name nearest to the caller's file (§19.5), INFERRED 0.85 with
context `call`; `[Other]::M()` across files, `$x.M()`, `& F` and `[T]::new()` give nothing.

### 41.3 Parity

- Golden: nodes 43/43, links 63/63, identical communities, analysis and report.
- Graphify corpus fixtures (`--ext .ps1,.psm1,.psd1`, frozen reference): nodes 25 shared of 28,
  links 27 shared of 34 (theirs) / 36 (ours), NMI 1.0. Every difference comes from §4.5 renames
  caused by `tests/fixtures/sample.{zig,jl,java}`, which define `Shape`, `Circle` and a
  `DataProcessor` constructor: dijkstral does not extract Zig and (in this branch) Julia, and in
  Graphify the `inherits` and constructor `method` links between two renamed nodes of
  `sample.ps1` are lost (**Open**: a probe with only Python collisions keeps them).
  `sample.cls` is Apex (§42), not PowerShell.

## 42. Visual Basic (2026-10-15)

Graphify 0.9.86 classifies `.vb` as code but parses it only with the optional
`tree-sitter-vb-dotnet` grammar (`pip install "graphifyy[vbnet]"`), which the reference install
lacks: every `.vb` file "contributed nothing to the graph" — **no file node**, no symbols. A
VB-only fixture (`tests/fixtures/vb/`) gives an empty raw graph (`vb.graphify.json`) and the
clustered build stops with "graph is empty". Dijkstral classifies `.vb` (it counts toward the
corpus check) and emits nothing; on an empty corpus it writes an empty graph instead of stopping.
A Rust crate `tree-sitter-vb-dotnet` 0.1.0 exists, but there is no reference output to match.

Related extensions: `.cls` is code in Graphify but extracted as **Apex** (the graphify corpus'
`sample.cls`: `AccountService`, `.createAccounts()`, DML `uses`), not VB6, and is not implemented
here; `.vbproj` is code (an MSBuild file, like `.csproj` §18.5, not implemented); `.bas`, `.frm`
and `.vbs` are not classified.
