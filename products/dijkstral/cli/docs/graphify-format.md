# Graphify `graph.json` format: black-box specification

This document specifies what `graphify extract <dir> --code-only` writes to `graph.json` for Python,
TypeScript/TSX, JavaScript/JSX, Rust, Go, Java, Kotlin and Scala, plus what JSON, TOML and shell files
contribute.
The reference is PyPI `graphifyy` 0.9.97 (with its `sql` extra). The spec was first written against
0.9.86 and moved to 0.9.97 on 2026-10-29: the fixture goldens were regenerated, and every changed
rule was found by running both releases on the same probes; where the text names 0.9.86 it records
what changed. It was derived only by running those binaries on inputs and reading the outputs: the fixtures under `tests/fixtures/`, small probe repositories, and scratch
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
33. Zig
34. Objective-C
35. Fortran
36. Verilog (`.v`)
37. Solidity
43. C# project and markup files
44. Other code-counted files
26. Groovy
27. Dart
28. Lua
45. SQL
46. YAML

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
| `zig/` | `main.zig`, `util.zig`, `src/{shapes,hue,lonely}.zig`, `tests/main_test.zig` | `zig.graphify.json` | `zig.clustered.graphify.json` |
| `objc/` | `main.m`, `Shop/Bridge.mm`, `Shop/Models/{Shape,Circle}.{h,m}`, `Shop/Models/Util.h` (C), `Shop/Services/Repo.{h,m}` | `objc.graphify.json` | `objc.clustered.graphify.json` |
| `fortran/` | `utils.f90`, `src/{constants,geometry,shapes}.f90`, `legacy/solver.f90`, `app/main.f90` | `fortran.graphify.json` | `fortran.clustered.graphify.json` |
| `verilog/` | `rtl/{top,alu,fifo}.v`, `mem/ram.v`, `counter.v`, `tb/tb_top.v`, `tb/model.py`, `vlang/main.v` (a V-language file), `empty.v` (0 bytes) | `verilog.graphify.json` | `verilog.clustered.graphify.json` |
| `solidity/` | `contracts/{Base,Token}.sol`, `scripts/deploy.py` | `solidity.graphify.json` | `solidity.clustered.graphify.json` |
| `csproj/` | `Shop.sln`, `Shop.slnx`, `src/{Shop,Core}/*.csproj`, `tests/Shop.Tests/Shop.Tests.csproj`, `legacy/{Legacy.vbproj,Broken.csproj}`, `src/Shop/Pages/{Index,Widget,_Imports}.razor`, `src/Shop/Views/{Home.cshtml,*.xaml,MainWindow.xaml.cs}`, view model and C# models (§43) | `csproj.graphify.json` | `csproj.clustered.graphify.json` |
| `misc/` | `pascal/` (`.pas .dpr .dpk .inc .dfm .lfm .lpk`), `apex/` (`.cls .trigger`), `cobol/` (`.cbl .cpy`), `byond/` (`.dmf .dmi .dmm`), `gpu/kernels.metal`, `inert/` (`.r .tf .ejs`), `scripts/` (shebangs) (§44) | `misc.graphify.json` | `misc.clustered.graphify.json` |
| `groovy/` | `build.gradle`, `settings.gradle`, `src/main/groovy/com/shop/{App,model/{Base,User},service/UserService,util/Format}.groovy`, `src/main/java/com/shop/{util/Logger,model/Auditable}.java`, `src/test/groovy/com/shop/UserServiceSpec.groovy` | `groovy.graphify.json` | `groovy.clustered.graphify.json` |
| `dart/` | `lib/shop.dart`, `lib/src/{shop_part,service}.dart`, `lib/src/models/{base,user}.dart`, `lib/src/util/format.dart`, `test/service_test.dart` | `dart.graphify.json` | `dart.clustered.graphify.json` |
| `lua/` | `lib/shop/{init,util,service}.lua`, `lib/shop/models/{base,user}.lua`, `spec/service_spec.lua`, `bin/shop` (lua shebang), `plugins/server.luau` | `lua.graphify.json` | `lua.clustered.graphify.json` |
| `sql/` | `db/migrations/{001_init,002_views,003_functions}.sql`, `db/procedures.sql`, `db/plpgsql.sql` and `db/maintenance.sql` (routines the grammar loses), `db/schema.sql` (pg_dump style), `db/mysql/legacy.sql`, `db/seed.sql` (DML only), `db/empty.sql` (0 bytes) (§45; Graphify **0.9.97** with tree-sitter-sql) | `sql.graphify.json` | `sql.clustered.graphify.json` |
| `yaml/` | `config/app.yaml` (nesting, sequences, anchors, merges, repeated and odd keys) and `config/app.py` (an id collision), `.github/workflows/ci.yml`, `.travis.yml`, `docker-compose.yml`, `deploy/k8s/{app,job}.yaml` (multi-document manifests), `api/openapi.yaml` (skipped), `config/{flat,list,empty,comment,settings}.yaml`, `config/settings.yml`, `broken/{syntax,stack,chart}.yaml` (refused by the loader), `pnpm-lock.yaml`, `config/secrets.yaml` and `credentials/db.yaml` (dropped as sensitive) (§46; Graphify **0.9.97**) | `yaml.graphify.json` | `yaml.clustered.graphify.json` |

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
{"directed":false,"multigraph":false,"graph":{"schema_version":1,"graphify_version":"0.9.97"},
 "nodes":[...],"links":[...],"hyperedges":[],"built_at_commit":"e89fff89ac9af12e8d4ce9d5fd07beb408ca730f"}
```

- `directed` is `false`, `multigraph` is `false`, and `graph` is exactly
  `{"schema_version":1,"graphify_version":"0.9.97"}` (the release; dijkstral writes the release it
  reproduces).
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
    `.yaml .yml` (since 0.9.97, §46),
    `Cargo.toml`/`pyproject.toml` (only those names, §14.2), `.sh .bash` (§14.3), and `.rb .c .h
    .cpp .cc .cxx .hpp .cu .cs .kt .kts .swift .php .scala .groovy .gradle .dart .lua .zig .ex .exs
    .erl .ml .jl .m .mm .v .sol .f90 .vb .vue .svelte .astro .ps1 .r .sql .tf`.
  - `.d.ts` and `.min.js` are ordinary code files.
  - Extensionless files with a python, sh, bash, zsh, node, ruby or perl shebang are code. Example:
    ripgrep's `benchsuite/benchsuite` becomes file node `benchsuite_benchsuite`.
- **Skipped in `--code-only`:**
  - Docs: `.md .mdx .rst .txt .html` (and `.yaml .yml` before 0.9.97).
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
  - `package-lock.json` and `pnpm-lock.yaml` are silently dropped.
  - Files that look sensitive (`.env*`, `*.pem`, data and documents named like secrets, anything
    under `.ssh`) are dropped (§14.5).
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
| `_callable` | `true` | functions, methods and class-like declarations in most languages (each language section lists its constructs); Go and Objective-C since 0.9.97. **Never on Rust, shell, JSON or YAML.** |
| `_callable_class` | `true` | the class-like subset of the above (classes, interfaces, type aliases, enums, Go types and receiver nodes…) |
| `_py_return_type`, `_py_property`, `_py_fixture` | string / `true` / string | Python defs (0.9.97, §9) |
| `_ts_fields`, `_ts_static_fields`, `_ts_public_twin` | object / object / `true` | TS/JS classes and `#private` twins (0.9.97, §10) |
| `_java_ret` | string | Java methods with a named return type (0.9.97, §13) |
| `external`, `type: "external"` | `true` / `"external"` | external import nodes |
| `type: "package"`, `ecosystem`, `version` | `cargo`/`python`/`go` | TOML and `go.mod` package nodes |
| `metadata` | object | Python `{"python_opaque_bases": true}`; `unresolved_calls` (each entry with `file` since 0.9.97) in Java, Kotlin, Go, TS/JS, PHP, C#, C++, Swift and Objective-C; C# method `arity`/`signature`; shell `{"language":"bash","kind":…}`; per-language sections give the rest. Never on Rust nodes. |
| `_rust_declaration_count` | int | every Rust struct/enum/trait node (count of same-name declarations in the file, e.g. two `#[cfg]` variants → 2) |
| `_rust_impl_key` | `"Name/n"` | Rust owner nodes created by plain-generic inherent impls (`impl<T> Pair<T>` → `Pair/1`) |
| `confidence` | `"EXTRACTED"` / `"INFERRED"` | TS/JS dynamic-import phantoms (quirk); every Verilog node is `INFERRED` (0.9.97, §36) |
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
pair (re-verified 2026-10-15 on extraction probes, see the shared-rules note below).

| Rank | Relations |
|---|---|
| 1 | `references` |
| 2 | `uses` |
| 3 | `calls`, `case_of`, `dynamic_import` |
| 3.5 / 3.6 | `references_constant` / `uses_static_prop` (PHP, §19.2) |
| 4 | `contains`, `method`, `inherits`, `implements`, `defines`, `embeds`, `rationale_for`, and every relation not listed |
| 4.1 | `instantiates` (Verilog, §36.3: beats `contains` in either order; its place against `imports` is **Open**) |
| 4.5 | `imports` |
| 5 | `imports_from`, `indirect_call`, `re_exports` |

**Orientation.** A pair's orientation is that of the **first** edge folded into it, not of the
current survivor (**Rule**, Kotlin probe: `A -contains-> Z`, then `Z -implements-> A` wins the tie
below, then `A -calls-> Z` still counts as same-orientation and lends `implements` its
`context: "call"`; a `Z -calls-> A` instead leaves no trace). A self-loop always counts as
opposite (below), so a repeated self-loop keeps the **first** (**Rule**: Zig's two `_` cases,
Objective-C's `@property Impl *a` inside `Impl`).

**Structural relations** are `contains`, `method`, `inherits`, `implements`, `defines` and
`embeds` (**Rule**: Java/C#/Kotlin/Dart/Go probes; `extends`, `mixes_in` and the rest of rank 4 are
**Open** and fold as non-structural).

**Same orientation** (`N.source` is the first edge's source):

- **If `rank(N) >= rank(E)`, N wins.**
  - When `N` is structural, it replaces `E` whole (Java: an earlier `references generic_arg`
    leaves no trace on `contains`; C#/Objective-C: a property `references` before `inherits`).
  - Otherwise `E = {**E, **N}`: `N`'s keys win, and keys only `E` had survive. Examples:
    `references(parameter_type)` + `uses` gives `uses` with `context: "parameter_type"`; an INFERRED
    0.85 `calls` followed by an EXTRACTED `calls` gives EXTRACTED with `confidence_score: 0.85`.
  - For the same relation the later edge wins: a duplicate `references` with `return_type` beats
    the earlier `parameter_type`; duplicate Rust `method` edges from two `impl` blocks give the later
    line.
- **Otherwise, E wins.**
  - When `E` is structural, nothing is merged, except that a folded `calls` edge gives `E` its own
    `context` when `E` has none: `"call"` for most languages (the 935 `contains` links with context
    `call`), none for a Zig call, which has none (quirk ZG3 is this rule). A folded `references`
    edge leaves no trace (Go `store_store_go_store_store → store_id` keeps `contains` with no
    context; Dart `file -defines-> X` beside `file -references-> X`, 88 dart-http links; C#
    `class B : A { A a; }`; PHP `extends` + property; Go `embeds` + field).
  - Otherwise `E = {**N, **E}`.

**Opposite orientation:**

- There is **no key merging**. One edge survives whole, with its own direction.
- If the ranks differ, the higher-ranked edge survives.
- If the ranks are equal and the relations are the **same**, the edge whose **source id is
  lexicographically smaller** survives, regardless of extraction order (probe `rev1`:
  `x_alpha→x_zeta` and `x_beta→x_gamma` both survive; Python `class Aa(Bb)` / `class Bb(Aa)` and
  Java `implements` cycles keep `Aa→Bb` in either order; Julia's `file -defines-> module` beats
  `module -defines-> file`, which was quirk JL1).
- If the ranks are equal and the relations **differ**, the **larger** source survives (**Rule**): a
  nested class that inherits its outer class keeps `contains` when its id sorts first
  (`f_result -contains-> f_ok`, Java `r_r -contains-> r_o`) and `inherits` otherwise (okhttp
  `…connectionevent_connectstart -inherits-> …connectionevent_connectionevent`; Java
  `interface Zz { class Aa implements Zz {} }` keeps `contains`). The same rule explains the
  package import cycles below, since `re_exports` ranks with `imports_from`.

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

**Import cycle through a re-exporting package (Rule, formerly a quirk).** A package
`__init__.py` that does `from .m import X` (`imports_from` + `re_exports` to `m`) while `m` does
`from . import Y` (`imports_from` back to the package) keeps the edge whose source is
lexicographically **larger**: `re_exports` and `imports_from` share a rank and differ, so the
opposite-orientation tie above applies.

| Probe | Edges | Survivor |
|---|---|---|
| `tri_a` | `pkg_init→pkg_a` (imports_from, re_exports); `pkg_a→pkg_init` (imports_from) | `pkg_init→pkg_a` `re_exports` |
| `tri_z` | `pkg_init→pkg_z` (imports_from, re_exports); `pkg_z→pkg_init` (imports_from) | `pkg_z→pkg_init` `imports_from` |

It occurs in 5 of 5 cases (4 in fastapi, 1 in the fixture: `pkg_service -imports_from-> pkg_init`).

**Still open.** Merges after the fold (C# partial types, quirk CS1; Swift extensions, quirk S4;
stubs resolved to a definition after the fold: Java `class Aa { Bb b; } class Bb extends Aa {}`
keeps `inherits` **with** `context: "field"`, the same with `Zz` in place of `Aa` without) follow
dict-update semantics this model does not cover; Fortran's `defines` against reversed `calls`
(§35.6) is folded by its extractor. Kubernetes keeps `context` (`return_type`,
`parameter_type`) on 2,513 Go `method` links from a reversed reference, which no small Go probe
reproduces.

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

**Fields added in 0.9.97** (probes `web-py1` to `web-py6`, `web-py19`, `web-py20`):

- `_py_return_type` on any function or method whose return annotation names a class: a plain or
  dotted name (last component), a string holding a dotted name (`"Box"`, `'pkg.sub.Thing'`),
  `Optional[X]` (any qualifier, nested) or `X | None`, unwrapped to `X`. The name must start with
  an upper-case letter (`int`, `list`, `node_t`, `_Priv` and `None` give nothing; `Any`, `Self`,
  `Callable` count); other subscripts and unions give nothing.
- `_py_property: true` on a def decorated (uncalled) with `property`, `cached_property`,
  `functools.cached_property` or `<x>.getter` (not `@x.setter`, `@abstractproperty`,
  `@builtins.property`, `@property()`).
- `_py_fixture` on a def decorated with `pytest.fixture`, `fixture` or `pytest_asyncio.fixture`
  (called or not) in a test module (`test_*.py`, `*_test.py`, `conftest.py`): the class it
  provides, else `"?"`. A return annotation decides alone: a plain or dotted capitalised name,
  or the first argument of `Iterator[…]`, `Generator[…]` and their async forms. Without one,
  every `return x` / `yield x` of the def's own body (nested defs and lambdas excluded, bare
  `return`/`yield` ignored, `yield from` unknown) must agree on a class: `Cls(...)` /
  `mod.Cls(...)` with a capitalised name, or a local whose every binding does (`n = Cls()`,
  `n: Cls = …`, `n: Cls`, `with Cls(...) as n`; loop targets and other bindings are unknown).

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
| `references` | decorated def → decorator | `@traced`, `@deco` (imported), `@ext_deco(arg=1)` → stub, `@pytest.fixture` → stub `fixture`. Builtins (`@property`, `@staticmethod`, `@classmethod`) and `@dataclass` produce nothing, nor (0.9.97) the accessor forms `@x.getter`/`@x.setter`/`@x.deleter` (a bare `@setter` still does). | EXTRACTED | `decorator` | **decorator line** |
| `references` | reader → `_py_property` method (0.9.97) | an attribute read (not a call, not an assignment target; `+=` counts; not inside a lambda): `self.p` through the class and its bases (never the property itself), or `x.p` with `x` typed as for `x.m()` (§9.4) to a property defined directly on that class. Not `cls.p`, `self.a.p`, `Cls.p` (probes `web-py17`, `web-py18`). | EXTRACTED 1.0 (`self`) / INFERRED 0.85 | `property_read` | the **last** read |
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
| `mod.f()` on a **local** module alias (`models.validate(...)`, `sh.make()`, `import pkg.models as pm; pm.validate()`) | EXTRACTED 1.0, from functions and (0.9.97) methods; a fully dotted `pkg.lib.shapes.make()` is dropped (probe `web-py14`) |
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

`Self` (any qualifier) is excluded from `references` like `Optional` (0.9.97; it still gives
`_py_return_type: "Self"`).

### 9.6 Duplicate names

- `helper()` exists in `models.py`, `service.py` and `util.py`.
- Calls resolve by scope:
  - the local `helper` is chosen for `key=helper` in `service.py`;
  - `util.py`'s own `helper` is used by `slugify`;
  - none of them is ever reached through the unique-name fallback.
- Two `.save()` methods (`Base`, `User`) are separate nodes. `user.save()` on `U(...)` is dropped.

**Id clashes (0.9.97, probes `web-py9`, `web-py10`, `web-py16`).** Module-level functions and
classes, and the methods written directly in a module-level class, decorated or not, keep apart
when their different names produce one id: a name not already in id form (`P`, `Get_Y`, `_m`)
or with a same-letters, different-case twin takes `_<sha1(name)[:6]>` (`class P` + `def p` →
`a_p_511993` and `a_p_516b97`; `class Foo` + `class foo` both suffixed). 0.9.86 did this only
for undecorated functions and methods. Nested classes, their methods and nested functions still
merge into the first definition, as do identical names. **Quirk (not reproduced):** a suffixed
class or decorated function loses the edges Graphify keys by the unsuffixed id: its decorator
`references` (the stub stays), its docstring's `rationale_for` and, for decorated functions,
in-body `calls`; dijkstral emits them.

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
| `require` of a **package** (or any non-relative or non-literal specifier), destructured or not | `file_id_slug(pattern text)` | `const { join } = require("path")` → `src_util_join`; `const lodash = require("lodash")` → `…_lodash`. A relative `require("./m")` (string starting with `.`) makes **no** node in 0.9.97 (`src_util_legacy` is gone) |
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
- **0.9.97:** `_ts_fields` / `_ts_static_fields` on a class (TS and JS; not interfaces) when any
  instance / static field has a type, as an object sorted by key (probes `web-ts1`, `web-ts5`
  to `web-ts7`):
  - declared: a field annotated with a plain type name, or `T | null | undefined` (`a: Node`,
    `#p: Node`, `m?: Node`, `q: node_t`; not `string`, generics, arrays, `ns.T`, function
    types), else, without an annotation, `= new X()` with a bare `X`; parameter properties
    (`private pa: Node`); `get x(): T` and `set x(v: T)`; `accessor a: T`; function-valued
    fields with an annotation (`handler: EventListener = () => …`). Declarations win.
  - assigned, for fields without a declared type: `this.x = new X()` or `this.x = p` (`p` a
    typed parameter of that method) in a method's own body (arrow callbacks count; `function`
    expressions, nested functions and function-valued fields do not; other right-hand sides
    say nothing). Constructor assignments rank first, then the other methods'; a tier whose
    assignments disagree types nothing. `this.x` in a static method types a static field.
- **0.9.97:** `metadata.unresolved_calls` on a function, method or (for calls in module-level
  callbacks) the file: `[{callee, receiver_type, lang: "typescript", line, file}]`, once per
  (callee, type) in source order, for `x.m()` / `this.f.m()` whose receiver type (as typed for
  this purpose, §10.4) is not a TS/JS module-level name of the repository other than a function
  (classes, interfaces, type aliases, enums, namespaces, values; case-insensitive), not a name
  destructured from a `require`, and not a standard global (`Map Set WeakMap WeakSet Array
  Promise Date RegExp Error TypeError RangeError SyntaxError ReferenceError EvalError URIError
  Object String Number Boolean Symbol BigInt JSON Math Intl URL URLSearchParams Headers Request
  Response Proxy Reflect FormData Blob File AbortController AbortSignal TextEncoder TextDecoder`;
  probes `web-ts8` to `web-ts23`). `lang` is `typescript` for JS too.

**Same-file id collisions (quirk).**

- Matching labels (`function dup` twice; `get x` with `set x`): the first node wins.
- Differing labels (`function foo` with `class Foo`): the merged node takes the later node's bare
  name and line and loses `_callable`/`_callable_class`. Calls to the shadowed definition disappear.
- **Rule (0.9.97), class and interface members only:** members whose different names produce one
  id keep apart. When exactly one of the names is public (not `_`- or `#`-prefixed) it keeps the
  id; every other member takes `_<sha1(raw name)[:6]>` (`m`/`M` → both suffixed, `_priv`/`priv`
  → `priv` plain, `#y`/`y`/`_y` → `y` plain, `['comp']`/`comp` → both). `#n` beside `n` marks
  both nodes `_ts_public_twin: true` (probes `web-ts17` to `web-ts19`).

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
| `const x = require("./m")` | `imports_from` (no value node for a relative specifier, §10.1) |
| `const { a, b } = require("./m")` | `imports_from` + `imports` → `m_a`, `m_b` (stubs if missing) |
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

**Barrels:** 0.9.97 no longer leaves the 0.9.86 stub (`src_models_index_user`) beside the real
`imports` edge (`src_service -imports-> src_models_user_user`). When two `export *` sources define
the same name, the first wins. A method still cannot see a barrel-imported class (`new User()` in
`register` is dropped, §10.4).

**Dynamic-import phantoms (quirk):** an `import()` target that is not a local code file becomes a
`file_type: "code"` node with `confidence: "EXTRACTED"` and no `source_location`.

- A bare specifier's phantom is its package (0.9.97): id `slug(package)`, `label` and
  `source_file` the package name, `type: "module"` (`@scope/pkg-types/x` → `scope_pkg_types`,
  `lodash/fp` → `lodash`). 0.9.86 used `slug(last path segment)` and the whole specifier
  (`pkg_types`, `fp`). A relative specifier keeps `slug(resolved path)` and the specifier as
  label, without `type`.
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
- `this.m()` resolves to the enclosing class's `m`; `Class.m()` on a same-file class to its `m`.
- 0.9.86's quirk binding `anything.m()` to the last-defined same-file method named `m` (the
  `src_service_userservice_log` self-loop from `console.log`) is gone in 0.9.97, and so is
  `const me = this; me.m()` (probe `web-js3`; dijkstral keeps binding the alias, a deliberate
  difference worth ~170 erpnext links).
- **0.9.97:** a typed receiver in the same file (`k.m()` with `k: K`, `const k = new K()`, or
  `this.f.m()` with `f` in `_ts_fields`) gives `calls` INFERRED 0.85, never to the caller itself
  (`k.n()` inside `n` gives nothing; `this.m()` inside `m` is still an EXTRACTED self-loop).
  Receivers are typed **file-wide by name, the last binding winning**: every parameter annotated
  with a plain type name (any function, callback, overload, method signature or function type)
  and every `x = new X()` declarator (probes `web-ts13`, `web-ts14`). Unions, generics, rest and
  destructured parameters bind nothing; annotated locals (`let j: K = make()`) do not count.
  Dijkstral types a receiver by its own function's parameters and locals first (a deliberate
  difference), but records `unresolved_calls` with Graphify's typing.

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
  resolved. In 0.9.97 this includes classes bound by an unaliased destructured `require`
  (`const { Store } = require("./store")`: `Store.create()` EXTRACTED 1.0, `new Store().add()`
  INFERRED) and `this.f.m()` typed by `_ts_fields` in JS (`this.store = new CachedStore()`).
- Dropped:
  - namespace-qualified calls (`ns.helper()`, `Storage.key()`);
  - calls on external modules (no `calls` edge to `ref_*`, unlike Python);
  - calls through aliased destructured requires (`{ Store: S2 }`; dijkstral resolves them);
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
| `type` alias at module level, inline modules included (0.9.97; generic `type R<T> = …` too) | `F_slug(name)` (`src_lib_registry`) | `Name` | `_rust_declaration_count` (two `#[cfg]` aliases → `2`, first line) | `contains` from `F`; no references from its right-hand side |
| `union`, trait/impl associated `type`, `type` inside a fn body | **not emitted** | | | |
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

**Case collisions (0.9.97).** File-level items whose ids collide although their names differ
(only in case) keep the plain id when they are a type (struct, enum, trait, alias) and take
`F_slug(name)_<sha1(name)[:6]>` when they are a function, constant or static: `struct Logger` +
`const LOGGER` → `src_lib_logger` and `src_lib_logger_d3a35e`; `static MSG` + `fn msg()` → both
suffixed; `fn e()` + `fn E()` → `src_lib_e_58e6b3`, `src_lib_e_e0184a` (ripgrep:
`crates_core_messages_messages_17f346`/`…_85cda8`). Equal names are not suffixed (below), and an
enum variant and a method of the enum (`B`/`.b()`) still share one node.

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
3. **Resolution (0.9.97):** a stub whose name the file imports resolves through that import
   (§11.3) or not at all: `use std::fs::File` keeps the stub `file` although the repository
   declares one `File`, and so does ripgrep's `use grep::printer::ColorSpecs` (another crate).
   Any other stub (globs count as not importing) binds by label: the only declared
   struct/enum/trait/alias of that label (with 0 or ≥2 candidates the stub stays: `Match` in
   ripgrep has 5 declarations); a global stub binds EXTRACTED, a file-scoped one **INFERRED,
   score 0.55** (`src_util_mod_render -references-> src_model_circle` through `use super::model::*`;
   0.9.86 bound every stub EXTRACTED and ignored imports). Candidates are restricted by language:
   a Rust stub does not resolve to a Python class in a `.py` file, but *does* resolve to one in an
   extensionless script (`benchsuite/benchsuite`). *Quirk.*
4. **Import rewiring.** Import edges point at the stub id of their leaf (§11.3); when that id is
   redirected — a local leaf resolved, or a global type stub of that id resolved — every file's
   edge to it follows, whatever its path: `use shapes::fromroot` in `src/main.rs` lands on
   `src_model_fromroot` because `src/lib.rs` has `pub use crate::model::fromroot`, while
   `use shapes::model::round_kind` stays external when no local import names `round_kind`.
   *Quirk: name coincidence, not module resolution.*

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
like `use`. One edge per imported leaf; `use` inside a fn body, `extern crate x [as y];` and
`mod x;` give nothing. The edge first targets the leaf's **stub id**:

| form | stub id(s) |
|---|---|
| `use std::borrow::Cow;` | `cow` |
| `use std::collections::{HashMap, HashSet};` | `hashmap`, `hashset` |
| `use std::{ffi::OsStr, io::{self, Read}, path::Path};` | `osstr`, `io` (`self` → parent segment), `read`, `path` |
| `use self::fmt::render as render_shape;`, `use a::b::{self as bee};` | `render_as_render_shape`, `b_as_bee` |
| `use x::y::Z as _;` | `z_as` (trailing `_` trimmed) |
| `use super::model::*;` / `use super::*;` | `model` / `super` (segment before `*`) |
| `use crate::model;`, `use ::absolute::Thing;`, `use Single;` | `model`, `thing`, `single` |

(rule 1 of §11.2: a type stub of the same id in the file takes the edge). **0.9.97 then resolves
local leaves**: a path starting with `crate`, `self`, `super` or a module the file declares
(`model::Kind` in `lib.rs`; not `util::h1` from a `main.rs` that declares no `mod util`, not the
package's own crate name `shapes::…`) names the **definition** whose name is its last segment —
a function, struct, enum, trait, alias, const, static, macro (`src_model_macro_sq`) or enum
variant (`Kind::A` → `src_model_kind_a`); `Circle as Round` names `Circle`. A module leaf or a
glob names the module's file when it is a `name.rs` (`use crate::util::fmt` → `src_util_fmt`,
`use super::model::*` → `src_model`) but not a directory's `mod.rs` (`use crate::util` stays
`util`); methods (`Circle::new`) and inline modules stay external. Graphify finds the definition
**by name alone**: the path is not checked (`use crate::util::Circle` finds `model.rs`'s
`Circle`) and a name defined twice resolves nothing (`render`, `dup`, `Kind` → externals).
The stub id is redirected, so other files' edges to it follow (§11.2 rule 4).

dijkstral resolves the same local leaves through its module tree (`resolve/rust/modules.rs`):
identical where the name is unique, and the defining item where Graphify's name lookup gives up
or guesses (quirk **R1**; ripgrep: `use grep_regex::RegexMatcher` is Graphify's
`searcher/src/testutil.rs` `RegexMatcher` through rule 4). Leaves Graphify leaves unresolved
but the module tree places in the repository (crate names, workspace crates, `mod.rs`
modules, `use` in fn bodies) keep a file → file edge to the defining file (Q7 not reproduced).
ripgrep `.rs` against 0.9.97: `imports_from` 692 shared, 174 ours only, 119 Graphify's only.

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

**dijkstral (Q23 not reproduced).** Calls Graphify leaves unresolved, or resolves only by name,
go through the module tree that also resolves `use` paths (crates named by their manifests:
`[lib] name`, `package =` renames, `workspace = true`, `pub extern crate … as` and `pub use`
facades; see `crate::resolve::rust`), as EXTRACTED edges with score 1.0: path-qualified calls
(`grep::cli::resolve_binary(..)`, `crate::flags::parse::lookup(..)`, `Type::new(..)` for a
struct or enum declared in another file or named by a `type` alias, `Enum::Variant(..)` as a
constructor reference; inline modules count as part of their file), bare
calls through `use … as` aliases and imported denylisted names, and `self.a.b.m(..)` through the
fields' declared types (`should_preprocess` → `Override::matched` in ripgrep). A bare name
imported from outside the repository no longer falls back to a same-named repository function.
Edges start only from a caller whose node the build keeps for its id (not a `#[cfg]` twin, a
same-named method of another impl or a fn sharing a struct's id). Trait-qualified calls
(`Trait::m(x)`) and calls inside macro arguments stay unresolved. Fixture: `build →
Circle::new`, `main → build`, `main → Circle::new` (also through `Round::new`), `main →
classify`, `main → run_all`, `util::render → fmt::render` (alias `render_shape`).

**Local type inference (dijkstral, Q23).** `recv.m(..)` on a local, a parameter or any
expression whose type a declaration fixes resolves to the unique method `m` over every impl of
that type (inherent and trait impls alike; two candidates resolve nothing), EXTRACTED 1.0, beside
whatever the in-file name rule chose. A receiver's type comes from `let x: T`, a parameter
`x: &T`, a struct literal `T { .. }`, `self`, the declared return type of the function or method
a call reaches (`T::new()`, `f()`, `a.b().c()`, builder chains returning `&mut Self`; tuple
structs and variants construct their type; `T::default()`, `T::from(..)`, `.clone()`), a field's
declared type, `?`, `.unwrap()` / `.expect()` and `Some(x)` / `Ok(x)` patterns on an `Option<T>` or
`Result<T, E>` written with its `T`, `.await` on an `async fn`, and `Box`/`Arc`/`Rc::new(x)` (whose
own `clone`, `as_ref`… come first). Scopes follow blocks: a later `let`, closure parameter,
`for`/`match`/`if let` pattern or nested item shadows a name, and anything not fixed by a
declaration is unknown, so trait objects, generic parameters, `impl Trait`, closures, iterator
items and patterns resolve nothing. Rust variables keep their type on assignment, so only
shadowing invalidates.

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

1. Import resolution is by unique definition name and stub-id rewiring (§11.3, rule 4), not
   module resolution (R1: a name defined twice resolves nothing, a wrong path still resolves).
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
Goldens: `go.graphify.json` (raw `--no-cluster`), `go.clustered.graphify.json`, from Graphify
0.9.97. Rules below were isolated with extra probes; the probe snippets are quoted inline. Where
0.9.97 differs from 0.9.86 the text says so (2026-10-29: kubernetes and probes run through both).

Notation: `slug` as in §4.1. `dir` = the **last path component of the directory containing the file**; for a file at
the scan root it is the **basename of the scan root directory itself** (probe: `root.go` with
`type R` scanned from `.../p1` gave `p1_r`). `fileid` = the file node id (`store/helpers.go` →
`store_helpers`).

### 12.1 Nodes

| Construct | id | label | source_location | extra fields |
|---|---|---|---|---|
| file | `fileid` | basename (`helpers.go`) | `L1` | — |
| `type X struct/interface/<named>`, alias `type X = Y` (generic ones too, grouped ones too) | `slug(dir)_slug(X)` → `store_id`, `service_user`, `store_entity`, `store_key` | `X` | line of the type spec | `_callable`, `_callable_class` |
| func `F` | `fileid_slug(F)` → `store_helpers_normalize`, `service_service_newuserservice` | `F()` | line of `func` | `_callable` |
| method `func (r T) M()` / `(r *T)`, generic `func (s *Store[T]) Put` | `<id of T>_slug(M)` → `service_userservice_register`, `store_model_identity`, `store_store_put` (the receiver's type arguments are dropped) | `.M()` | line of `func` | `_callable` |
| interface method `M()` | `<interface id>_slug(M)` → `service_greeter_greet` | `.M()` | line of the method spec | — |
| receiver type with no `type T` earlier in the file (quirk G3) | `slug(dir)_slug(T)` | `T` (no type arguments) | line of the first such method | `_callable`, `_callable_class` |
| qualified external type `http.Request`, `testing.T` | `go_type_` + `slug(import path)` + `_` + `slug(Name)` → `go_type_net_http_request`, `go_type_testing_t` | `net/http.Request` (import path + `.` + Name) | `""` | `source_file: ""`, `file_type: "code"`, `_origin: "ast"` |
| external import | `go_pkg_` + `slug(import path)` → `go_pkg_fmt`, `go_pkg_github_com_google_uuid` | **same as id** (quirk G6) | absent | `file_type: "concept"`, `type: "external"`, `external: true`, `source_file: ""`, no `_origin` |
| `go.mod` | `pkg_` + `slug(module path)` → `pkg_example_com_shop` | module path `example.com/shop` | `L1` | `type: "package"`, `ecosystem: "go"`, `source_file: "go.mod"`; no edges; `require` lines produce nothing; stderr warns "yielded no symbols ... go.mod" |

Raw key order: `id, label, file_type, [type, ecosystem,] source_file, source_location, _origin`.
External: `id, label, file_type, type, external, source_file`.

`_callable` and `_callable_class` are new in 0.9.97 (0.9.86 gave Go nodes neither). A type whose
id a qualified stub of its own file already holds (`ktesting.TContext` in dir `ktesting`) leaves
the stub's label in place but gives it both flags. `metadata` only carries
`unresolved_calls` (§12.2). No `node_kind`.

An alias is a node with its `contains` edge and nothing else: no `references` from its right-hand
side, no interface methods for `type F = interface{ M() }` (probe `go-alias`, 0.9.86 emitted no
alias at all).

**Case variants.** Names of one file whose ids coincide (`Set`/`set`, `STM`/`stm`, `IntervalTree`
/`intervalTree`, `run`/`run_`) are told apart by `_` + `sha1(name)[:6]`, except that the group's
only exported name (capital first letter) keeps the plain id: `concurrency_stm` (`STM`) and
`concurrency_stm_a92c15` (`stm`); a suffixed type's methods hang off its suffixed id
(`adt_intervaltree_5f7fc6_find`). This held for all 2,996 names in such groups on kubernetes.
0.9.86 suffixed only functions and methods, and folded a type's case variants into one node.

Not emitted at all (verified): `const`, `var` (incl. `var ErrNotFound`), closures / func literals
(`run := func(){}`), struct fields, type parameters, and **any comment** — Go produces **no
rationale nodes** (package doc, `// NOTE:`, `// WHY:`, `// TODO:`, `// HACK:`, `/* WHY: */` all
ignored; probe `p6`). Grouped `type ( G1 struct{} )` is emitted normally.

#### Type ids held by several files (0.9.97)

Types are keyed by directory, so build-tag variants (`ztypes_linux_386.go` and
`ztypes_aix_ppc.go` each declaring `Rusage`), case variants in different files
(`ConfigProvider`/`configProvider`) and receiver nodes in files without the declaration share one
id. Before the §4.5 rename (probes `go-xfile*`, `go-xdir`, `go-x9-*`; 2,996 kubernetes groups):

- When every declaration and receiver node of the id (a stub-held declaration included) lies in
  **one directory and in files of one kind** (all `_test.go`, or none), they keep **one node**:
  the first file in path order that declares the type, else the first receiver node
  (`admission_configprovider` labelled `configProvider` from `config.go`; `netlink_ifreq` labelled
  `Ifreq`). The other files' nodes disappear; their `contains`, `method` and other edges stay on
  the kept id.
- Otherwise nothing merges: a receiver in a `_test.go` file beside a non-test declaration
  (`realImageGCManager`), declarations in two directories with the same last component
  (`cmd/kube-proxy/app` and `test/e2e/dra/test-driver/app` declaring `Options`) and a non-test
  declaration plus a test one all go to §4.5, which renames every holder per file, receiver nodes
  included.
- After a merge, other emitters of the id (a same-named file node, a qualified stub of another
  package, another language) still rename the kept node (§4.5): `a_x_go_a_util` beside `util.py`
  and `util.ts`. A dropped file's edges keep the plain id and then dangle (`a/y.go`'s
  `method` edge from `Util`).

0.9.86 renamed every holder in every case.

#### Cross-package type re-keying (quirk G1, unchanged in 0.9.97)

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
| `references` | struct → field type | named field type resolvable to a local type or a `go_type_*` stub | EXTRACTED | `field` | field line |
| `references` | func/method → type | parameter / return / generic argument | EXTRACTED | `parameter_type` / `return_type` / `generic_arg` | the func line |

A type parameter is no type: `[T any]`, a receiver's `[T]` and a constraint's parameters produce no
stub and no edge, even where the parameter shadows a declared type (`func F[ID any](x ID) ID`).
0.9.86 made global stubs `t`, `n` for them.

Stubs bind by label over **every** definition of the graph, not only Go's: a bare `T` binds to
the unique node labelled `T` (types, then functions), and a qualified `u.T` to the unique type `T`
of `u`'s directory, but a Python class or a YAML key labelled `T` (anywhere for the bare form, in
`u`'s directory for the qualified one) leaves the stub unbound (probes `go-blabel*`, `go-qlabel`;
`pkg/kubelet/status/.mockery.yaml` lists `PodStatusProvider`, so `status.PodStatusProvider`
becomes `go_type_k8s_io_kubernetes_pkg_kubelet_status_podstatusprovider`).
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
5. **Typed receivers (0.9.97).** `x.M(..)` that rule 2 binds to nothing, or only to the caller
   itself (`func (o *Outer) Encode() { o.in.Encode() }`), with `x` typed `T` or `*T` (a plain name,
   no qualifier, no type arguments) by a declaration: the caller's own receiver, parameters,
   named results, `var x T`, `x := T{..}` / `&T{..}` / `new(T)`, else the **first** such
   declaration of the name anywhere in the file (another function's parameter, a struct field,
   a package `var`). Inside a method, `r.f.M(..)` on the receiver `r` types `f` from the file's
   declarations alone (struct fields included; first wins, whatever type `r` has). `T` then
   selects the unique Go type or receiver node labelled `T` (any directory) and its method `M`
   through the `method` edges (interface methods too): `calls` INFERRED 0.85. A type that is
   known but ambiguous or lacks `M` gives nothing (promoted methods are not followed); a `T` no
   Go type carries (an undeclared or function-local type, a type parameter, a case variant
   merged away) is recorded in the caller's `metadata.unresolved_calls` as
   `{"callee","receiver_type","lang":"go","line","file"}`, once per callee and type, in call
   order. Builtin types (`err error`) and call results (`x := NewT()`) type nothing (probes
   `go-recv*`, `go-field*`, `go-deleg`).
6. **Dropped**: method calls a declaration does not type (`svc := NewUserService();
   svc.Register()`, `s.users.Put` outside methods), calls through fields/vars/closures
   (`s.log(...)`, `cb()`, `run()`), unknown qualified names (`ml.Missing()`), all external package
   calls (`fmt.Println`, `strings.ToLower`, `sort.Slice`, `uuidlib.New`) — Go never emits `calls`
   to `go_pkg_*`, composite literals `&T{}` / `T{}` (no constructor edge), `go f()` / `defer f()`
   count as plain calls.
7. One `calls` edge per (caller, callee) pair; the first call site's line is kept.

**dijkstral (Q24 not reproduced).** A method call whose receiver's type a declaration fixes
resolves to that type's method wherever the package declares it, EXTRACTED with score 1.0, beside
whatever the same-file name rule (G5) chose and only for pairs rule 5 did not already link (those
keep Graphify's INFERRED 0.85). The receiver's type comes from the method receiver,
parameters and named results, `var x T`, `x := T{..}` / `&T{..}` / `new(T)` / `v.(T)`, a
type switch case with a single type, the declared results of the function or method a call
reaches (`x, err := NewT()`, `a.B().C()`, conversions `T(x)`, explicitly instantiated
`pkg.F[T]()`), a field's declared type and package-level variables (`pkg.Var.M()`). Methods and
fields promote from embedded fields at the shallowest depth that has the name, and only when
every type at that depth is a struct of the repository. Scopes follow blocks (`if`/`for`/
`switch` headers included); `:=` redeclares only in a new scope, and range variables, receive
statements and closure parameters without a named type shadow as unknown. Interfaces, type
parameters, slices, maps, values typed by the standard library (`t *testing.T`) and anything
else a declaration does not fix resolve nothing; edges start only from a caller whose definition
holds its id. `pkg.F[T](..)` is a call of `F`, and
`pkg.F(..)` binds to the package's unique function `F` even when methods share the name (a
method is never `pkg.M`). Fixture: `main → UserService.Register` (`svc :=
NewUserService()`), `TestRegister → UserService.Register`, `Register → Store[T].Put` (field
`s.users`), `Register → Model.Identity` (promoted), `NewUserService → NewStore`
(`store.NewStore[User]()`).

### 12.3 Test files

`*_test.go` are ordinary code files: `service_service_test` / `service_service_test_testregister`,
`testing.T` param → `go_type_testing_t` stub. Nothing marks them as tests.

### 12.4 Quirks (do not reproduce unless chasing byte parity)

- **G1** cross-package type re-keying to `<slug(def file)>_<dir>_<name>`; method ids not re-keyed.
- **G2** id collisions merge file and type nodes (file node lost, self-loop `contains`), and func vs method.
- **G3** a receiver type not declared earlier in the file gets a node at the first method's line
  (`func (m Mixed) Run()` before `type Mixed`), with no `contains` from the file; 0.9.97 merges it
  with the package's declaration as above. 0.9.86 also kept generic receivers apart
  (`store_store_t`, label `Store[T]`).
- **G4** package imports link to `_test.go` files.
- **G5** method calls resolved by bare name, last definition in the file wins.
- **G6** external package nodes use the id as label (`go_pkg_fmt`), unlike Python (`json`).
- **G7** (0.9.97) typed receiver calls type a variable by the first declaration of its name in
  the whole file, and a receiver's field by name alone (`func (x *Box) M() { x.inner.Do() }`
  follows whichever `inner` the file declares first). Reproduced: Graphify's edges are its
  output; dijkstral's own inference (Q24) adds the sound targets beside them.
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
| method, constructor, interface/abstract method (with or without body), annotation element | `<type id>_slug(name)`: `…model_user_user_getname`; constructor `…model_user_user_user` | `.name()`; constructor `.User()` | `_callable: true`; optional `_java_ret`, `metadata.unresolved_calls` |
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

`_java_ret` (0.9.97): the simple name of a method's declared return type — `a.B` → `B`,
`Box<B>` → `Box`, `Map.Entry<String, B>` → `Entry`, an unresolved `Missing` too — on
`…userservice_register` (`User`), `…user_user_of` (`User`), `…model_model_status` (`Status`).
Absent for constructors, `void`, primitives, arrays (`B[]`, `Box<A>[]`), type parameters of the
class or method (`T`, `<U> U`, `<T extends A> T`) and the builtin type names below. Overloads
merged into one id keep it only when every overload names the same type (`A same(int)` +
`A same(String)` → `A`; `A`/`B`, `String`/`A` or `A`/`void` → none). Annotation elements
(`@interface Ann { Kind kind(); }`) are not nodes. Probe `jvm-j1`–`j5`.

Builtin type names (one list decides references, stubs and `_java_ret`; probed name by name,
179 names, the module's `COMMON_JDK_TYPES`): the boxed primitives and `Object`, `String`,
`Number`, `Void`, `Record`, `Enum`, `Class`, `Package`, `Math`, `System`, `Runtime`, `Process`,
`ProcessBuilder`, `Thread`, `ThreadLocal`, `Throwable`, `Exception`, `Error` and the common
`java.lang` exceptions; the `java.util` collections, `Arrays`, `Collections`, `Objects`,
`Optional*`, `UUID`, `Date`, `Calendar`, `Locale`, `Random`, `Scanner`, `StringJoiner`,
`StringTokenizer`; `java.util.function` (`Function`, `BiFunction`, `Consumer`, `Predicate`,
`Supplier`, …); `Stream`, `IntStream`, `LongStream`, `DoubleStream`, `Collector(s)`; the
common `java.util.concurrent` types; `File`, `Files`, `Path(s)`, readers, writers and streams
of `java.io`; `java.time` (`Instant`, `Duration`, `LocalDate(Time)`, `ZonedDateTime`, …);
`BigDecimal`, `BigInteger`. Still referenced and stubbed: `Pattern`, `Connection`,
`ReentrantLock`, `ConcurrentLinkedQueue`, `Charset`, `URI`, `URL`, `Lock`, `Condition`,
`Phaser`, `ForkJoinPool`, `TimeoutException`, `ExecutionException`, `Flushable`, reflection
types (`Field`, `Method`, `Constructor`, `ClassLoader`), `Logger`.

`metadata.unresolved_calls` (raw: after `source_location`, before `_callable`): a list of
`{"callee", "receiver_type", "lang": "java", "line": "L<n>", "file": <source_file>}` (`file`
new in 0.9.97) in source order, one per `Type.method(...)` call whose receiver is a
capitalised identifier that is not a class of the repository: `Objects.requireNonNull` and
`Instant.now` on `…model_model_model_model`, `Collections.sort` on `…userservice_names`,
`Math.max` in probe. A class-like node of another language counts as a class: a Kotlin class,
object or anonymous object (`object : KtAnon()`), a Scala class (probe `jvm-x1`; on okhttp,
receivers typed `Response`, `OkHttpClient`, `HttpUrl` are Kotlin classes, 256 entries).
Not recorded: `System.out.println` (field receiver), lowercase receivers, ambiguous local types
(`Formatter.shout`, see below).

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
N package_dependencies_react  label "react@^18.3.1"  L5   contains from package_dependencies
N ref_react               label "react"  file_type "concept"  source_file "package.json" L5
E package -imports-> ref_react   EXTRACTED ctx "import"  L5
E package_dependencies_react -declares-> ref_react   EXTRACTED  L5
```

- Every object key becomes a node (file_type `code`, `_origin` `ast`, location = line of the key),
  recursively through nested objects; array elements are never descended.
- Key node id: depth 1 → `<fileid>_<slug(key)>`; depth ≥ 2 → `<fileid>_<slug(parentkey)>_<slug(key)>`
  — **only the immediate parent key**, not the full path (quirk): `deep.a.b.c.d` gives
  `package_deep`, `package_deep_a`, `package_a_b`, `package_b_c`, `package_c_d`.
- Keys whose slug is empty are dropped (`"@/*"` under `paths`).
- Edges: `contains` parent→child (no context, EXTRACTED).
- Dependency keys inside `dependencies|devDependencies|peerDependencies|optionalDependencies`
  whose value is a **non-empty string** are labelled `name@version` (`react@^18.3.1`,
  `local@file:../local`, `ws@workspace:*`; 0.9.86 used the bare name) and additionally create a
  concept node `ref_<slug(name)>`, an edge `file -imports-> ref_*` (context `import`) and an edge
  `<key node> -declares-> ref_*` (no context, the key's line; new in 0.9.97). `"@types/node"` →
  `ref_types_node`, `"lodash.merge"` → `ref_lodash_merge`. Non-string values (`{"a": 1}`, `5`)
  and `""` keep the bare label and get no ref (probe `web-pkg1`).
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
and also `.astro .psm1 .psd1 .cls .vbproj` (§38–§42), and since 0.9.97 `.yaml .yml` (§46).
`.d.ts` and `.min.js` are code (not filtered).

- `.r`, `.tf` count as code but contribute nothing, not even a file node (stderr: optional
  grammar missing), and so does `.sql` without Graphify's SQL extra (§45). `.ejs` gets its file node
  (§44.7). The project, markup and grammar-less
  formats `.csproj .fsproj .vbproj .sln .slnx .razor .cshtml .xaml .pas .dpr .pp .inc .lpr .dpk
  .dfm .lfm .lpk .cls .trigger .cob .cbl .cpy .dmf .dmi .dmm .metal` are code too (§43, §44; the
  final per-extension and shebang check is §44.8).
- Some code files "yield no symbols" (warning lists them): e.g. `fun kt() {}` in `.kt` (the
  function's id `<file>_kt` is the file's internal id, so it merges into the file node, §21.1),
  `fn = () => 1` in `.jsx`, any `.vue`/`.svelte`/`.lua`/`.zig` stub, root workspace `Cargo.toml`.
  Such files still get a **file node** unless noted (JSON/TOML below are exceptions).
- `*.toml`: only files named `Cargo.toml` and `pyproject.toml` are code; `config.toml` is
  "not classified".

Doc (skipped in code-only): `.md .mdx .rst .txt .html`, and `CMakeLists.txt` (as .txt). Before
0.9.97 `.yaml .yml` were docs too.
Paper: `.pdf`. Image: `.png .svg`.

Not classified: `.css .scss .less .graphql .proto .xml .ini .cfg .conf .csv .jsonc .json5 .geojson
.pyi .ipynb (even a valid notebook) .hh .hs .clj .nim .fish .zsh .bat .cmd .pl .pm .fs .elm .ino
.cmake .lock .tsbuildinfo .markdown .toml.example`, and extensionless `Dockerfile Makefile Gemfile
Rakefile`.

Extensionless files with a shebang are code: `#!/usr/bin/env python3`, `#!/usr/bin/python`,
`#!/bin/sh`, `#!/bin/bash`/`env bash`, `#!/bin/zsh` (shell extractor; note `.zsh` *extension* is
not classified — quirk), `#!/usr/bin/env node`, `#!/usr/bin/env ruby`, `#!/usr/bin/env perl`
(counted as code, zero nodes). `#!/usr/bin/env -S deno run` is not classified. Interpreter names
are matched with an optional version suffix (§44.8): `ksh`/`dash` are shell, `node`/`nodejs`
JavaScript, `php` PHP, `fish`/`tcsh` inert like `perl`; `python3.11`, `ruby2.7` and `node18`
count, `Python3` and `pypy3` do not.
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

- `package-lock.json` and `pnpm-lock.yaml` are silently dropped (not counted as code, not
  reported). `yarn.lock`, `Cargo.lock`, `composer.lock`, `bun.lock` are `.lock` → not classified.
- Sensitive filter (reported as `N file(s) skipped as potentially sensitive`, probed on 0.9.86 and
  0.9.97 alike; only JSON mattered for the graph before YAML became code):
  - Always, whatever the type: `.env`, `.env.*`, `.envrc`, `*.pem`, `*.der`, `*.key`, `*.crt`,
    `*.p12`, `*.pfx`, `id_rsa`, `id_rsa.pub`, `id_ed25519`, `.netrc`, `.npmrc`, `.pgpass`,
    `.pypirc`, `.htpasswd`, `secring*`, and everything below a directory named `.ssh`, `.aws`,
    `.gnupg` or `.gcloud` (any depth, case-insensitive; `.ssh/x.py` too).
  - Files that are not source code (JSON and YAML count as not source; `.py .go .ts .sql .tf
    .csproj .xaml …` are spared): anything below a `secrets`, `credentials` or `.secrets`
    directory, and names whose stem (the name without its last extension), case-insensitively,
    ends with one of `secret secrets token tokens credential credentials password passwords
    passwd serviceaccount service-account service_account private_key` after `-`, `_` or `.`, is
    one of them, or has at most two such parts and starts with one: `secret.yaml`,
    `core.v1.Secret.yaml`, `x-token.json`, `token-config.yaml`, `secret.x.yaml`, `.secret`,
    `api_token.txt`; not `my-token-config.yaml`, `a-secret-b.yaml`, `tokenizer.yaml`,
    `secretary.yaml`, `private-key.yaml`, `api_key.yaml`. Markdown and reStructuredText are
    dropped only for the bare word (`secret.md`, `token.md`; not `tokens.md`, `x-secret.md`).
  - Not flagged: `credentials.py`, `secrets.py`, `id_rsa.py`, `secrets/mod.py` (extracted).
  - dijkstral implements this list in `discover.rs`; Graphify's own documentation describes
    further prose carve-outs that no corpus exercises.

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
| Q8 | TS/JS `anything.m()` binds to the last same-file method named `m`, which creates self-loops (`console.log` inside `.log()`); gone for TS/JS in 0.9.97. Go has the same last-definition rule for same-file method names. | §10.4, §12 |
| Q9 | TS barrel imports leave a dangling stub (`src_models_index_user`) beside the real edge. Dynamic-import phantoms carry `confidence` and no `source_location`, and can coexist with a `ref_*` stub for the same package. | §10.3 |
| Q10 | Go: cross-package type re-keying (`store_store_go_store_model`); root-level type ids depend on the checkout directory name; generic receivers create phantom `Store[T]` nodes; package imports link to `_test.go` files; external labels equal ids. | §12 |
| Q11 | Java: a wildcard import resolves to a class named like the package; static imports link to every same-named class; whether an import targets a class depends on its use elsewhere in the repo; `@Override` gets one stub per file. | §13 |
| Q12 | Python: calls to external modules are emitted only from top-level functions and only through `import x` / `import x as y` aliases. Calls qualified by a local module are dropped. | §9.4 |
| Q13 | Collapse artefacts: `contains`/`method` links carrying `context: "call"` (935 in the corpus); `uses` links carrying `parameter_type`; an EXTRACTED link with `confidence_score: 0.85`. | §8.2 |
| Q14 | Generic type parameters become global stub nodes (`t`, `u`, `f`) shared across unrelated files (Go: 0.9.86 only). | §9, §11, §12 |
| Q15 | Rust in-file and cross-file call resolution skips Python builtin names (`len`, `map`, `iter`, `min`, …). Rust type stubs can resolve to classes in extensionless Python scripts. | §11 |
| Q16 | JSON key ids use only the immediate parent key (`package_c_d` for `deep.a.b.c.d`). TOML package files are reported as "yielded no symbols" yet emit a node. | §14 |
| Q17 | `dynamic_import` edges have no `weight` and no `source_location`. | §7.1 |
| Q18 | Python: `from . import x` inside `pkg/__init__.py` produces an `imports_from` self-loop, which the build then drops. | §8.3 |
| Q19 | Kotlin/Scala: a definition named like the file's extension (`fun kt()` in `a.kt`, `def scala()` in `Main.scala`) merges into the file node, which gains `_callable` and a `contains` self-loop. | §21.1, §22.1 |
| Q20 | Kotlin: declarations found inside `init` blocks, secondary constructors, misparsed headers and anonymous objects outside functions become top-level functions of the file; Scala does the same for `def`s in expression blocks of a class body. | §21.1, §22.1 |
| Q21 | Kotlin: references to a qualified type use its **first** segment (`Interceptor.Chain` → `Interceptor`, `java.io.File` → `java`). | §21.2 |
| Q22 | Scala: imports link to the first path segment (`import shop.model.User` → `shop`). | §22.2 |
| Q23 | Rust cross-file calls bind bare names only: path-qualified calls (`a::b::f()`, `Type::f()` across files), `use … as` aliases, `self.field.m()` and method calls on typed locals and parameters (`let x = T::new(); x.m()`, builder chains) are never resolved, imported denylisted names (`build`) are dropped, and an imported external name still binds to a same-named repository function. | §11.5 |
| Q24 | Go method calls bind by bare name within the file (G5): a method call on a typed local, parameter, field or call result whose method lives in another file is dropped, as are `pkg.F[T](..)` and `pkg.F(..)` when a method of the package shares the name. | §12.2 |

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
- **Go (§12).** Unresolved stubs are per file, not global; type parameters make none (0.9.97).
  Same-file names that differ only in case get `_sha1(name)[:6]` unless the name is the group's
  only exported one. Stubs bind globally by label (unique type, else unique function), and a
  label any other language's definition carries binds nothing. In-file calls bind against all of
  the file's nodes, last wins, skipping a builtin denylist; bare cross-file calls bind by path
  proximity; typed receivers then bind by declared type (0.9.97). Receiver nodes exist for any
  receiver type not declared earlier in the file, and merge with the package's declaration.
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

All rankings run over the NetworkX graph Graphify analyses: nodes in **build order** (or, under
`cluster-only`, `graph.json` order), and links **added sorted by `(source id, target id)` as
written** (**Rule**, 2026-10-15: of `G.edges()` over the links in file order, over first
appearance, and over `(min, max)` pairs, only this reproduces the surprise order of all 19 corpus
graphs and 33 fixtures under `cluster-only`). Each node's neighbours follow that sort, so ties in
`G.edges()` order, betweenness and the bridge communities follow it. Degree counts a self-loop
twice. Under `cluster-only` the node order is the loaded `graph.json` order, which is why `extract`
and `cluster-only` can name different tenth god nodes for the same graph.

- **Symbol**: `source_file` has a suffix (`PurePath.suffix`; `benchsuite/benchsuite` does not) and
  the node is not a file node (label is a `/`-suffix of `source_file`). Stubs and externals
  (empty `source_file`) are not symbols; concepts with a `source_file` (JSON `ref_*`) are.
- **Minor function**: a label ending in `()` with degree ≤ 1. Never a god node, surprise endpoint
  or bridge (**Rule**, probes: `lonely()` with one link is skipped where a class with one link is
  not; with two links it qualifies).
- **Hub candidate** (surprise endpoints, bridges) = symbol, label not a method (`.x()`), not a
  minor function. Concepts, rationale and noise labels **qualify** (**Rule**: Swift's
  `extension String` is a surprise endpoint and a bridge, `net9.0` and XAML bindings in the
  `csproj` fixture are surprise endpoints, `Foundation` and `Protocol` are bridges in alamofire and
  okhttp, a rationale node is both).
- **God nodes**: hub candidates whose label is not in the noise list:
  `Any AsyncMock Bool Callable ClassVar Color Combine Comparable Counter Data Date Dict Double Enum
  Equatable Error False Final Float Font Foundation Hashable Identifiable Int List Literal MagicMock
  Mock Optional OrderedDict Path Protocol Sendable Set String True Tuple Type URL UUID Union View abc
  bool bytearray bytes complex datetime defaultdict float int io json object os patch re sentinel
  str sys typing` (case-sensitive; found by feeding ~593k candidate labels to `graphify god-nodes`)
  and not a generic JSON key (in a `.json` file, case-insensitively: `bundleddependencies
  bundledependencies data dependencies description devdependencies end id items key name
  optionaldependencies peerdependencies properties start title type value version`); top 10 by
  degree, stable. The wiki, `god-nodes` and MCP use the same rule.
- **Surprising connections**: links in `G.edges()` order between two hub candidates in different
  files, except `contains`, `method`, `imports` and `imports_from` links (**Rule**, one probe per
  relation: every other relation, `re_exports` included, can be a surprise). Score = confidence
  (EXTRACTED 1, INFERRED 2, AMBIGUOUS 3) + 2 if the top-level directories differ (`main.py` → `""`)
  + 1 if the communities differ + 1 if one end has degree ≤ 2 and the other ≥ 5. Stable sort by
  score, top 5; two links with the same labels and files are both listed (alamofire's two `.value`
  properties; the C# rule of one surprise per label pair was a tie-order artefact). `why` joins, in
  this order: `inferred connection - not explicitly stated in source`, `connects across different
  repos/directories`, `bridges separate communities`, ``peripheral node `p` unexpectedly reaches
  hub `h` ``; with none of them, `cross-file semantic connection`.
- **Community-bridge fallback** (**Rule**): when no link qualifies as a surprise, links between
  different communities, other than `contains`, `method`, `imports` and `imports_from`, whose
  endpoints are neither file nodes, methods nor minor functions (stubs, rationale and files without
  a suffix qualify), stably sorted by confidence (1/2/3, no other score), the first per unordered
  pair of communities, at most 5. Each has `note: "Bridges community a → community b"` instead of
  `why`, the communities of `G.edges()`'s `(u, v)`; the report adds `  _note_` after the files.
  Seen in the `misc` fixture (`Area() → Compute()`, both in `pascal/Shapes.pas`).

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

- **Summary**: the `(S shown, T thin omitted)` clause only when `T > 0` (**Rule**).
- **Words**: `len(text.split())` per file, thousands separators. Files/words count every file
  `update` indexes (code and markdown; dijkstral counts the code files it indexes).
- **Unclassified** (`update` only, **Rule**): `- Unclassified: N file(s) not represented in the
  graph (top: .sbt 2, (none) 1)` after the verdict, when the walk saw files that are neither code
  nor a document, paper, image or media file (`md mdx rst txt html yaml yml pdf png svg jpg jpeg gif
  webp mp4 mov webm m4v mkv avi mp3 wav ogg m4a docx xlsx gdoc gsheet gslides`, probed one
  extension at a time). Lockfiles (`package-lock.json Cargo.lock yarn.lock composer.lock
  poetry.lock Gemfile.lock go.sum`; not `bun.lock`) and likely secrets (`.pem .key .crt .p12 .pfx
  id_rsa .npmrc .netrc .env`) are not counted. The key is `PurePath.suffix` as written (`.SBT` and
  `.sbt` are distinct, `a.tar.gz` is `.gz`, dotfiles are `(none)`); the three most common, ties
  in sorted path order. **Open**: `.env.example` counts in Graphify but dijkstral's sensitive
  filter never walks it.
- **Percentages**: `round(100·count/links)`; the INFERRED clause only when there is one, with the
  mean INFERRED `confidence_score` as `repr(round(x, 2))` (`0.5`, `0.91`). A link without a score
  counts **0.5** there although `graph.json` writes it as 0.55, so `update` and `cluster-only`
  disagree on the same links (`misc`: 0.5 and 0.55).
- **Listed members** (Community sections, hubs, thin): symbols, not rationale, not methods, not
  functions (`…()`) with degree ≤ 1; in id order, first 8 shown. *Thin* = fewer than 3 listed.
- **Knowledge Gaps**: the section is omitted when there are no isolated nodes and no thin
  communities (**Rule**).
- **Isolated nodes**: symbols, not rationale, not functions, degree ≤ 1, unless their one link is a
  `contains`; build order; the explanation adds `or undocumented components` when
  the graph has `document` nodes; the total counts every node with degree ≤ 1.
- **Import cycles**: a file digraph from `imports_from` links and module-level `re_exports` (target
  is a file node), skipping `type_only` and `deferred` imports (erpnext's `TYPE_CHECKING` cycle,
  excalidraw's dynamic imports); simple cycles of at most 5 files, shortest first, each rotated to
  start at its smallest path, at most 20.
- **Suggested questions**, interleaved one per kind per round (bridge, verify, isolated, low
  cohesion), at most 7:
  - *bridge_node*: of the **3** hub candidates with the highest betweenness (stable), those
    touching other communities (**Rule**: jq skips `RoffWalker`, ranked second with no other
    community, and still asks about the third; Vue's `formatTitle()` was fourth). ``Why does `X`
    connect `<own>` to `<a>`, `<b>`?`` — other communities in the iteration order of a CPython
    `set` built from the neighbours in adjacency order (above). Why: `High betweenness centrality
    (0.304) - this node is a cross-community bridge.`
  - *verify_inferred*: god nodes among the first 5 with ≥ 2 INFERRED links. ``Are the n inferred
    relationships involving `X` (e.g. with `a` and `b`) actually correct?`` (first two INFERRED
    neighbours by id).
  - *isolated_nodes*: ``What connects `a`, `b`, `c` to the rest of the system?`` / `n
    weakly-connected nodes found - possible documentation gaps or missing edges.`
  - *low_cohesion*: communities with cohesion < 0.15 (a connected community needs 14+ members), in id
    order: ``Should `label` be split into smaller, more focused modules?`` / `Cohesion score
    <repr> - nodes in this community are weakly interconnected.`
  - None at all: the report says `_Not enough signal to generate questions. This usually means the
    corpus has no AMBIGUOUS edges, no bridge nodes, no INFERRED relationships, and all communities
    are tightly cohesive. Add more files or run with --mode deep to extract richer edges._` and the
    `cluster-only` sidecar lists `{"type": "no_signal", "question": null, "why": "<the same
    sentence>"}` (**Rule**).

### 17.6 Goldens

`tests/fixtures/<lang>.graphify_analysis.json` (from `extract`), `<lang>.graphify_report.md`
(from `update`; `mixed` from a copy without `README.md`, which `update` would index) and
`<lang>.cluster_only.graphify_{analysis.json,report.md}` (from `cluster-only --no-label` on
`<lang>.clustered.graphify.json`). Reports are compared after replacing the title line (directory
name and date) and `graphify update`/`graphify query` in hints with `dijkstral …`; analysis files
need no normalisation.

### 17.7 Remaining differences

Verified 2026-10-15: `dijkstral cluster-only` reproduces `graphify cluster-only --no-label`'s
`graph.json`, analysis and report byte for byte on all 33 fixtures, and the graph and analysis on
all 19 corpus graphs; the extract-time analysis and report match on every fixture whose graph
matches, except the two below.

- **Extraction**: wherever dijkstral's node or link set differs from Graphify's (the `ALLOWED`
  quirks, stage-1 parity), Leiden sees a different graph and community ids move globally
  (NMI 0.83–0.95 on the corpus).
- **Extract-time ties in node order**: equal-score surprises tie in build node order, which
  differs where Graphify merges nodes: Objective-C headers and implementations (`objc`) and XAML
  bindings (`csproj`, whose `update` graph is also larger than `extract`'s).
- **verify_inferred**: on fastapi (`HTTPException`) and erpnext (`make_stock_entry()`,
  `make_sales_order()`) Graphify asks no question for a top-5 god node with ≥ 2 INFERRED links,
  although it does for the same node in its ego graph, and stopping at the first god node without
  any breaks ripgrep and guzzle. Not explained (**Open**).
- **Import cycles**: the same cycles are found where Graphify lists all of them (fastapi, monet,
  erpnext, graphify); the order of equal-length cycles, and therefore which 20 are listed in
  ripgrep, excalidraw and Kubernetes, differs. Graphify's enumeration order is not
  `networkx.simple_cycles` over the arcs in link or `G.edges()` order (tested).
- **Words** count the code files dijkstral indexes; `update` also counts markdown and other
  documents.

---

## 18. C# (2026-10-14)

Fixture `tests/fixtures/csharp/` (file-scoped and block namespaces `Shop.Models`,
`Shop.Services`, `Shop.Tests`; a `partial class User` split over `User.cs` and
`User.Validation.cs`). Goldens: `csharp.graphify.json`, `csharp.clustered.graphify.json` and the
§17.6 report goldens. Probes `p1`–`p6` (namespaces, members, call forms, `new`, `.csproj` and
`.razor`) and the serilog corpus (Graphify's `graphify-out/cache/ast/*.json` per-file facts,
`raw_calls` and `member_field_tables`, were read as observable output). Re-checked against
Graphify 0.9.97 (2026-10-29) with probes `p1`–`p15` run through both releases: overload ids,
signature metadata, delegates, overload-aware calls, receivers typed by return types, `foreach`
and field steps, and `dispatches_to` changed. Implementation:
`crates/core/src/{extract,resolve}/csharp.rs`.

### 18.1 Nodes

| Construct | id | label | extra |
|---|---|---|---|
| file | `fileid` (§4.2) | basename | — |
| namespace (every declaration, nested ones joined: `Outer.Inner`) | `csharp_namespace:` + `sha1(name)[:16]` — one node per name for the whole repo | the full name | `type: "namespace"`, `metadata {"kind":"csharp_namespace","namespace":…}`; `source_file`/`source_location` of the first declaring file in path order |
| class, struct, interface, enum, record, `record struct`, delegate (**nested types flat**) | `fileid_slug(ns.Name)`, plus `_<arity>` for generic types (`…_ientity_1`, `…_c3_2`, delegate `Factory<T>` → `…_factory_1`); global namespace `fileid_slug(Name)` | `Name` | `_callable`, `_callable_class` |
| method (also interface/abstract/partial/extern, explicit `IX.Do()`) | `typeid_slug(name)`; an overload whose signature differs from the **previous** declaration with that id gets `_` + `sha1(signature)[:6]` (below) | `.Name()` | `_callable` |
| property (interface properties too) | `typeid_slug(Name)` | `Name` | — |
| enum member | `enumid_slug(Name)` | `Name` | — |
| method outside any type (tree-sitter error recovery, e.g. serilog `ILogger.cs` broken by `#if`) | `fileid_slug(name)` (no namespace); overloads merge, first line kept, no signature metadata | `name()` | `_callable` |
| unresolved type (field, parameter, return, base, generic argument, attribute) | `slug(Name)`, **one global node** (no §4.5 renaming) | `Name` | stub shape |
| `using N;` of a non-local namespace, `using static`, alias to a non-local type | `slug(text as written)` (`system_collections_generic`, `system_math`, `system_datetime`) | = id | external |

Every symbol node in a namespace has `metadata`, keys in this order: for methods `arity`,
`min_arity`, `variadic` and `signature` (below); `is_nested_type: true` (nested types),
`is_partial: true` (a `partial` declaration) and, after it, `type_path` (the enclosing types and
the name, without namespace: `User`, `Outer.Inner`); `namespace`; `scope_chain` — the
`s<row>:<col>` (0-based) of every enclosing namespace declaration, outermost first (a file-scoped
namespace on line 15 is `["s14:0"]`) — and, cross-file, `unresolved_calls`. Symbols in the global
namespace have no `metadata` unless they are methods, nested, partial or have unresolved calls.

**Method signature (Rule, 0.9.97).** `signature` joins the parameters with `,`: each is its
modifiers (`ref`, `out`, `in`, `readonly`, `this`, `params`) and its type with every whitespace
character removed (`Dictionary<string, int>` → `Dictionary<string,int>`, `scoped Span<int>` →
`scopedSpan<int>`, a named tuple `(int a, string b)` → `(inta,stringb)`, qualified names as
written: `System.String`, `global::System.Int32`), joined by a space (`ref readonly int`,
`params object[]`, `this string`); attributes, names and default values are left out and
`__arglist` is an empty entry. A generic method prefixes `` `N: `` (`` `2:T,U ``). The stored text
is HTML-escaped (`List&lt;int&gt;`); the overload suffix hashes the unescaped text
(`User,Point` → `_512555`; an empty list → `_da39a3`). `arity` counts the parameters;
`min_arity`, written only when smaller, counts those without a default value that are neither
`this` nor `params` (`#if` lines inside the list do not count); `variadic: true` marks a `params`
parameter. A property and a method with the same id stay one node (the property's), as before.

**Overload ids (Rule, 0.9.97).** Ids are assigned in declaration order. The first method with an
id keeps it; a later one with the same id keeps it too when its signature equals the previous
declaration's under that id (both merge, the node keeping the first line), and otherwise takes
`_<sha1(signature)[:6]>` (`Do()`, `Do(int)`, `Do(int)` → `do`, `do_46f8ab`, `do`; probe `p10`:
`Twin(int)`, `Twin(int)`, `Twin(string)`, `Twin(string)` → `twin`, `twin`, `twin_ecb252`,
`twin`). serilog: 369 suffixed nodes, every suffix reproduced. 0.9.86 merged every overload.

Not emitted: constructors (static ones too), fields, events, indexers, operators, conversion
operators, destructors, local functions, lambdas, top-level statements and their local
functions, type parameters (no stub). A delegate's parameters give no reference.

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

Per file (no `confidence_score`): a bare call `M()` binds to a method named `M` of the enclosing
type, else of any type in the file; `new T()` binds to a type `T` declared in the file
(constructors or not). Calls named like a predefined type (`Int()`, `String()`, `Decimal()`) are
ignored. Constructors' calls come from their type; lambdas' and local functions' from the
enclosing method. **0.9.97:** a call on any other receiver that is not a name, `this` or `base`
(`f().M()`, `a.b.c.M()`, `kvp.Key.Render()`, `d["k"].M()`) no longer binds by name in the file;
the forms below type some of them.

**Overloads (Rule, 0.9.97).** Whatever finds a method by name finds the overloads of the first
declaration with that name; the call binds to the **only** one that takes its argument count
(`min_arity ≤ n ≤ arity`, any `n ≥ min_arity` when variadic), else to the first (probe `p9`:
`Do(1)` → `do_46f8ab`; `Q()` and `Q(1,2,3)`, which fit none, → `q`; serilog
`CreatePropertyValue(value, d, 1)`, which fits two, → the first). "The only method named `M`"
counts types, not overloads.

Cross-file (unbound bare and `new` calls of the file first, then the rest in source order):

| form | binds to | confidence |
|---|---|---|
| `M()` | the enclosing type's or a base type's `M`, else the only type's method named `M` | INFERRED 0.85 |
| `this.M()`, `base.M()` | own type and bases / bases | EXTRACTED 1.0 |
| `X.M()`, `X?.M()` with a capitalized `X`; `(x as T).M()`, `((T)x).M()` | `X` as a type (lookup, else the only type named `X`), else the declared type of field/property/local `X` | EXTRACTED 1.0 |
| `x.M()` on a lower-case parameter, local (`T x`, `var x = new T()`, `is T x`, `out T x`, `foreach (T x in …)`), field or property (also `this.x`) of non-predefined declared type | that type (lookup, else unique name) and its bases | INFERRED 0.85 |
| `x.M()` after `var x = <call>` (0.9.97), the call being `F()`, `this.F()`, `base.F()`, `X.F()` or `y.F()` on a typed name, `await` not included | the declared return type of the method that call binds to, when that method has no overloads (`User?` → `User`, `Box<User>` → `Box<T>`); looked up from the callee's file; never recorded in `unresolved_calls` | INFERRED 0.85 |
| `x.M()` in `foreach (var x in f)` and `f[i].M()` (0.9.97), `f` a field or property (also `this.f`) of type `T[]` or a one-argument generic `G<T>`, not shadowed by a local | `T` | INFERRED 0.85 |
| `a.f.M()`, `a` a name of declared type and `f` a field or property of that type or a base | `f`'s declared type (one step: `a.f.g.M()` binds nothing) | INFERRED 0.85 |
| `new T(…).M()` (0.9.97) | `T` | INFERRED 0.85 |
| `new T()`, `T` not used as a type in the file | lookup, else the **type** named `T` nearest the caller's directory (0.9.86 also took same-named methods: `new LogEventProperty(…)` in a test went to `Some.LogEventProperty()`) | INFERRED 0.85 |
| `new T()`, `T` used as a type in the file | lookup only, and not when the caller already references `T` | INFERRED 0.85 |

`foreach (var x in …)` over anything else, `var x = f()` on an overloaded `f` or on a chain, and
`var x = a.f` give `x` no type; an untyped lower-case receiver never binds.
**`metadata.unresolved_calls`** lists, per caller and in source order, one
`{"callee","receiver_type","lang":"csharp","line","file"}` per (callee, type) for a capitalized or
typed receiver whose type names no local type: `Console`, `Assert`, `Guid`, `List` (from
`List<Address> _addresses`), `T`, `TextWriter` (a property `Output` of that type), and `Street` /
`Renamed` (a property of predefined type, an event: the receiver's own name). `file` (0.9.97) is
the caller's file.

**`dispatches_to` (Rule, 0.9.97)**: for each class listing a local interface in its base list,
each interface method links to the class's method of that name with the same signature, else its
first method of that name, INFERRED with context `call` at the class's line in its file. The score
counts the classes that implement the interface method: 0.85 for one, 0.75 for two, 0.65 for
three or more (no cap; serilog `ILogEventSink.Emit` has 11). A call from the implementation to the
interface method ranks with `dispatches_to` in the collapse (§8.2, opposite orientation, equal
rank, different relations): the larger source survives, so `ILogEventEnricher.Enrich` keeps its
dispatches over `ConditionalEnricher`'s forwarding call while the `…sinks_…` forwarders keep their
calls (serilog: 13 dispatches, all reproduced). 0.9.86 linked only an interface method with
exactly one implementation and no call.

### 18.5 `.csproj`, `.sln`, `.razor` in a code-only build

Resolved: see §43 (project files, solutions, Razor, XAML).

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
- Surprises: an earlier reading had Graphify list one surprise per (source label, target label,
  source files). It does not (alamofire lists two `.value → Result` links); the `csharp` fixture's
  order is the sorted-link tie order of §17.4.

### 18.7 Parity (2026-10-29, Graphify 0.9.97)

- Golden `csharp`: nodes 93/93, links 142/142, identical communities, analysis and report; the
  three CS1 field differences remain. `csproj`: nodes 113/113, links 134/134.
- serilog (`--ext .cs,.csproj,.sln,.slnx,.props,.targets` against the frozen 0.9.97 reference):
  nodes 2,238/2,238 both ways (0.9.86 code: recall 83.5%); links 6,038 shared, 4 only ours and 5
  only Graphify's, all `calls` (0.9.86 code: recall 80.3%, precision 95.3%); `references`,
  `method`, `dispatches_to` and the rest exact. The calls that remain differ against 0.9.86 too
  (**Open**): a primary-constructor parameter as a typed receiver
  (`DelegatingLoggingFailureListener(ILogEventSink sink)`), `new A()` binding to a property `A`
  of another nested type rather than the nested class `A`, a local function `CreateLogger()`
  bound to a same-named method of another class, and `Error(…)` from one of its own overloads,
  which Graphify leaves unlinked.
- Field differences there: CS1 (3), CS4 receivers in `unresolved_calls` (5 callers), and 8
  `new T()` calls Graphify marks EXTRACTED where dijkstral says INFERRED 0.85 (also against
  0.9.86, **Open**).
- Graphify corpus `.cs`/XAML/Razor/project and PowerShell files: the 12 method `metadata` fields
  now agree; the differences of §41.3 and §43 remain.
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
| method (abstract and interface methods included) | `type_name` (`__construct` → `construct`) | `.__construct()` | `_callable`; `metadata.unresolved_calls` (§19.4) |
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
| `calls` | caller → callable / class (`new X` only) | §19.4 | EXTRACTED (in file), EXTRACTED 1.0 or INFERRED 0.85 | `call` | first call line |
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
The 0.9.97 rules below come from twelve probe repositories (static calls, typed receivers,
closures, ambiguity, unresolved records) and guzzle (5,969/5,969 calls).

| Form | Result |
|---|---|
| `f()`, `$x->m()`, `$this->m()` with a definition named `f`/`m` in the file | EXTRACTED to the **last** function or method of that name in the file (any class, quirk like Q8); `$this->m()` inside `m` keeps its self-loop |
| `$x->m()` / `$this->p->m()` with no `m` in the file, `$x` / `p` typed by the file (below) | INFERRED 0.85 to the own method `m` of the only class-like named like the type |
| `$this->m()` with no `m` in the file, untyped receivers, `$x?->m()`, `Status::A->m()`, `foo()->m()` | dropped |
| `X::m()` (name or qualified name, `\` prefix allowed, last segment; aliases not expanded) | a class of the file named `X` → its own method `m`; else the only class-like named `X` in the repository → its own `m`, EXTRACTED 1.0. Never the class |
| `self::m()`, `static::m()` in a method | the method's class's own `m`, EXTRACTED |
| `parent::m()` in a method | the own `m` of the class's written parent (in the file or the only one of that name), EXTRACTED |
| `self::`/`static::`/`parent::` inside a closure or arrow function | dropped |
| `X::m()` resolving to the calling method itself (`self::rec()` in `rec`) | dropped |
| `new X` with class `X` in the file | EXTRACTED to the class |
| `new X` where `X`'s last segment is a non-aliased import of a class | EXTRACTED 1.0 to the class |
| `new X` otherwise (also `new A\B`, `new \X`), unless the file writes `X` as a type | INFERRED 0.85 to the nearest class `X` (§19.5) |
| an aliased `new C` | dropped |
| bare `f()` not in the file | INFERRED 0.85 to the nearest function or method `f` |
| a builtin-like name (`Proxy`, `Promise`, `Map`, `len`, `sorted`, … the Q15 list) as `f()` or `new X` | dropped (`Map::of()` and a `Map $m` receiver bind like any other class) |
| a cross-file call whose caller already links to the target in the file (type reference, constant use) | dropped |

- **Receiver types (Rule).** One map per file, built over the whole file before any call (a use
  may precede the declaration, and a type declared in one method types the same variable name
  in every other function, class and closure of the file): variable or property name → the
  **first** declaration in source order among typed parameters (promoted ones included),
  typed properties (`private ?T $p` types both `$p` and `$this->p`) and `$v = new T(...)`. A
  declared type names a class only as `T`, `?T` or `\A\T` (last segment); unions,
  intersections and primitive types type nothing; `self` stays the name `self`. A static call's
  result (`$v = X::make()`) types nothing.
- **Class lookup.** By short name over every class, interface, trait and enum of the repository,
  ignoring namespaces and imports (`use App\A\Dup; Dup::dm()` with two `Dup` classes binds
  nothing). Only the class's **own** methods count: an inherited `m` binds nothing.
- **Rule (`metadata.unresolved_calls`).** A typed `$x->m()` whose type, or an `X::m()` whose
  name, matches no class-like of the repository is recorded on the caller (closures included) as
  `{"callee": m, "receiver_type": …, "lang": "php", "line": "L<n>", "file": <caller's file>}`, in
  source order, once per (callee, receiver type) and caller. The receiver type is the typed name
  (last segment: `Ext4` for `\Vendor\Ext4 $c`, `R` for an alias) or the static scope as written
  (`\Vendor\Ext`, `Psr7\DiagnosticValue`); builtin-like names are recorded too
  (`Promise::all()`). An ambiguous name, a class without the method and `parent::m()` record
  nothing. guzzle: 831 callers, 1,600 entries.
- **Changed from 0.9.86.** `X::m()` linked the class (EXTRACTED, 1.0 through an import, or
  INFERRED to the nearest class); `self::`/`static::`/`parent::` were dropped; typed receivers
  and `metadata` did not exist.

`X::CONST`/`X::class`/`X::$p` link only when the file declares `X` or writes it in a signature
(→ its per-file stub) or a supertype list (→ the shared supertype stub); unqualified names only;
`self::`/`static::`/`parent::` and class-body initialisers produce nothing.

### 19.5 Nearest candidate (PHP and Ruby)

A name bound across files (INFERRED) picks the candidate with the greatest key
(**the candidate is a test file exactly when the caller is**, shared leading directories, same
directory, same file); an equal best binds nothing.

- **Rule (blocking labels).** For a bare call `f()` (PHP) or `f` (Ruby), every node of any
  language whose label is exactly `f` competes too — a JSON key (`scripts.aa`), a package
  dependency (`ref_bb`, `bb@1.0`), a Cargo package, and since 0.9.97 a YAML key — while other
  languages' functions (`cc()` in Python, `dd()` in JavaScript) do not. When such a node is the
  nearest candidate, or ties with the best function, the call binds nothing: sinatra's
  `env['…']` in `lib/` lost its link to `rack-protection/spec/support/spec_helpers.rb` because
  `.github/workflows/test.yml` has two `env` keys (non-test candidates beat a test one, and the
  two tie). Test files: a `test`, `tests`, `spec`, `specs`
or `__tests__` directory, or `*_test`, `*_spec`, `test_*`, `*.test.*`, `*.spec.*`. Measured
pairwise from `rack-protection/spec/support/x.rb`: `test/` beats `rack-protection/lib/rack/`,
`rack-protection/spec/` beats `test/`, `test/` ties `spec/`.

### 19.6 Quirks and open questions (PHP)

- **Rule (fold).** A class `extends`/`implements` link and a property `field` reference on the
  same pair keep no context: `inherits` and `implements` are structural (§8.2).
- **Open.** `use GuzzleHttp\Server\Server;` inside a braced namespace becomes the external
  `server` when a later namespace block of the file defines a function (guzzle
  `tests/bootstrap.php`); dijkstral keeps the class import.
- **Parity (2026-10-29, guzzle against 0.9.97, `--ext .php`).** Nodes 4151/4152 (the `server`
  external), links 12,006/12,007 recall and precision, calls 5,969/5,969, no field differences.
- **Open (pre-existing).** Graphify binds an unqualified type to the class of the current
  namespace (`Dup` in `App\A` → `App\A\Dup`) when several classes share the name; dijkstral
  keeps the per-file stub.

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
| `typealias Name = …` (0.9.97), in the file or a class body | `file_slug(Name)`, `contains` from the file or class; generic aliases keep the bare name (`Boxed<T>` → `Boxed`) | `Name` | `_callable`, `_callable_class` |
| anonymous object in a function body | `<function>_object_slug(Type)_l<line>` (first supertype, last segment) | `Type` | `_callable`, `_callable_class` |
| type referenced but not declared earlier in the file | per-file stub `slug(Name)` | `Name` | §4.4 |
| unresolved import | external `slug(last segment)` | | §4.4 |

- Not emitted: properties (top-level or member), constructors (primary or secondary), `init`
  blocks, accessors, local functions and classes, script statements of `.kts` files, and
  rationale (KDoc, `// NOTE:` comments). A type alias references nothing; references to it bind
  to its node like to a class (`fun early(x: Later)` before `typealias Later = Thing` →
  `-> a_a_later`), and so do imports of it (`import a.Id` → `a_a_id`). 0.9.86 emitted no alias
  node (the fixture's `typealias UserId = String`, probe `jvm-k6`).
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
  nothing. The type parameters of the enclosing classes and function produce nothing (0.9.97;
  0.9.86 made them stubs `T` → `t`): `class Q<T : Thing>(val t: T)`, `fun <U : T> m(u: U):
  List<U>`, `Repository<T>` as a supertype argument. An undeclared `T` (`fun stray(): T?`)
  is still a stub.
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

**`metadata.unresolved_calls`** (0.9.97): on the caller (function, method, the class for its
property initializers, the file for top-level ones), in source order, one
`{"callee", "receiver_type", "lang": "kotlin", "line", "file"}` per (callee, receiver) for each
`Type.m(…)` with a capitalised identifier receiver that bound nothing in the file (a file
callable named `m` binds `Other.lookup()` whatever the receiver) and whose `Type` is neither
the label of a class-like node of a Kotlin file — classes, objects, enums, interfaces, type
aliases and anonymous objects (`object : Thread() {}` makes `Thread.sleep()` resolved), even
when ambiguous or lacking `m` (`Helper.missing()`) — nor a denied receiver. Java classes do not
count (`JavaThing.missing()` is recorded). Denied receivers (probed name by name, partial):
`Array BigInt Blob Bool Boolean Bundle Character Color Data Date Decimal Double Error EvalError
File Float Font FormData Headers Int Int32 Int64 Intl JSON Locale Map Math Number Object Promise
Proxy RangeError Reflect RegExp Request Response Set String Symbol SyntaxError TypeError URL
URLSearchParams UUID WeakMap WeakSet`, `AbortController AnyObject Calendar DateFormatter
DispatchGroup DispatchQueue FileManager Int8 Int16 JSONDecoder JSONEncoder NSError NSLock
NSString NotificationCenter NumberFormatter OperationQueue ReferenceError Sendable TextDecoder
TextEncoder TimeZone UInt UInt8 UInt16 UInt32 UInt64 URIError URLComponents URLRequest
URLSession UserDefaults View` (not `Uuid`, which the call denylist has). The callee name is not
filtered (`Objects.hash`, `Integer.parseInt` are recorded). Not recorded: `a.B.m()` and longer
receivers, lowercase or typed receivers (`x.go()` with `val x: Thing`), calls in `init` blocks.
Probes `jvm-k1`–`k5`, `jvm-k3` (Java receivers, aliases, ambiguous classes).

### 21.5 Parity

Fixture: 100% of nodes and links, communities and array order exact, report goldens exact
(0.9.97 goldens). Corpus okhttp (`.kt`/`.kts`, frozen reference): file and def nodes 100%/100%;
all nodes 99.9% recall / 100.0% precision; links 99.8%/99.8% (`calls` 99.7/99.8, `imports`
99.8/99.8, `references` 99.9/99.7, all other relations ≥ 99.6). Against 0.9.97 (2026-10-29,
`.kt .java .kts`): nodes 99.90/99.58, links 98.60/98.98, every `unresolved_calls` entry equal
(620 Kotlin, 75 Java). Remaining: in-file call targets where
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
| any type name not declared in the file | per-file stub `slug(Name)` — **no denylist**: `String`, `Int`, `Unit`, `List`, `Option` are stubs; never a type parameter of the enclosing classes or `def` (0.9.97: `trait Repo[K, V]`, `class Box[+T]`, `def map[U]` reference no `k`, `v`, `t`, `u`; 0.9.86 stubbed them; an undeclared `T` still is one) | `Name` | §4.4 |
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
and `// NOTE:`/`// WHY:` are ignored). The generic parameters of the enclosing type declarations
and function (`struct Box<T>`, `func map<U>`, `Pair<Key: Hashable, Value>`) are never referenced
and make no stub (0.9.97; 0.9.86 treated them as undeclared types, `T` → `t`); scope is
lexical: `func c(_ y: Gadget)` beside `struct W<Gadget>` still references `gadget`, and an
undeclared `T` is still a stub. Associated types are not type parameters (probe `jvm-w2`).

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
  (`indirect case nested(EI)`). References dedupe per declaration by (target, context). `Self`
  references nothing (0.9.97; `func me() -> Self` made a stub `self` in 0.9.86).
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
parameters, `for`/`if let`/`guard let` bindings and top-level statements are not recorded, and
neither is a parameter or property typed by a type parameter in scope (0.9.97): with `func b(_
x: Thing)` then `func a<X>(_ x: X)`, `x` stays `Thing` (0.9.86 overwrote it with `X`), and
`items: [T]` in `struct Box<T>` records nothing, so `items.append(…)` is not unresolved.

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
   the caller's `metadata.unresolved_calls` (`{"callee", "receiver_type", "lang": "swift",
   "line", "file"}`; `file` new in 0.9.97), except builtin-like types (the list above, `Int`,
   `Set`, `Sendable`, `Locale`). Ambiguous or extension-only types elsewhere give nothing.
5. A cross-file edge is skipped when the caller already links to the target (any relation), which
   is why an INFERRED edge can shadow a later EXTRACTED one.
6. **`dispatches_to`**: a protocol links each requirement to every conforming type's
   (`implements`) member of the same name, context `call`, at the conforming type's node
   (`source_file`, `source_location`), INFERRED with a score by the protocol's number of
   conforming types — 0.85 for one, 0.75 for two, 0.65 for three or more, whether or not each
   one implements the requirement (probes `jvm-w5`, `jvm-w6` up to 12). 0.9.86 linked only a
   protocol with exactly one conforming type (Alamofire: 15 edges then, 60 in 0.9.97).

### 23.5 Build-stage observations

- Swift structural edges lack a `context` key, so a folded edge keeps the other edge's context:
  `contains` + a later `references field` on the same pair gives `contains` with
  `context: "field"`, and in the opposite orientation `implements` + `references field` keeps
  `implements` with `context: "field"`; a `calls` edge folded over a `references return_type` loses
  `swift_plain_return`. dijkstral's fold (§8.2) merges no keys into a surviving structural link
  and none across orientations (**Quirk S4**, merges of extensions after the fold; 104 link
  fields on Alamofire after the shared fold rules of 2026-10-15).
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
Graphify's cache for all 134 files. Against 0.9.97 (2026-10-29): nodes 99.91/99.97, links
99.79/99.79 (`dispatches_to` 60/60; the rest is the §23.5 `…_swift_codingkey` rename).

### 23.8 Open

- Whether an extension constructing its own type from its body binds in general: the only such
  call Alamofire has outside the denylist (`CharacterSet(...)` in a property closure of
  `extension CharacterSet`) binds; `DispatchQueue(...)` and `Bundle(...)` are denylisted.
- The full builtin-type denylists (calls and `unresolved_calls`) are known only for the names
  Alamofire exercises.

### 23.9 Analysis

The god-node noise labels (§17.4) keep `String` out of the god nodes only: an `extension String`
symbol is a surprise endpoint and a bridge (fixture `swift`, `cluster-only`). The shared analysis
applies this (**Rule**, §17.4), and the fixture's extract-time and `cluster-only` sidecars match
byte for byte.
## 24. C

Fixture `tests/fixtures/c/`; goldens `c.graphify.json`, `c.clustered.graphify.json` and the report
goldens of §17.6. Extensions `.c` and `.h`; tree-sitter-c 0.24.2 (Graphify's version). Rules were
isolated with probe repositories run through `graphify extract --code-only` (0.9.97, each change
from 0.9.86 checked by running both releases on the same probe); jq (`.c`/`.h`) is the corpus
check. Implementation: `crates/core/src/{extract,resolve}/c.rs` (shared with C++).

### 24.1 Which language a file is

- **Rule.** `.c` is C. `.h` is C++ when its bytes contain any of `class `, `namespace `,
  `template`, `::`, `public:`, `private:`, `protected:` or `extern "C"` anywhere, comments
  included (case sensitive, plain substrings: `// subclass Foo` makes a header C++, `// class`
  followed by a newline does not; `extern "C"` must be written exactly so: `extern "C++"`,
  `extern  "C"` and `extern"C"` do not count, nor does `__cplusplus` alone). Every other `.h` is
  C. Probed up to 200 KB into a file. `extern "C"` is new in 0.9.97.
- C headers therefore get no class, struct, field or typedef nodes. jq's headers, which all wrap
  their declarations in `#ifdef __cplusplus extern "C" {`, are C++ since 0.9.97 (their structs,
  enums and typedefs are nodes); fmt's C++ headers always were.

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
- Not emitted (C files and C headers): prototypes, `struct`/`union`/`enum` (with or without
  body), typedefs, macros (`#define`), globals, and comments: C has no rationale nodes
  (`NOTE:`/`WHY:`/`HACK:` ignored).
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
(`../include/util.h` works). `<p>` never resolves that way.

**Search (0.9.97).** A quoted `p` that names no file relative to the including one is looked up
by suffix: the indexed files whose path is `p` or ends with `/p`, whole components, after
`os.path.normpath` (`./sub/v.h` is `sub/v.h`; a `..` left over, as in `../sub/w.h`, finds nothing).
One candidate is taken wherever it is; among several, the one sharing the most leading directories
with the including file, and a tie (two candidates sharing as many, including none) finds nothing.
There is no depth tie-break (`a/b/p.h` and `a/b/c/p.h` tie for `a/x.c`). Probes: `"shape.h"` from
`src/shape.c` → `include/shape.h`; `"geo/shape.hpp"` from `src/` → `include/geo/shape.hpp`;
`"b/y.h"` does not match `ab/y.h`. Angle includes are never searched (`<sub/v.h>` with a single
`inc/sub/v.h` stays external).

Every quoted include that did not resolve relative to the including file, found by the search or
not, carries `metadata {"raw_include": p, "include_kind": "quoted"}` (`p` as written) on its edge,
whatever it lands on below; relative and angle includes carry none.

Otherwise the target is the external id `E = slug(stem of p)` (`<sys/types.h>` → `types`), and
Graphify then lands the edge, in this order, on:

1. the stub of the first header (`.h`/`.hpp`, by path) that has a renamed (§4.5) stub `E`
   (`#include "jv.h"` from `tests/` → `src_jv_h_jv`; fmt's `<vector>` →
   `test_gtest_gmock_gmock_h_vector`);
2. a node of the including file with local id `E` (its own stub `Shape` next to
   `#include "shape.h"`), renamed and bound like that stub;
3. any node whose id is `E` (an unrenamed stub elsewhere);
4. the only definition labelled exactly `E` (a class, field or enumerator `E`, or a function
   `E()`; not a method): fmt's `<ostream>` → class `ostream`, `"fmt/printf.h"` → `printf()`;
5. the only file whose stem is `E`, in any language (§8.1; jq's `"version.h"` →
   `scripts/version`; since 0.9.97 indexes YAML, jq's `"oniguruma.h"` →
   `.github/workflows/oniguruma.yml` and fmt's `<windows.h>` → `windows.yml`);
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
- A bare `f()` named like a stub of the file (a type `F` the file references anywhere, `Pod()`
  beside a parameter `Pod p`) binds nothing, in the file or across files.
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
- **Typedefs and aliases (0.9.97).** `typedef T X;` (each declarator, through pointer, array and
  function declarators: `typedef int A1, *B1;`, `typedef int (*Fn)(int);`), `typedef struct
  {...} X;` and `using X = T;` (template aliases too) give a node `file_slug(X)`, label `X`, line
  of the declaration, no `_callable`, contained by the file, with no other edge; at file or
  namespace scope, in `extern "C"` blocks and `#if` branches. Not: in class bodies (also a misread
  class body, §25.1 below, and `#if` inside one), in function bodies, when the nearest enclosing
  scope (`template` and `#if` aside) is an error-recovery node (fmt's `format.h`, whose namespaces
  open through macros), and when `T`'s last name is `X` itself (`using qux = ns::qux;`,
  `typedef ns::zed zed;`, fmt's `using named_arg = detail::named_arg<T, Char>`; `using Any =
  ::absl::any` is a node). The declared names come before a class the typedef's type declares;
  that class then keeps the typedef's label and line and gains `_callable`/`_callable_class`
  (jq: `typedef struct yy_buffer_state *YY_BUFFER_STATE;` is node `YY_BUFFER_STATE` with both
  flags); after a class of the same id the typedef changes nothing (`using r = R;`). Type names
  and calls bind to these nodes like classes without members (§25.3, §25.4).
- Namespaces, `using` declarations and directives, macros and comments give nothing (no
  rationale).
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
- **Template parameters (0.9.97)** give no reference anywhere in their template (parameters,
  return types, fields, template arguments, out-of-line `T Box<T>::get()`), also when one shadows
  a class (`template <class Point> Point same(Point p)`); 0.9.86 made stubs `t`, `n`. A `T` outside
  any template is a stub as before, and a base `struct D : T` still `inherits` the stub `T`.
- **GPU qualifiers (0.9.97).** In `.cu` files `__global__`, `__device__`, `__host__`,
  `__shared__`, `__constant__`, `__managed__`, `__forceinline__`, `__noinline__`,
  `__launch_bounds__` and `__grid_constant__`, and in `.metal` files `kernel`, `vertex`,
  `fragment`, `device`, `constant`, `thread`, `threadgroup`, `threadgroup_imageblock`, `ray_data`
  and `object_data`, which the grammar reads as type names, give no reference (probed one word per
  file; `__restrict__`, `__align__`, `visible`, `mesh`, OpenCL's `__kernel`/`__global` and
  macros like `MYAPI` are unchanged). The real type behind them is lost with them
  (`fragment half4 fs(thread Frag f)` references nothing).

### 25.3 Type targets and bases

- **Rule.** A type name binds in its file to whatever node has the id a file-level definition of
  that name would have (`file_slug(name)`), a class but also a function or a field (fmt binds base
  `testing::Test` to the macro function `TEST()`); otherwise to a per-file stub.
- A base names its stub after the base text minus its first qualifier: `Tmpl<int>` → `Tmpl`,
  `::testing::Test` → `testing::Test`, `detail::buffer<char>` → `buffer<char>` (id
  `buffer_char`); the in-file lookup uses the innermost name (`buffer`) and never the class
  itself (`struct is_contiguous : detail::is_contiguous<T>` → stub `is_contiguous<T>`).
- Stubs bind across files to the only definition labelled like them (classes, typedefs and
  aliases, enumerators, fields): `Rect` with a class `Rect` and an enumerator `Rect` stays a stub.
- A type name binds in its file to a typedef or alias declared anywhere in it, later ones too
  (like classes).

### 25.4 Calls

In-file binding is §24.5, with two additions:

- In a method defined in a class body, `f()` and `this->f()` first take the class's own member `f`.
- Template calls `f<T>(...)` and `obj.template f<T>()` bind nothing.

Qualified calls `A::...::f(...)` (scope = outermost qualifier, name = the rest):

1. scope names exactly one class, typedef or alias (not templated: `formatter<std::tm,
   Char>::parse` skips this) → its member (method, field or enumerator) `f`, else the only
   out-of-line definition `A::f()`, EXTRACTED 1.0; else the class itself is `references`d with
   context `call`, EXTRACTED 1.0 (`Widget::missing()`, `Gadget::draw()` on an alias); a method
   never calls itself this way;
2. otherwise the file's node answering to the rest of the name (`ns::helper()` → `helper()`,
   `std::f()` → a local `f()`, `internal::FilePath::MakeFileName` → the out-of-line
   `FilePath::MakeFileName()`), EXTRACTED; never across files.

Typed calls: `x.m()` / `x->m()` with no `m` in the file use `x`'s type, taken from the file's
**first** local declaration of `x` (any function, earlier or later) that has a single declarator
and a capitalised type name (`Shape s;`, `Shape *p = ...;`, `const Shape &r = ...;`,
`geo::Shape g;` → `Shape`, condition declarations; not parameters, `auto`, templates
`Box<int> b`, `Shape a, b;`, range-for). The class's member `m` binds INFERRED 0.85; a class (or
typedef, alias, enum) without one is `references`d with context `call`, INFERRED 0.85; an alias
is not followed to its type (`Gadget q; q.draw()` references `Gadget`). No inheritance walk,
out-of-line definitions are not members here.

`metadata.unresolved_calls` (§6), `{"callee","receiver_type","lang":"cpp","line","file"}` in
source order, once per (callee, type) and caller, the first 64 of a caller only: typed calls whose
type names no class, typedef or alias (`BiggestConvertible parsed; parsed.push_back(...)`,
`T x; x.push_back(1)` in a template), and qualified calls whose scope starts with a capital and
names none (`L::lm()`, `TypeParameterizedTest<...>::Register`). `file` (the caller's path) is new
in 0.9.97; C++ has no receiver denylist (`String`, `NSString`, `Color` are recorded).

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
- **An include landing on the file's own definition.** gtest.h's `#include "absl/types/any.h"`
  lands on the alias `Any` two lines below it (`test_gtest_gtest_gtest_any`), not on a stub `any`;
  dijkstral's step 2 only looks at the file's stubs (one fmt link).
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
  `module -defines-> file`: of two links of one relation the smaller source survives (§8.2), as
  dijkstral's fold now does.
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

- Golden: nodes 44/44, links 84/84 with nothing allowed (0.9.97 dropped the Q8 `.mount()`
  self-loop of 0.9.86), fields included (`_ts_fields` on `TodoStore`); communities identical.
- TS behaviours seen through Vue and not reproduced, as for `.ts`: Q8 (`this.other()` inside an
  options-API method binds to the flattened `other()`), and Graphify emits uninitialised
  `export let x` late without a `contains` link (dijkstral links it).

### 38.6 Analysis

`dijkstral cluster-only` once asked one more bridge question than Graphify on the Vue golden
(`formatTitle()`, ranked fourth by betweenness). Bridges come from the top **three** candidates
(§17.5), not five; same-file neighbours count as usual. The sidecars now match byte for byte.

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
  phantom code node plus an `imports_from` link without context, line or weight, beside the
  normal link to `ref_lib`. In 0.9.97 a bare specifier's phantom is one node per **package**
  (the first segment, or `@scope/name`): id `slug(package)`, label and `source_file` the package,
  `confidence` and `type: "module"`, no location (`$lib/Cart.svelte` and `$lib/Badge.svelte` →
  one `lib` labelled `$lib`; `$app/stores` → `app`; `@scope/pkg-types/sub/Thing.svelte` →
  `scope_pkg_types`). 0.9.86 named it after the last segment (`cart_svelte`, label the
  specifier, no `type`). A missing relative path still gives `slug(resolved path without
  extension)`, labelled with the specifier, without `type` (probe `web-sv1`). Some of these
  links disappear in the clustered build (`pkg/C.svelte`). Dijkstral keeps the external only.

Golden: nodes 27/27 and links 45/45 after the phantom node and link (SV2); NMI 1.0.

## 40. Astro (2026-10-15)

Fixture `tests/fixtures/astro/` (pages, a layout, components with frontmatter, a client
`<script>`, a React island `.tsx`). The frontmatter body (the file's first line is `---`, up to the
next `---` line) and every `<script>` block form one TypeScript module; a `---` later in the
markup is not a fence. Everything else is §39: no import-scoped pass, no default-import links,
no heritage or type references, no `type_only`, no props without a value, no rationale, and the
SV2 phantoms for unresolved imports (`@/H.astro` → `h_astro`, `comp-lib/Comp.astro` → `comp_lib`
in 0.9.97). INFERRED member calls involving an
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

**Calls** (Rule): callers are functions and methods (a nested function owns its own calls; calls
in script blocks belong to the enclosing function). **0.9.97:** top-level commands, script blocks
included (Pester `Describe`/`It`), are the file node's: a command naming no function of the file
links the file to the function across files as below (`tests_shop_tests -calls->
modules_shop_shop_new_shoporder`); one naming a function of the file shows nothing (it would fold
into `contains`). 0.9.86 gave top-level code no caller. In the file, a
command name binds case-insensitively to a function of the file, `$this.M()` to the class's method
and `[Class]::M()` to a method of a class of the file, EXTRACTED without context, once per pair,
grouped by caller in definition order. Other command names go to the function **or method** of
exactly that (case-sensitive) name nearest to the caller's file (§19.5), INFERRED 0.85 with
context `call`; `[Other]::M()` across files, `$x.M()`, `& F` and `[T]::new()` give nothing.

### 41.3 Parity

- Golden (0.9.97): nodes 43/43, links 64/64, identical communities, analysis and report.
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
## 33. Zig

Fixture `tests/fixtures/zig/`; goldens `zig.*` (§1, §17.6). Extension `.zig`; tree-sitter-zig 1.1.2
(Graphify's version). Rules were isolated with probe repositories and checked on zls, libxev,
http.zig and zig-clap (identical node and link sets) and on the Graphify repository's
`tests/fixtures/sample.zig`. Graphify does extract Zig symbols: §14.5's "yields no symbols" holds
only for a stub without declarations. Implementation: `crates/core/src/{extract,resolve}/zig.rs`.

### 33.1 Nodes

| Construct | id | label | extra |
|---|---|---|---|
| file | file id | §5.1 | — |
| `const`/`var` holding a `struct`, `enum`, `union` (any layout keyword) or error set | `file_slug(Name)`, **flat** (also when nested in another container) | `Name` | — |
| function (`pub`, `extern`, `export`, `inline` alike) | `file_slug(name)` | `name()` | — |
| function of a named container | `container_slug(name)` | `.name()` | — |
| enum field, tagged-union field, error-set name | `container_slug(NAME)` | `NAME` | — |
| `@import` target with no node or file (§33.3) | `slug(name)` | id | external |

- **Rule.** Declarations are read outside function bodies: at file level, in container bodies,
  and anywhere inside `test` and `comptime` blocks (nested `if` blocks included), where
  containers are file-level containers. Containers declared inside a function (generic
  `return struct {...}`, local `const L = struct {...}`) give nothing, nor do their functions.
- The container may sit in the type position (`const t: struct { a: i32 } = .{}` gives `t`),
  but a declaration that also holds a field chain gives no container: `var s: enum { a } = .a;`
  (an enum literal is a field expression), `const P = @import("x.zig").P;`.
- Functions of an anonymous container in a `test` block's **statement**
  (`loop.timer(..., (struct { fn callback(...) ... }).callback)`) are file-level functions
  `callback()`; same-named ones merge (§8.1). An anonymous container inside a variable
  declaration's value (`_ = run(struct { fn cb() ... })`, also at file level) is not walked.
- A struct (or plain union) field whose type contains an inline container hands that
  container's members to the struct: `kind: enum { file, dir }` gives cases `S_file`, `S_dir` of
  `S`; `opt: ?enum { maybe }` too; an inline `struct { flag: enum { on, off }, fn im() }` gives
  cases `on`, `off` and method `.im()` of `S`. Fields of an enum or tagged union are cases and
  their types are not walked (`slow: struct { depth: enum {...} }` inside a `union(enum)` adds
  nothing).
- A union is tagged when `(` follows `union` (`union(enum)`, `union(enum(u8))`, `union(Tag)`);
  plain and `extern` unions give no cases. Struct fields never give nodes.
- Not emitted: `opaque` declarations (with their functions), type aliases (`const T = u32`),
  merged error sets (`error{A} || error{B}`), `usingnamespace`, `test` and `comptime` blocks
  themselves, globals and constants, comments (`// NOTE:`, `///` doc comments: no rationale),
  type references. No node carries `_callable`.
- Nested containers are flat (`const Outer = struct { const Inner = ... }` → `file_inner`,
  contained by the file); two same-named nested containers of one file merge (§8.1, ZG1).

### 33.2 Edges

| relation | source → target | context | confidence | location |
|---|---|---|---|---|
| `contains` | file → container / function | — | EXTRACTED | declaration line |
| `method` | container → function | — | EXTRACTED | function line |
| `case_of` | container → member | — | EXTRACTED | member line |
| `imports_from` | file → file / node / external (§33.3) | — | EXTRACTED | declaration line |
| `calls` | function → function, in the file | — | EXTRACTED, no score | call line |
| `calls` | function → definition, across files (§33.4) | `call` | EXTRACTED 1.0 / INFERRED 0.85 | call line |

- The non-exhaustive marker `_` slugs to nothing, so it is a `case_of` self-loop on the enum.
  **Rule (build).** When a node has two such self-loops (two same-named enums with `_` in one
  file), the build keeps the **first**: a repeated self-loop always does (§8.2).

### 33.3 Imports

- **Rule.** `@import("p")` counts when it is a direct child of a variable declaration, alone or
  as the innermost object of a field chain (`@import("std").mem`, `var w: i32 =
  @import("v.zig").val`), outside function bodies (in `test`/`comptime` blocks too). Not inside
  call arguments, conditionals, `usingnamespace`, functions; `@cImport`/`@embedFile` give
  nothing. Every occurrence is an edge, so `@import("std")` twice folds to the later line.
- The target id is `slug` of the first dot-separated part of the path's last segment:
  `"std"` → `std`, `"sub/deep.zig"` → `deep`, `"x.y.zig"` → `x`, `"build.zig.zon"` → `build`,
  `"Upper.zig"` → `upper`. The edge lands on, in order:
  1. any node with that id (a root-level file: `"util.zig"` → file `util`; `"q_b.zig"` →
     file `q/b.zig`, whose id is `q_b`);
  2. the only file whose stem (up to the last dot) slugs to the id, **in any language**
     (§8.1; `@import("xev")` → `include/xev.h`);
  3. the external node of that id (also when two files share the stem).
- An import that lands on the importing file itself is dropped by the build (raw output keeps
  the self-loop).

### 33.4 Calls

- **In the file.** Calls are read from function bodies, including the fields and constants of
  containers declared there (`return struct { items: Buffer(T), const K = if (pick()) ... }`
  calls `Buffer()` and `pick()` from the generic function) but not the bodies of their
  functions. The callee is the identifier or the last member of a field chain (`a.b.f()` →
  `f`); builtins (`@as`), calls through other expressions and calls at file level or directly in
  `test` blocks give nothing.
- **Rule.** A call binds to the **first** function or method of that name in declaration order,
  whatever the receiver (`x.run()` and `B.run`'s own `go()` both bind to `A.run`/`A.go` when
  `A` comes first). A function whose id an earlier node already took (`fn sentinel` after
  `const Sentinel`, `fn help` after `const Help`) answers to no call, but its own calls count,
  from that shared id. A call that binds to its caller is dropped (no recursion self-loops, also
  `self.run()` in `run`). One edge per (caller, callee), first line kept. Containers, members
  and error names never bind in the file.
- **Across files.** A **bare** call that binds nothing in its file (member calls never do)
  binds to the Zig definitions answering to that name: functions, methods (`norm()` → `.norm()`),
  containers, enum and error members, the caller's own file included (`Bar()` on a struct
  `Bar`, `red()` on an enum member). Only the node each id keeps counts (`Sentinel`, not the
  merged `sentinel()`).
  1. Exactly one candidate in a file the caller's file imports **directly** (§33.3 step 1:
     the import id is that file's id) → EXTRACTED 1.0.
  2. Otherwise the nearest candidate by path: most shared leading directories, a tie binding
     nothing, depth below the common directory ignored (`src/x/f.zig` and `src/y/z/f.zig` tie
     from `src/a.zig`); a candidate sharing no directory binds only from the same directory
     (two root files) or when it is the only one → INFERRED 0.85.

### 33.5 Parity

- Fixture: node, link, community, analysis and report goldens all exact.
- Probes and real repositories (`graphify extract --code-only` on scratch copies, `.zig` only):
  zls (1,998 nodes, 3,743 links), http.zig (761, 1,039), libxev (1,212, 1,606), zig-clap (110,
  177) identical node and link sets and fields, except one zig-clap `method` link (ZG3).
- Graphify corpus (`--ext .zig`): 12 of 14 nodes and 10 of 15 links. `Point` and `Shape` are
  renamed by Graphify because `sample.jl`, `sample.ps1` and `sample.f90` emit the same ids
  (§4.5); dijkstral does not index `.jl`/`.ps1`. The `std` import keeps L1 (ZG2).

### 33.6 Quirks (Zig)

- **ZG1** Flat container ids: same-named nested containers of one file merge, and so do their
  same-named members; a function and a container whose names slug alike (`Sentinel`/`sentinel`,
  `add`/`add_`) share one node.
- **ZG2** When the build performs any declaration/definition merge (§25.5, e.g. a C++
  `Logger.h`/`Logger.cpp` pair anywhere in the repository), every duplicate
  (source, target, relation) keeps its **first** occurrence instead of the last: the Graphify
  repository's `sample.zig` keeps `imports_from std` at L1 where a Zig-only build has L2.
  Not reproduced (a build-wide rule, not Zig's).
- **ZG3** A context-less in-file `calls` folded into a `method` link on the same pair leaves the
  link without context: a structural link takes the call's own context (§8.2), which Zig calls
  lack. Reproduced since 2026-10-15. Seen once in zig-clap (`fn help` sharing the id of `Help`,
  calling `Help.description`).
- **ZG4** In-file binding by first definition ignores receivers and the caller's own container;
  imports never bind member calls.

---

## 34. Objective-C

Fixture `tests/fixtures/objc/`; goldens `objc.*` (§1, §17.6). Extensions `.m`, `.mm` and
Objective-C headers (§34.1); tree-sitter-objc 3.0.2 (Graphify's version), also for `.mm`, so the
C++ of Objective-C++ gives nothing. Rules were isolated with probe repositories; the corpus check is
AFNetworking. Implementation: `crates/core/src/{extract,resolve}/objc.rs`.

### 34.1 Which language a `.h` is

- **Rule.** A `.h` is Objective-C when its bytes contain `@interface`, `@implementation`,
  `@protocol`, `#import` or `@import` anywhere (plain substrings, comments included:
  `// #import` and `x@interface` count; `# import`, `@INTERFACE`, `@class`, `@end`, `@property`,
  `@"..."`, `NSObject` and `#include <Foundation/...>` do not). This test runs before C++'s (§24.1):
  a header with `@interface` and `class ` in a comment is Objective-C. Every other `.h` is C or C++
  by §24.1, also beside a `.m` of the same stem.
- `NS_ASSUME_NONNULL_BEGIN`/`END` and `NS_HEADER_AUDIT_BEGIN`/`END(...)` are removed before
  parsing (lines unchanged; tree-sitter-objc misreads the rest of the file otherwise). Other macros
  (`CF_ASSUME_NONNULL_BEGIN`, `NS_SWIFT_UI_ACTOR`, `CF_EXTERN_C_BEGIN`, `__BEGIN_DECLS`, custom
  ones) are not, and the parse breaks wherever they break it.

### 34.2 Nodes

| Construct | id | label | extra |
|---|---|---|---|
| `@interface`/`@implementation X`, class extension `X ()`, category `X (C)` | `file_slug(X)`, one node for all of them in a file (first wins) | `X` | `_callable`, `_callable_class` |
| the same, as a category or extension in a file whose stem is exactly `Before+After` | `slug(dir/Before)_slug(X)` (0.9.86 prefixed the absolute scan root, OC1) | `X` | `_callable`, `_callable_class` |
| `@protocol P` | `file_slug(P)` | `<P>` | `_callable`, `_callable_class` |
| method `- (T)a:(U)x b:(V)y` / `+ ...` | `owner_slug(selector)` | `-ab` / `+ab` | `_callable`; `metadata.unresolved_calls` (§34.5) |
| method outside any class (error recovery) | `slug(path with extension)_slug(selector)` | as above | `_callable` |
| `typedef enum { ... } E;`, `enum E { ... }` with a body | `file_slug(E)` | `E` | — |
| `typedef NS_ENUM(T, E) { ... };`, `NS_OPTIONS`, `NS_CLOSED_ENUM` (0.9.97) | `file_slug(E)`, after every other node of the file | `E` | — |
| enumerator | `enum_slug(NAME)` | `NAME` | — |
| type stub (superclass, protocol, property type, `[X alloc]`) | `slug(name)`, per file | name | stub |

- **Selector**: the method's direct identifiers concatenated (keywords without colons, plus any
  trailing macro: `-fooNS_REQUIRES_SUPER`, `-initWithXNS_DESIGNATED_INITIALIZER`); attributes in
  their own node (`API_AVAILABLE(...)`, `__attribute__`) are not part of it. A broken parse can
  fold body text into it (`-deallocdownloadProgressremoveObserverforKeyPathfractionCompleted`).
- **Bodies.** An `@interface` contributes only its direct members: methods, properties and the
  instance-variable block; anything under `#if` in an interface is ignored. An `@implementation`
  and a `@protocol` contribute their methods at any depth (`#if` included, also `@optional` /
  `@required`). Protocol properties give nothing.
- `_callable`/`_callable_class` (classes, categories, extensions, protocols) and `_callable`
  (methods) are new in 0.9.97; enums and enumerators get neither.
- **Macro enums (0.9.97).** The parse loses `typedef NS_ENUM(NSInteger, Mode) { ... };`; Graphify
  reads it from the text after the walk, so the enum, its `contains` edge and its enumerators
  (`case_of`, each at its own line; the enum at the `typedef` line) come after every other node of
  the file. Matched: `typedef NS_ENUM`, `NS_OPTIONS` or `NS_CLOSED_ENUM` `(type, Name)` then `{`
  (spacing and newlines free, `unsigned long` as the type), a body closed by `}` and `;`, each
  enumerator a name with an optional `= value` (comments, `(1 << 2)`, `MAX(1, 2)`, `-1`, `A | B`
  allowed; an empty body gives the enum alone). Not matched, nothing emitted: `NS_ERROR_ENUM`,
  `CF_ENUM`, `CF_OPTIONS`, custom macros, `NS_ENUM` without `typedef`, a macro between `)` and `{`
  (`NS_SWIFT_NAME(...)`) or between `}` and `;` (`API_AVAILABLE(...)`), and a body with any
  enumerator carrying an attribute (`A API_AVAILABLE(ios(13)) = 4`, `B __deprecated`).
- Not emitted: C functions and anything in their bodies (`main`), structs, unions, typedefs of
  non-enums, the macro enums above that do not match, `@class`, globals, macros, properties,
  instance variables, comments (no rationale: `NOTE:`/`WHY:` are ignored), and the C++ of `.mm`
  files.
- A method label `-Bar` beside `-bar` merges into the first (same slug, §8.1).

### 34.3 Edges

| relation | source → target | context | location |
|---|---|---|---|
| `contains` | file → class (every `@interface`; an `@implementation` only when it creates the node), protocol, enum | — | declaration line |
| `method` | class or protocol → method, for every declaration and definition; file → method outside a class | — | method line |
| `inherits` | class → superclass | — | class line |
| `implements` | class or category → adopted protocol; protocol → protocol it extends | — | declaration line |
| `references` | class → every type identifier of a property's type (`NSSet <NSData *>` → both, `Sub <P> *` → both; not `id<P>`, not block parameters) | `field` | property line |
| `case_of` | enum → enumerator | — | enumerator line |
| `references` | method → `X` for `[X alloc]` (also `[self alloc]` → stub `self`) | `type` | message line |
| `calls` | method → in-file node (§34.4) | `call` | message line |
| `accesses` | method → method of its own class named exactly like `x.name` (any receiver) | — | expression line |
| `imports` | file → §34.6 | `import` | directive line |
| `calls` / `references` | method → method / class (§34.5) | `call` | message line |

Type names bind to the file's class, protocol or enum of that name (also one declared later in the
file), else to a per-file stub. Stubs then bind to the only Objective-C class, protocol or enum with
that name in the repository (protocols by `P`, not `<P>`); an ambiguous name stays a stub (two
classes named `Foo`, a class and a category elsewhere).

Fold behaviour once taken for Objective-C differences, now the shared rules of §8.2:

- A property `references` on a pair that also has `inherits` or `implements` (`@interface A : B`
  with `@property B *b`) leaves no `context` on the survivor, in either order (both are
  structural).
- A repeated self-loop keeps the first (`@property Impl *a` then `NSArray<Impl *> *b` inside
  `Impl`).

### 34.4 Calls inside the file

- `[r sel ...]` in a method body (blocks included) binds to **every** node of the file whose id
  ends with the lower-cased selector, except the file node, the caller itself and stubs first
  created by a `[X alloc]` in a body: `[self bar]` calls `-bar`, `-fooBar`, `-foo_bar`, `-setBar`
  and `J`'s `-jbar` (OC2); `[self type]` calls a stub `LaterType`; `[self far]` calls the stub
  `Far`. The receiver is ignored.
- `@selector(name)` calls the file's methods whose selector is exactly `name`.
- One edge per (caller, target) across `calls` and `accesses`, the first kept (`self.r5; [self r5]`
  gives `accesses`). `references` from `[X alloc]` are separate.
- Body edges follow every structural edge of the file, in method and source order. Graphify's
  order among the several targets of one message varies between runs (hash order); only raw output
  shows it.

### 34.5 Messages across files

Each message with a typed receiver, in source order, after the in-file pass:

| receiver | class | confidence |
|---|---|---|
| `self`, `super` | the enclosing `@implementation`'s class (`super` too) | EXTRACTED 1.0 |
| an identifier declared `T *x` anywhere in the file (first declaration wins, other methods and classes included; not parameters, `for (T *x in ...)`, block parameters or `id`) | `T` | INFERRED 0.85 |
| another capitalised identifier | itself (case-insensitive: `[FOO base]` finds `Foo`) | EXTRACTED 1.0 |
| another lower-case identifier | the instance variable, else the property, of that name on the class or a superclass, any file | INFERRED 0.85 |
| `self.prop` | the property's type on the class or a superclass, any file; a generic or protocol-qualified type (`T<P> *`) gives nothing | INFERRED 0.85 |

"The class" is the enclosing class itself (by id: its header, implementation and extensions
together, not a same-named class of another file: the iOS and macOS example `AppDelegate`s of
AFNetworking keep their own `window`), then its superclass by name when exactly one class has that
name. A declaration with two type names, as `@property IBOutlet UIImageView *v` reads, has no
type.

The class must be the only Objective-C class with that name. If it has a method of that selector
(`class_slug(selector)`: instance or class method, no inheritance walk), the caller `calls` it,
else it `references` the class with context `call`. A pair the file already links (in-file call,
access or `alloc` reference, after stub binding) is skipped, the first edge of a pair wins, and a
method never calls itself.

**`metadata.unresolved_calls` (0.9.97).** A message whose receiver type no Objective-C class has
(not an ambiguous name: two classes `Dup` record nothing) is recorded on the caller as
`{"callee": selector, "receiver_type": T, "lang": "objc", "line", "file"}` (the selector's
keywords concatenated, `foobar` for `foo:bar:`), in pre-order source order (`[NSDictionary
outer:[NSSet inner]]` records `outer` then `inner`; `[[NSMutableSet alloc] init...]` records
`alloc`), once per (selector, type), the first 64 of a caller only. `self`/`super` never record.
Receiver types never recorded (observed): the JavaScript and Python builtins of §24.5's list that
start with a capital plus `Intl` and `globalThis`; `Bool`, `Character`, `Double`, `Float`, `Int`,
`Int8`–`Int64`, `UInt`, `UInt8`–`UInt64`, `AnyObject`, `Data`, `Decimal`, `UUID`, `URLRequest`,
`URLComponents`, `URLSession`, `Bundle`, `Calendar`, `Locale`, `TimeZone`, `DateFormatter`,
`ISO8601DateFormatter`, `NumberFormatter`, `JSONDecoder`, `JSONEncoder`, `DispatchQueue`,
`DispatchGroup`, `OperationQueue`, `RunLoop`, `FileManager`, `NotificationCenter`, `UserDefaults`,
`IndexPath`, `IndexSet`, `Codable`, `Decodable`, `Encodable`, `Equatable`, `Hashable`,
`Comparable`, `Identifiable`, `Sendable`, `CaseIterable`, `CustomStringConvertible`,
`RawRepresentable`, `View`, `Color`, `Font`, and `NSString`, `NSObject`, `NSError`, `NSLock`,
`NSAttributedString` (other Foundation and UIKit classes, `NSMutableString` and `NSRecursiveLock`
included, are recorded). The list is in `resolve/objc.rs`; names beyond the ~250 probed may be
missing.

### 34.6 Imports

`#import`/`#include "p"` names the file at `p` relative to the importing file when it is indexed
(its id before collision renaming), else (0.9.97) the file the search of §24.4 finds, with the
same `metadata {"raw_include", "include_kind": "quoted"}` on every quoted import that is not
relative; else, like `<p>` and `@import M`, `slug(stem of p)`. Within a file, every import of the
same key (the relative file's id, else `slug(stem of p)`, whether or not a search follows) lands
where the first one did: `<Lib/Core/Session.h>` then `"Session.h"` both land on the external
`session`, the other order on the header the search found. Once any definition merges (§34.7)
imports keep the first edge of a pair too. Otherwise the id lands, in order, on:

1. a node with exactly that id (an unrenamed file, stub, class, external);
2. the node with that id of the first header (`.h`, by path) that emitted it before collision
   renaming: the header of a header/implementation pair (`#import "Shape.h"` from `Shape.m` →
   `shape_h_shape`), a header's renamed type stub (`#import "AFURLSessionManager.h"` from another
   directory → `afnetworking_afhttpsessionmanager_h_afurlsessionmanager`), followed through stub
   binding (`#import "Helper.h"` → class `Helper` through `Base.h`'s stub);
3. for a non-relative path only, the importing file's own renamed node with that id (a test
   file's stub of the imported class);
4. the only file whose stem slugs to that id (`<AFNetworking/AFCompatibilityMacros.h>` → that C
   header, §8.1);
5. an external node.

An import whose target is another language's node is dropped: Swift's module nodes
(`import Foundation`, `import UIKit`) swallow every Objective-C `#import <Foundation/...>`.

### 34.7 Header and implementation merge

A definition id (class, protocol, method, enum, enumerator) emitted by an Objective-C header and by
the `.m`/`.mm` with the same directory and stem is not renamed apart (§4.5): the header's node
stays and gains `definition_file` and `definition_location` (the implementation's first node of
that id: a class extension's line before the `@implementation`'s). File nodes and stubs the two
share are renamed as usual.

- Edges the implementation adds from a merged definition are dropped when the header, or an earlier
  edge of the implementation, has the same source, relation and target as written (before
  renaming): the header's `method` lines survive, and a class extension's property typed like a
  header property leaves its own renamed stub isolated.
- **Once any definition merges**, every Objective-C edge keeps the first of its (source, target,
  relation) instead of the last, and Objective-C self-loops are dropped; a repository without a
  merge keeps the last (an `@interface` method line loses to its `@implementation` line). Other
  languages are not affected (Python keeps the last).
- An Objective-C stub named like a Swift supertype stub (`UIApplicationDelegate`,
  `UIViewController` in a mixed iOS project) is renamed `slug(path)_id`, as if the Swift file had
  emitted it too.

### 34.8 Quirks (Objective-C)

- **OC1** (fixed in 0.9.97) In a file whose stem is exactly `Before+After`
  (`UIButton+AFNetworking.h`), categories and class extensions get the id
  `slug(dir/Before)_slug(Class)`; 0.9.86 prefixed the absolute scan root (`/private/tmp/...`
  resolved) and dijkstral never did. Stems with no or two `+` (`A+B+C.h`, `+L.h`, `T+.h`) use the
  file id.
- **OC2** In-file messages bind by id suffix, regardless of receiver, so one message fans out to
  every method, class, protocol or structural stub whose id ends like its selector.
- **OC3** `.h` classification is a substring test that also reads comments (§24.1 is the C/C++
  analogue).
- **OC4** A method that sends to the same class through a typed variable and through its name
  (`[serializer isKindOfClass:[AFHTTPResponseSerializer class]]`) keeps whichever edge Graphify
  processes first, and that order changes with unrelated files in the repository (AFNetworking:
  EXTRACTED with its tvOS Swift example present, INFERRED without). Dijkstral keeps source order.
- **OC5** Methods whose parse lands outside any class become file-owned with ids that keep the
  file's extension (`afnetworking_afurlsessionmanager_m_dealloc`); in AFURLSessionManager.m the
  whole file is one `ERROR` node, so 73 methods hang off the file with body text in their names.

### 34.9 Parity (AFNetworking, Graphify's frozen reference, `.m .h .mm`)

Against 0.9.97 (2026-10-29): nodes and links at 100% recall and precision, every node field equal
(323 methods with `unresolved_calls` included), one link field differing (OC4). Against 0.9.86 the
51 category nodes differed by the OC1 root prefix only. Two Graphify runs on fresh copies are
identical, so the reference is stable. The `objc` golden matches at 1.0 with exact
communities; its extract-time surprise order is not compared (§17.7). Two analysis differences
seen on Graphify's own graphs turned out to be shared rules (§17.4–§17.5): surprise ties follow
the sorted-link adjacency (no `definition_file` rule is needed), and bridges come from the top
three betweenness candidates.

### 34.10 Open (Objective-C)

- Whether the keep-first and self-loop rules of §34.7 are triggered by C++ declaration merges
  (§25.5) too; the Zig probes saw keep-first across languages after a C++ merge (§33).
- `@import M.Sub` beyond the stem rule; properties of protocols with a class type; nested `#if` in
  an `@interface` that contains its `@end`.

---

## 35. Fortran

Fixture `tests/fixtures/fortran/`; goldens `fortran.*` (raw, clustered, analysis and report goldens
of §17.6). tree-sitter-fortran 0.6.0 (Graphify's version). Rules were isolated with probe
repositories through `graphify extract --code-only`, and checked on the graphify corpus's
`tests/fixtures/sample.f90` and on the 88 Fortran test sources of numpy's `f2py` (free and fixed
form). Implementation: `crates/core/src/{extract,resolve}/fortran.rs`.

### 35.1 Which files

- **Rule.** `.f90 .f95 .f03 .f08 .f` (case-insensitive: `.F90`, `.F`) are Fortran; `.for`, `.f77`,
  `.fpp`, `.f18`, `.ftn`, `.pf` are not classified. Fixed-form `.f` goes through the same grammar.
- No rationale nodes: `! NOTE:` / `! WHY:` comments are ignored. No `_callable` on any node.

### 35.2 Nodes

Every name is lower-cased (`module Geometry` → label `geometry`); ids are flat, whatever the
nesting.

| Construct | id | label |
|---|---|---|
| `module m` | `file_slug(m)` | `m` |
| `program p` | `file_slug(p)` | `p` (no parentheses) |
| `subroutine s` / `function f` (any prefix: `pure`, `recursive`, `real`, `type(T)`, `module`), internal procedures, procedures in unnamed or `abstract` interface blocks | `file_slug(name)` | `name()` |
| named interface `interface g` | `file_slug(g)` | `g()` |
| derived type `type :: T` (also `extends(...)`) | `file_slug(t)` | `t` |
| `use m` (any form: `use, intrinsic ::`, `only:`) | `slug(m)` — **not** file-prefixed | `m`; `source_file` and `source_location` of the `use` statement, not a stub |
| type stub | `slug(t)`, per file | `t` |

- Not emitted: `submodule` (nothing inside it either), operator and assignment interfaces,
  bodies of named interfaces, components, parameters and variables, `generic`/`final` bindings,
  `block data`.
- A `use` node is per file; two files using the same module both emit `slug(m)` and are renamed
  apart by §4.5 (`app_main_f90_geometry`, `src_shapes_f90_geometry`). It also collides with a
  file whose id is `slug(m)` (`use utils` beside `utils.f90` renames both), and it never binds to
  the module's own node (quirk **FO1**).
- Preprocessor branches (`#ifdef`/`#else`) are walked like their parent; a procedure defined in
  both branches is one node at the first line (§8.1).

### 35.3 Edges

| relation | source → target | context | location |
|---|---|---|---|
| `defines` | file → module / program / free procedure; module, program or host procedure → type, procedure, interface | — | definition line |
| `imports` | enclosing module, program or procedure → `use` node | `use` | statement line |
| `references` | procedure → type, for a `type(T)`/`class(T)` declaration of a dummy argument | `parameter_type` | declaration line |
| `references` | the same, for the result variable (`result(r)`, else the function name) | `return_type` | declaration line |
| `method` | type → implementation of a type-bound `procedure` | `type_bound_procedure` | binding line |
| `dispatches_to` | named interface → every procedure its `procedure` / `module procedure` statements list | `generic_interface` | statement line |
| `calls` | procedure or program → procedure, type or module of the same file | `call` | call line |

- A procedure emits `defines`, then its `references` (declarations in order, one edge per plain
  identifier declarator: `a(:)`, `b(10)`, `c => null()` and `class(*)` give nothing; locals give
  nothing), then its other children in source order (`use`, interface blocks, internal
  procedures). Modules and programs walk their children in source order. Type-bound `method`
  edges follow the walk, then `dispatches_to`, then calls.
- A type-bound `procedure` statement uses only its **first** declarator (`procedure :: a, b` →
  `a`; `procedure :: c => cc, d` → `cc`); a named interface's statement lists every name.
- Type names bind to a node **emitted earlier in the file** with id `file_slug(T)` (a type, but
  also the enclosing module); a reference to the procedure itself gives no edge; a type defined
  later in the file, or elsewhere, goes through a per-file stub (§35.4). Type-bound and generic
  targets bind to any node of the file (also defined later), else a stub.
- `function_result` prefixes (`type(T) function f()`) give no reference.

### 35.4 Cross-file binding

Nothing else crosses files: function references, `use` and type-bound targets stay in their
file; only `call` statements and type names go through stubs.

- **Rule.** A per-file stub binds to the only node, of **any language**, labelled exactly `t`,
  else the only function labelled `t()` — a Fortran module, type, procedure or `use` node, a
  Python class or function, a JSON key (`type(name)` → `package_name`). Methods (`.m()`) do not
  count; labels compare case-sensitively, so the lower-cased stub `widget` never takes a Python
  class `Widget`. An ambiguous name (`label` defined in two modules) or an unknown one stays a
  stub.

### 35.5 Calls

- `call s(...)` targets `file_slug(s)` when the file defines `s`; otherwise it goes through the
  per-file stub `slug(s)` of §35.4 (0.9.97; 0.9.86 left it dangling): `call ext1()` binds to the
  only function `ext1()` of any file and language, a module (`call geo()`) or a type of that name
  binds too, and an ambiguous or unknown name keeps the stub (`call missing_routine(n)` →
  `missing_routine`, renamed per file by §4.5 when several files call it). `call x%m()` gives
  nothing.
- A function reference or array element `f(...)` (also inside `print`, conditions, keyword
  arguments) is a call only when the file has a node `file_slug(f)`: a procedure, an interface, a
  type (`Point(1.0, 2.0)` calls the type) or a module. A function naming itself in an expression
  (`f = n * factorial(n - 1)`) is not a call; `call s` inside `s` keeps its self-loop.
- The caller is the innermost procedure or the program; calls at module level (parameter
  initializers) give nothing. Every call site gives an edge (duplicates fold, the later line
  wins).

### 35.6 Build-stage behaviour of `defines`

**Rule (observed).** `defines` does not fold like `contains` (§8.2) against `calls` and
`references` on the same pair. The probes fit: fold each direction first (a `defines` keeps
itself and takes a same-direction call's `context`; `calls` beats `references`; otherwise the
later edge wins), then fold the edge whose source id is **larger** over the other, keeping the
earlier edge's `context` when the survivor has none — except that a `defines` from the smaller
source ignores a reversed `references`.

| Edges on the pair (in order) | Survivor |
|---|---|
| `aa -defines-> bb`; `bb -calls-> aa` | `bb -calls-> aa` |
| `zz -defines-> bb`; `bb -calls-> zz` | `zz -defines-> bb`, context `call` |
| `zz -defines-> bb`; `bb -references-> zz` (parameter_type) | `zz -defines-> bb`, context `parameter_type` |
| `aa -defines-> zz`; `zz -references-> aa` | `aa -defines-> zz`, no context |
| `aa -defines-> bb`; `aa -calls-> bb` | `aa -defines-> bb`, context `call` (as §8.2) |

`method` against a reversed `calls` follows §8.2 (the `method` survives whatever the ids).
The shared fold (§8.2) ranks `defines` with `contains` and treats it as structural, which covers
the same-direction rows but not a reversed `calls` winning on a larger source (rows 1–2), so
`extract/fortran.rs` still folds each such in-file pair itself and emits one edge; the reproduction covers an internal procedure calling its
host and a procedure calling or referencing its module.

### 35.7 Quirks (Fortran)

- **FO1** `use m` creates a separate per-file node `m` instead of linking to module `m`, so
  every using file gets its own renamed node and the import never reaches the module.
- **FO2** Cross-file calls bind by label only (§35.4), and only `call` statements cross files:
  a function reference `f(...)` to another file's function gives nothing.
- **FO3** Type stubs bind by exact label across languages (a JSON key `name`, a Python class).
- **FO4** Only the first declarator of a type-bound `procedure` statement is bound.
- **FO5** A type used before its definition in the same file goes through a stub (and binds back
  only when the label is unique in the repository).
- **FO6** `defines` folds against reversed `calls` and `references` by source id order (§35.6).

### 35.8 Parity

- Fixture: nodes and links 100%, communities and array order exact, extract and cluster-only
  analysis and report goldens equal.
- graphify corpus `tests/fixtures/sample.f90` alone: 13/13 nodes, 17/17 links, communities exact.
  Inside the full corpus its four renamed ids (`…_sample_f90_…_point`, `geometry`, `distance`,
  `circle_area`) depend on same-stem files in languages dijkstral does not extract yet
  (`sample.jl`, `sample.ps1`; `sample.zig` with §33).
- numpy `f2py` test sources: 62 `.f90` files, 244 nodes / 192 links, 100% and communities exact;
  all 88 Fortran files (`.f .f90 .f95 …`) 344 / 266, 100%, with the §35.1 extensions.

---

## 36. Verilog (`.v`)

Fixture `tests/fixtures/verilog/`; goldens `verilog.*` and the report goldens of §17.6. Graphify
reads `.v` as Verilog/SystemVerilog, not V, with tree-sitter-verilog 1.0.3 (same version in
dijkstral). Probes were run through `graphify extract --code-only` on scratch copies. The corpus
check uses two open-source repositories, alexforencich/verilog-axi and YosysHQ/picorv32 (§36.7).
Implementation: `crates/core/src/{extract,resolve}/verilog.rs`.

### 36.1 Which files

- **Rule.** `.v`, `.sv`, `.svh` and `.vh` are Verilog (case-insensitive: `.V`, `.SV`). `.vams`,
  `.veo`, `.verilog`, `.vhd` are not classified. Dijkstral currently classifies `.v` only
  (`lang.rs`); the other three are a one-arm addition.
- A V-language file (`fn main() {}`) parses to error nodes and yields only its file node
  (Graphify warns "yielded no symbols").
- **V8** The file node is at `L1` even for a 0-byte file (§3 says `null` for other languages).

### 36.2 Nodes

| Construct | id | label | extra |
|---|---|---|---|
| file | file id | §5.1 | `confidence: "INFERRED"`, `confidence_score: 1.0` |
| module (also nested, `macromodule`, `extern module`) | `file_slug(name)` | `name` | same |
| function inside a module | `module_slug(name)` | `name()` | same |
| task inside a module | `module_slug(name)` | `name` (no parentheses) | same |
| function or task outside every module (package, interface, program, file scope) | `slug(path with extension)_slug(name)` (`rtl_alu_v_scale`) | as above | same |
| instantiated module not declared in the file | `slug(name)` (shared, §36.4) | name as written | stub, same |
| SystemVerilog class (§36.5) | `file_slug(Name)` | `Name` | same |
| class base / interface (§36.5) | `file_slug(Base)` | `Base` | same, at the class's line |
| class method (§36.5) | `class_slug(name)` | `name` (no parentheses) | same |

- **V1** Every Verilog node, file nodes and stubs included, carries the node fields
  `confidence: "INFERRED"` and `confidence_score: 1.0` (raw and clustered; 0.9.86 had only the
  score); every in-file edge carries `confidence_score: 1.0` in raw output too.
- **V2** A module's name is the first identifier anywhere in its header, so an attribute comes
  first: `(* keep_hierarchy *) module wide_path` is module `keep_hierarchy`.
- **V3** Definitions outside modules hang off the file's internal id, which keeps the extension
  (`a.v` → `a_v_twice`); they are contained by the file node `a`.
- A function's name is the identifier of its `function_identifier` (`function void Cx::m()`
  gives `Cx()`); the line is the `function`/`task` keyword's.
- Not emitted: packages, interfaces, programs, primitives (UDPs), ports, nets, variables,
  parameters, `always`/`initial`/`assign`, `` `include ``/`` `define ``, DPI imports, and comments
  (`NOTE:`/`WHY:` give no rationale nodes).

### 36.3 Edges

| relation | source → target | location |
|---|---|---|
| `defines` | file → module, file → class | definition line |
| `contains` | module → function/task; file → function/task outside modules | definition line |
| `instantiates` | innermost module → module or stub | instantiation line |
| `method` | class → method | method line (§36.5) |
| `inherits` / `implements` | class → base node | class line |

No `calls`, `references` or `imports`: function calls, task enables and `` `include `` give
nothing. Context is never set.

- **V4** An instantiation counts only where tree-sitter reads it as a `module_instantiation` or a
  `checker_instantiation`, anywhere in the module (generate regions, `if` blocks, nested
  modules use the innermost module). The grammar reads `name inst(a, b, c);` (positional
  ports, no parameters) as a UDP instance, an instance inside a `for` generate loop as an
  interface instance, and primitives (`and a0(...)`) as gates: none of these give an edge.
  Instantiations inside interfaces and programs give nothing.
- Every instantiation gives an edge (the same module twice: two edges, the later line survives
  the fold). A module instantiating itself gives a self-loop.
- **Fold rank.** `instantiates` outranks `contains`: a module that instantiates `m_x` and contains
  a function `x` whose id is also `file_m_x` keeps `instantiates` whatever the order (probe).
  `graph.rs` ranks it 41, just above the structural tier (§8.2).

### 36.4 Instantiation targets

- **Rule.** A name declared as a module anywhere in the same file (the module-name rule of V2
  applies, before or after the instantiation) is the target directly. Any other name gets a
  stub `slug(name)`, label as written, one per file.
- **Rule.** The stub binds across the repository to the only node, in any language, whose label
  without a trailing `()` equals the stub's label exactly (case-sensitive): a module, a function
  (`twice` → `twice()`), a task, a class, a Python class (`ref_model`). Two candidates, or a
  label differing in case (`Leaf2` vs `leaf2`), leave the stub. Even a function of the
  instantiating file binds this way.
- **V9** Verilog stubs are not file emitters in the §4.5 rename: two files instantiating `ghost`
  share one node `ghost`. A stub whose id equals a node id of another file renames only that
  node (`counter.v` → file `counter_v_counter` next to stub `counter`, which then binds to module
  `counter_counter`; an unbound `vlang` stub stays `vlang` beside file `vlang_v_vlang`).

### 36.5 SystemVerilog classes

**Rule.** Classes do not come from the tree but from a text scan, over the source with `//...`
(to the end of the line, the newline kept) and `/*...*/` (newlines included) removed, of
`\bclass\s+(\w+)([^;]*);(.*?)endclass` (non-overlapping, across lines):

- the header (`[^;]*`) gives `extends (\w+)` → `inherits`, and `implements A, B` → one
  `implements` per item (its leading word); each target is a node `file_slug(Base)` labelled
  `Base` at the class's line, created unless the id exists (a class of the same file, first
  wins); it never binds to the real class elsewhere (V6). `extends P #(int)` → `P`,
  `extends pkg::Q` → `pkg`;
- the body gives methods for `\bfunction\s+\w+\s+(\w+)\s*\(`: exactly one return-type word, so
  `function void f()`, `virtual function void f()`, `static function int f(int x)` and an
  `extern` prototype count, while `function new()`, `function automatic void f()`,
  `function bit [7:0] f()` and `function void f;` do not;
- then, in a second scan of the body, for `\btask\s+(\w+)\s*\(` (0.9.97; 0.9.86 skipped class
  tasks): `task drive()`, `task t ()`, `extern task t()`, `static task t()` and a name on the
  next line count, `task automatic run(...)` and `virtual task t;` do not. A task named like an
  earlier function is the same node (the function's line) and its `method` edge, emitted later,
  is the one that survives;
- nodes come after every tree-derived node of the file; edges in the order `defines`, `inherits`,
  `implements`, `method`s.

The tree walk skips class bodies, so class methods appear only through the scan; a class without
`endclass` is no class, and the tree's error recovery may then expose its functions as
file-level functions.

- **V5** The scan matches inside strings (`$display("class Fake;")` is a class), and a
  `typedef class X;` or any unmatched `class` header swallows the text up to the next
  `endclass`: `typedef class F; class A; ... endclass` yields class `F` with `A`'s methods, and
  no `A`.
- **V7** A class's line is computed from its offset in the stripped text applied to the
  original text, so every comment character before it moves it up (the fixture's two comment
  lines put `base_txn`, at L3, on L1). A method's line is its class's line plus the
  newlines of the stripped body before the `function` keyword.

### 36.6 Quirks (Verilog)

- **V1** `confidence: "INFERRED"` and `confidence_score: 1.0` as node fields on every Verilog node.
- **V2** Attributes name modules.
- **V3** Out-of-module functions and tasks keep the extension in their id prefix.
- **V4** Instantiations depend on tree-sitter's ambiguous reading (UDP/interface/gate instances
  are lost).
- **V5** Class scan over raw text: classes in strings, `typedef class` swallowing.
- **V6** Class bases are per-file nodes, never the real class.
- **V7** Class and method lines drift after comments.
- **V8** 0-byte files at `L1`.
- **V9** Stubs shared across files and exempt from collision renaming.

All nine are reproduced (they decide ids and lines).

### 36.7 Parity

`dijkstral extract` against Graphify's cold `extract --code-only` on scratch copies, `--ext .v`
(`parity.py`), node and link sets as recall / precision:

| repo | `.v` files | nodes | links | communities (NMI) |
|---|---|---|---|---|
| verilog-axi (master, 2026-10-14) | 83 | 170: 100% / 100% | 147: 100% / 100% | 1.000 |
| picorv32 (main, 2026-10-14) | 41 | 135: 100% / 100% | 141: 100% / 100% | 1.000 |
| 14 probe repositories | — | 100% / 100% | 100% / 100% except two links of one probe (fold rank, §36.3) | 1.000 |

Field values agree on every shared node and link. Community ids differ on the whole-repository
graphs only through their Python and C files (verilog-axi's cocotb benches: 468 extra Python
`calls` in dijkstral). Cold build: verilog-axi 0.42 s vs Graphify 7.3 s (144 files), picorv32
0.32 s vs 6.9 s.

### 36.8 Open

- The comment-stripping order for overlapping `//` and `/*` (left to right is assumed) and
  `implements` items written with a scope (`pkg::I`).
- The rank of `instantiates` relative to `imports`/`calls` (no Verilog pair can show it).
- **Not reproduced: SystemVerilog class members.** On the Graphify repository's
  `tests/fixtures/sample.sv` (the same output as `.sv` or `.v`), Graphify also emits field
  `references` (`rand Config m_cfg` → `Config`, context `field`; `Result #(Payload) current` →
  `Result` and `Payload`, `generic_arg`), method signature `references` (`parameter_type`,
  `return_type`), a method whose return type is parameterised (`function Result #(Payload)
  build(...)`, which the class scan misses), and `import math_pkg::*` as `imports_from` to a node
  `math_pkg` (located at the import). Dijkstral has 100% precision there and 11 of 13 nodes, 13 of
  20 links.

---

## 37. Solidity

Fixture `tests/fixtures/solidity/` (`contracts/{Base,Token}.sol` beside `scripts/deploy.py`);
goldens `solidity.*`. Implementation: `Lang::Solidity` in `crates/core/src/lang.rs`, no extractor.

- **Rule.** `.sol` counts as code (§3) but Graphify 0.9.86 ships without its optional
  `tree-sitter-solidity` extra, so every `.sol` file contributes nothing: no file node, no
  symbols, no edges. `extract` warns `N .sol file(s) contributed nothing to the graph because a
  dependency is missing` (the other "count but contribute nothing" extensions are `.r .tf`, and
  `.sql` without the SQL extra, §45).
- The files still count: `GRAPH_REPORT.md`'s corpus check lists them in its file and word totals
  (`3 files · ~105 words` for two contracts and one Python file). A repository of `.sol` files
  only gives an empty graph with a non-zero corpus check.
- Dijkstral classifies `.sol` (`Lang::Solidity`, no grammar) and emits nothing for it, so the
  totals match. A Rust Solidity grammar exists (`tree-sitter-solidity` 1.2.x), but using it would
  diverge from the reference until Graphify installs its extra.
## 43. C# project and markup files (2026-10-15)

Fixture `csproj/` (a solution and `.slnx`, three `.csproj`, a namespaced `.vbproj`, a broken
`.csproj`, `Directory.Build.props`, two Razor components and a `.cshtml` view, four XAML views with
code-behind and a CommunityToolkit view model); goldens `csproj.graphify.json`,
`csproj.clustered.graphify.json` (113 nodes, 134 links) and the §17.6 report goldens. Derived
from about sixty probe trees, the graphify repository's `tests/fixtures` (copied, inputs only) and
serilog. Implementation: `crates/core/src/extract/{xml.rs,dotnet/}`, `crates/core/src/resolve/dotnet.rs`,
plus the additions to C# named in §43.5.

`.csproj .fsproj .vbproj .sln .slnx .razor .cshtml .xaml` are code (case-insensitive);
`.props .targets .vcxproj .proj .shproj .projitems .sqlproj .nuspec .resx .config .axaml
packages.config` are not classified. The XML-based files (`.csproj` family, `.slnx`, `.xaml`) are
read with a conforming namespace-aware XML parser: a file that is not well-formed (unclosed tag,
junk after the root, undeclared prefix, undefined entity, duplicate attribute, leading whitespace
before `<?xml`, empty file) produces **nothing, not even a file node**. A BOM is accepted.

### 43.1 MSBuild projects (`.csproj`, `.fsproj`, `.vbproj`)

Elements are matched by local name anywhere in the document, whatever their namespace or root
element. All nodes and edges have `source_location: null` / no location; edges are EXTRACTED with
no context.

| Source | Node | Edge from the file |
|---|---|---|
| file | file node, label = basename, `source_location: null` | — |
| each `TargetFramework`, then each `TargetFrameworks` (split on `;`, trimmed, empty dropped; `$(Var)` kept verbatim) | `framework_<slug>` concept, label as written | `references` |
| `PackageReference Include=` (not `Update=`; the `Version` *attribute* only, a `<Version>` child or `VersionOverride` is ignored) | `nuget_<slug(Include)>` code node, label `Include (Version)` or `Include` | `imports` |
| `ProjectReference Include=` | the referenced project's node (§43.2) | `imports` |
| root `Sdk=` attribute (verbatim, `A;B/2.0` stays one) | `sdk_<slug>` concept | `references` |

Order: frameworks, packages, project references, SDK; one node per id per file (the first label
wins). `<Sdk Name=…>`, `Import`, `Reference`, `FrameworkReference`, `Using`, `Compile` produce
nothing. A `<Project></Project>` keeps its file node. `framework_*`/`sdk_*` collide across files
and are renamed (§4.5), then same-label concept nodes merge into the shortest id when the label's
entropy is at least 2.5 bits per character (the JSON rule of §16): serilog's
`src_serilog_serilog_csproj_framework_net10_0` serves every project; `net48` (2.32 bits) stays per
file. `nuget_*` nodes are code and never merge.

### 43.2 Solutions and project nodes

`.sln`: the file node (`source_location: null`, also for an empty or unparsable file) and, for
every `Project("{type}") = "Name", "Path", "{GUID}"` line (case-sensitive `Project`, any spacing):

- a project node: `Path` with `\` read as `/`, joined to the solution's directory and normalised
  (`os.path.normpath`); id `file_id(path)`, label `Name`, `source_file` = that path,
  `source_location: null`. A `Path` equal to `Name` (solution folders, whatever the type GUID) is
  not joined: id `slug(Name)`, `source_file` = `Name` (serilog: `assets`, `.github` → `github`).
- `solution -contains-> project` (unlocated), in file order.
- after all entries, `ProjectDependencies` sections: `project -imports-> dependency` by GUID
  (case-insensitive; unknown GUIDs dropped). `NestedProjects` is ignored.

`.slnx` (XML): every `Project Path=` (at any depth, inside `Folder`s too; lower-case `project`
ignored) gives the project node, labelled with the path's last `/` segment without extension
(backslashes are not split: `..\a\Core.csproj` → label `..\a\Core`), `contains` from the file, and
each `BuildDependency Project=` child an `imports` after all entries. `DisplayName`, `Folder`,
`File` produce nothing.

Project paths that leave the root keep their `../` form as `source_file` and get
`ext_<slug(path)>`; an absolute path is reduced to its basename. Inside the root, a referenced or
listed project is one node however many files name it: **a project node's `source_file` is the
project's path, not the naming file's**, and such nodes are not renamed by §4.5 (two solutions,
a solution and the project file itself all merge). When several namings carry different labels,
the **shortest label wins, ties by the smaller string** (`Graphify: node … was extracted twice …
keeping …`): serilog's `src/Serilog/Serilog.csproj` is labelled `Serilog` (from the solution), not
`Serilog.csproj`. A project node whose label equals its path's basename then takes part in file
labelling (§5.1, `x/Q.csproj`); any other label does not.

### 43.3 Razor (`.razor`, `.cshtml`)

A line scanner. The file node has `source_location: null` (also when empty). Directives must start
at column 0 and are read **in line order**; a directive naming `slug(X)` reuses a node the file
already emitted under that id (an `@inject` stub stays a stub, an `@inherits` def stays a def):

| Directive | Node | Edge from the file |
|---|---|---|
| `@page "p"` (unquoted ignored; one route per id) | concept `route_<slug(p)>` (`route` for `/`), label `route:p`, at its line | `references`, unlocated |
| `@model T` | def `slug(T)`, label `T` | `references`, located |
| `@using N` / `@using static N` / `@using A = T` (`;` stripped) | def `slug(target)`, label = target | `imports`, context `import`, `metadata {"using_kind": namespace\|static\|alias, "target_fqn": HTML-escaped target, "scope_kind": "file"[, "alias"]}` |
| `@inject Type name` (`[\w.<>]+` then a name; `IFoo<A, B>` does not match) | stub `slug(simple name)` | `references`, context `field`, `metadata {"ref_token"[, "qualified": true, "ref_qualifier"]}` |
| `@inherits T` | def `slug(T)` (generic arguments kept) | `inherits` |

`@namespace`, `@implements`, `@layout`, `@attribute`, `@typeparam`, `@functions` as a directive
produce nothing. Then **components**: every `<Name` in the whole file (code blocks and directive
lines included: `List<CounterRecord>` and `ILogger<Index>` count) where `Name` is an uppercase
ASCII letter followed by at least one ASCII letter or digit and then whitespace, `/` or `>`, except
`article aside body button div footer form head header html input label link main meta nav option
script section select span style table textarea title` (any case): def `slug(Name)` at its first
line and a `calls` edge per occurrence (no context). Then **methods**: in each brace-matched `@code`
/ `@functions` block, `(modifier\s+)+ type\s+ name\s*(` with modifiers `public private protected
internal static async override virtual abstract sealed` (case-sensitive) and a type of word
characters, `<>[],` and whitespace only: node `<file>_<slug(name)>`, label `name` (no
parentheses), located at the first modifier, `contains` from the file (unlocated); the first of
several wins. Constructors, properties, operators other than `operator int(` (→ `int`, quirk RZ1),
generic methods, tuple or nullable return types are skipped; comments are not stripped.

Cross-file (§43.5 lookups):

- `@using N` of a local C# namespace imports the namespace node; `@using A = T` of a local type
  the type; otherwise the def node stays.
- `@inject T`: C# name lookup from the file — aliases, the global namespace, then its own
  `@using` namespaces (two usings supplying the name: unresolved; `_Imports.razor` does not count);
  unresolved → the shared stub `slug(T)` (not the file's own stub).
- `@inherits T` and components bind **only when their node is an `@inject` stub**: to the only
  local type of that simple name in the repository, if its namespace is global, one of the file's
  `@using`s or one of an `_Imports.razor` in the same directory. A def-shaped component never binds.
- A node of the file whose every edge was bound elsewhere disappears.
- An `@inject` reference and a `calls`/`inherits` edge to the same target fold to the latter
  **without** the reference's `context` and `metadata` (not the §8.2 merge).

### 43.4 XAML (`.xaml`)

Strict XML (§43 intro). The file node is at `L1`. The root becomes `<file>_<slug(tag)>` (local
name), `file -contains-> root`. `x:` means the XAML language namespace
`http://schemas.microsoft.com/winfx/2006/xaml`.

- **Named elements**: the first `Name` or `x:Name` attribute of any element (the root too) gives
  `<file>_<slug(name)>`, contained by the **root** (flat), with `references` (context `type`) to
  the concept `xaml_<slug(local tag)>` (one per file; the prefix is dropped). Concepts and named
  nodes are located at the element.
- **Edge source**: a named element's own attributes start at its node; everything else (unnamed
  elements, however deeply nested, and the root) at the root.
- **Bindings**: `{Binding p}`, `{Binding Path=p, …}` (other keys ignored, `{Binding}` and
  `Mode=…`-only produce nothing; `x:Bind`, `TemplateBinding`, `StaticResource` alone are ignored)
  and `<Binding Path=…>` elements give the concept `binding_<slug(p)>` (label `p` as written,
  `A.B[0].C` too) with `references`, context `binding_command` when the attribute is `Command`,
  else `binding_path`; `Converter={StaticResource X}` gives `binding_converter_<slug(X)>` with
  context `binding_converter`. Located at the attribute's line.
- **`x:Class="N.C"`**: `root -references-> class` (context `x_class`, root line) to the class `N.C`
  declared in the `.xaml.cs` file beside it (exact path, exact fqn); otherwise to the file's own node
  `<file>_<slug(C)>`, which is only kept in that case.
- **Events**: any attribute (not `x:`, not `xmlns`, not `Name Header ToolTip DataContext Source Uid
  Key Class Text Tag Content Title Style`) whose value is an identifier naming a method of that
  code-behind class with an `…Args` parameter type (`RoutedEventArgs`, `MyArgs<int>`; not
  `Foo.BarArgs`, `object`, `List<EventArgs>`) gives `references` (context `event`, attribute line)
  to the method.
- **View models** — only for a view inside the directory of a `.csproj` (the nearest one):
  explicit `DataContext="{d:DesignInstance Type=vm:X}"` / `Type={x:Type vm:X}` attributes (not the
  positional form) and the elements inside any `*.DataContext` property element, in document order,
  give `root -references-> X` (context `view_model`, EXTRACTED, at the attribute/element) to the
  first C# class named `X` (arity 0) under that directory; with none written, the naming
  convention: the `x:Class` short name (else the file stem) without a trailing `UserControl`,
  `Control`, `Window`, `View` or `Page`, plus `ViewModel` (`MainWindow` → `MainViewModel`,
  `HomePage` → `HomeViewModel`, `Nested` → none), INFERRED, at the root line.
- **Generated members**: every view model bound this way gets the members CommunityToolkit.Mvvm
  generates — `[ObservableProperty]` fields (`title`, `_title`, `m_title` → `Title`; `mName` →
  `MName`) and `[RelayCommand]` methods (`SaveAsync` → `SaveCommand`) — as nodes
  `<view model id>_<slug(member)>` (source the view model's file, located at the attribute list)
  with `defines` from the class (INFERRED, context `communitytoolkit_observable_property` /
  `communitytoolkit_relay_command`), unbound ones too. A binding whose whole path names one of them
  adds an INFERRED `references` (same context and line) beside its concept edge; hand-written
  properties never match.

### 43.5 C# additions

- `crates/core/src/extract/csharp.rs` records the generated members of §43.4 in `CsFacts.generated`
  (no node is emitted by C# itself).
- `crates/core/src/resolve/csharp.rs`: **Razor and XAML code nodes act as types of the global
  namespace** — a type C# cannot resolve binds to the only Razor/XAML code node carrying its simple
  name (the last segment of a qualified name): `class A : Window` with one XAML `<Window>` root
  `inherits` that root node; with two such roots it stays the stub `window`.

### 43.6 Discovery and ordering notes

Graphify reads the code-behind and view-model files while extracting a XAML file, so it lists
the bound C# class and the generated members among the view's nodes; dijkstral emits them once,
from the C# file and the cross-file pass. The node order differs only in their positions (the
fixture's communities are unaffected).

### 43.7 Quirks (not reproduced)

- **PJ1** A project or solution-folder node whose id equals another file's node id: Graphify
  renames by the nodes' `source_file`s (`a` folder + `a.py` → `a_a`, `a_py_a`) or keeps the
  project's id and gives the other file its with-extension id (`pkg_mod` + `pkg_mod_py`);
  dijkstral merges them.
- **PJ2** Graphify's per-file C# stubs collide with same-id Razor nodes, so a Razor component
  `widget` that a C# file also references as an unresolved type becomes `<file>_razor_widget`;
  dijkstral creates C# stubs in the cross-file pass and keeps `widget`.
- **PJ3** An `x:Class` class that inherits its own XAML root's tag through §43.5 gets the
  root's `x_class` context merged into its `inherits` link in Graphify.
- **PJ4** (resolved) Surprises: Graphify lists project-file concept nodes (`net9.0`,
  `Microsoft.NET.Sdk`, XAML bindings) as surprise endpoints and never `imports` links; the shared
  analysis does the same (§17.4), and the fixture's `cluster-only` sidecars match byte for byte.

---

## 44. Other code-counted files (2026-10-15)

Fixture `misc/` (Pascal unit/program/package/include, a Delphi form, a Lazarus form and package,
Apex class and trigger, COBOL program and copybook, BYOND skin/icon/map, Metal, `.r .tf .ejs`,
five shebang scripts); goldens `misc.graphify.json`, `misc.clustered.graphify.json` (110 nodes,
109 links) and the §17.6 report goldens. Implementation: `crates/core/src/extract/legacy/` and
`crates/core/src/resolve/legacy.rs`. None of these uses a grammar; every file node is at `L1`,
even for an empty file.

### 44.1 Pascal (`.pas .dpr .pp .inc .lpr .dpk`)

Comments (`{…}`, `(*…*)`, `//`) and strings are blanked first; keywords are case-insensitive.

- `unit|program|library X;` (the first non-blank line) → `<file>_<slug(X)>`, contained by the file;
  `.dpk` packages and include files have none (routines hang off the file node).
- Every `uses` clause (interface and implementation) → `imports` (context `import`, at the `uses`
  line) from the unit to each entry (`X in 'x.pas'` → `X`): the file node of the file declaring
  `unit X`, else the external `slug(X)`.
- `T[<…>] = class|interface|dispinterface[ sealed|abstract][(bases)]` (not `class of`) → type
  `<file>_<slug(T)>` contained by the unit; its body is every line up to the next `end;`, whose
  `[class] procedure|function|constructor|destructor M` lines are methods `<type>_<slug(M)>`,
  label `M()`, `method`. **A forward `T = class;` opens a body too** (quirk PAS2): it swallows the
  declarations up to the next `end;` and keeps the type's first-declaration entry, whose methods
  merge in but whose later base list is never bound across files. Records and enums produce
  nothing.
- Bases: a type of the file → `inherits`; otherwise a node `slug(Base)` of the file at the
  declaration (`tobject`, renamed per file by §4.5), unless a used unit declares the type, which
  then gets the edge and the node is dropped. Interfaces are `inherits` too.
- Routines defined after `implementation` (the whole file without it), outside type bodies →
  `<file>_<slug(R)>`, label `R()`, contained by the unit; nested routines too, flat; interface
  forward declarations make no node; `Class.Method` definitions make none either.
- Calls: in each routine's first `begin` … next `end;` (so an outer routine with a nested one only
  sees the nested body), every identifier not qualified by anything but `Self.` that names a
  routine: the caller's class when it declares that name exactly once (overloads count twice), else
  the only declaration of that name in the file; self-calls dropped, one edge per target. A method
  whose name stays unresolved walks its class's bases (in the file, then in used units) in the
  cross-file pass (EXTRACTED 1.0). The reported line is the routine's header line plus the number of
  lines between `begin` and the call (quirk PAS1).

### 44.2 Forms (`.dfm`, `.lfm`, text form)

`object Name: TType` → node `<file>_<slug(TType)>` (one per type: two `TButton`s share it),
contained by the nearest enclosing `object` (else the file); `OnX = Handler` → `<file>_<slug>`,
label `Handler()`, `references` with context `event` from the enclosing object. `inherited` and
`inline` blocks (and collection `item`s) nest but make nothing, so their events are dropped. Each
edge once, at its first line.

### 44.3 Lazarus packages (`.lpk`, XML)

`Package/Name` (else the file stem) → `<file>_<slug(name)>`, `file -contains->`; each
`RequiredPkgs/*/PackageName` → node `slug(name)` and `imports` (context `import`); each
`Files/*/UnitName` → node `slug(unit)` and `contains`; all at `L1`. A unit named like exactly one
`.pas`/`.pp` file **in the package's directory or below** (the `Filename` value is ignored:
`zz.pas` with unit `C` finds `sub/c.pas`; `../e/f.pas` is never found) takes that file's id as
emitted by the package, which the §4.5 rename turns into `<package file>_<pas file id>`
(`d_d_lpk_d_sub_c`). When that id is the package file's own (same directory and stem), the unit
is a node `slug(<pas path with extension>)` instead (`pascal/Widgets.pas` → `pascal_widgets_pas`,
label `Widgets`) and the package file keeps its label (`Widgets.lpk`); 0.9.86 merged the unit into
the package file node.

### 44.4 Apex (`.cls`, `.trigger`)

A line scanner. `class X` → `<file>_<slug(X)>` contained by the file (inner classes too) and the
current class; `interface`/`enum` are contained by the current class without replacing it.
`extends B` → node `slug(B)` and INFERRED `extends`; `implements A, B.C<…>` → nodes for the first
dotted segment and INFERRED `implements`. A line starting with an access/member modifier or `void`
and naming `name(` is a method of the current class (`<class>_<slug>`, `.name()`), including
constructors and interface methods; an annotation on the same line hides it. After `@AuraEnabled`
or `@InvocableMethod` the file also `contains` it (INFERRED). `trigger X on Obj` → the trigger node
(current class) with INFERRED `uses` to `slug(Obj)`. SOQL `[SELECT … FROM Obj` and DML statements
(`insert update delete upsert merge undelete` followed by a word) give INFERRED `uses` to
`slug(Obj)` / `dml_<verb>` from the current class, **one edge per occurrence** (the last one's line
survives).

### 44.5 COBOL (`.cob .cbl .cpy`, fixed format)

Every node carries `metadata {"language":"cobol","kind":…}`. `PROGRAM-ID. X.` →
`<slug(file with extension)>_program_<slug(X)>` (`_callable`, kind `program`); level-numbered
data items → `…_data_<name>` (kind `data_item`, of the program, or of the file in a copybook);
area-A names in the PROCEDURE DIVISION → `…_paragraph_<name>` (kind `paragraph` or `section`,
`_callable`); `PERFORM X [THRU Y]` → `calls` (no context) within the program. Comment lines
(column 7 `*` or `/`) are skipped.

### 44.6 BYOND (`.dmf .dmi .dmm`)

`.dmf`: `window "w"` → `<file>_window_<w>` (label `window "w"`); its named `elem "e"` →
`<file>_elem_<w>_<e>`, label `elem "e" [TYPE]` from its `type =`; `macro`/`menu` blocks and unnamed
elements produce nothing. `.dmi`: the PNG `tEXt` `Description` chunk's `state = "s"` lines →
`<file>_state_<s>`, label `"s"`, located by their line in that text. `.dmm`: the file node only.
`.dm` is not classified.

### 44.7 Notes and gaps

- `.metal` is C++ (§25) and matches Graphify's output; since 0.9.97 Metal's function qualifiers
  and address spaces (`kernel`, `device`, ...) give no type reference (§25.2).
- **Inert code**: `.r .tf` and `perl`, `fish`, `tcsh` shebang scripts count as code (the
  report's file and word counts) but produce nothing, not even a file node.
- **EJS** (0.9.97): an `.ejs` file has its file node (`null` location when empty). Graphify also
  extracts the JavaScript of its `<% … %>` scriptlets and `<script>` blocks (`function foo()` →
  `foo()`, `contains`, in-file `calls`) and links `<%- include('b') %>` to `b.ejs` with an
  `includes` edge; dijkstral emits only the file node (no corpus has `.ejs`).
- Not reproduced: zTXt (compressed) `.dmi` headers (no inflate; only the file node), and the
  rename Graphify applies to a `.pas` file node when an `.lpk` names its unit from another stem
  (`c.pas` → `d_sub_c_pas_d_sub_c`).
- Surprises: with no cross-file candidate Graphify falls back to a community bridge between Pascal
  routines (`Area()` → `Compute()`); the shared analysis does the same (§17.4), and the fixture's
  sidecars match byte for byte.

### 44.8 Discovery after §43–§44

Running both tools over one file of every extension and shebang of §14.5 plus the new ones, every
file produces nodes in dijkstral exactly when it does in Graphify, except the languages other
stages add (Astro, Dart, Elixir, Fortran, Groovy/Gradle, Julia, Lua/Luau, Objective-C `.mm`,
PowerShell `.ps1 .psd1 .psm1`, SQL `.sql` (§45), Svelte, Verilog `.v .sv .svh .vh`, Vue, Zig). Shebangs are matched
by interpreter name after `env` and its flags, with an optional version `\d+(\.\d+)*` (0.9.97:
`python3.11`, `python311`, `python3.11.4`, `node18`, `nodejs20`, `ksh93`, `zsh5.9`, `ruby2.7`,
`php8` count; `Python3`, `pypy3`, `pythonw`, `python3.`, `python3.11a`, `python3-dbg`, `node-18`
do not): `python`, `sh bash zsh ksh dash`, `ruby`, `node nodejs` (JavaScript), `julia`,
`php`, and the inert `perl fish tcsh`; a BOM or a blank first line hides the shebang.
## 26. Groovy (2026-10-15)

Fixture `tests/fixtures/groovy/` (Gradle layout, Groovy sources under `src/main/groovy`, two Java
files the Groovy code uses, a Spock specification, `build.gradle` and `settings.gradle`); goldens
`groovy.graphify.json`, `groovy.clustered.graphify.json` and the §17.6 report goldens. `.groovy`
and `.gradle` are Groovy (`build.gradle.kts` stays Kotlin). Graphify parses them with
tree-sitter-groovy 0.1.2 (amaanq's Java-derived grammar; identified by matching the per-file
cache's `parse_errors.first_error_line` on every probe), and its error recovery decides much of
the output: a field without `;`, `s.trim()` as a last expression or a ternary are parse errors,
and declarations swallowed by an error node are lost. Derived from the fixture, about ten probes
and Graphify's own fixtures `sample.groovy`/`sample_spock.groovy`.

### 26.1 Nodes

| Construct | Id | Label | Fields |
|---|---|---|---|
| class, interface, enum — nested at any depth outside method bodies | `file_slug(Name)`, flat | `Name` | `_callable`, `_callable_class` |
| method, constructor (overloads merge, first line) | `<type id>_slug(name)` | `.name()` | `_callable` |
| method the grammar left outside any type (after an error) | `file_slug(name)` | `name()` | `_callable` |
| enum constant | `<enum id>_slug(NAME)` | `NAME` | none |
| supertype or annotation not declared earlier in the file | per-file stub `slug(Name)` | last segment of the name | §4.4 |

Not emitted: `trait`, `record`, `@interface`, top-level `def f()` (`function_definition`), script
statements, closures, fields, properties, and rationale (Groovydoc, `// NOTE:`). Gradle DSL
blocks (`plugins {}`, `dependencies {}`, `task x {}`) produce nothing; a class in a `.gradle` file
is an ordinary class.

### 26.2 Edges

All in-file edges: EXTRACTED, no score, weight 1.0.

| relation | source → target | when | context |
|---|---|---|---|
| `contains` | file or enclosing type → type; file → stray method | always | — |
| `method` | type → method | one edge per overload | — |
| `case_of` | enum → constant | always | — |
| `inherits` | class → superclass; interface → each `extends` type | generic `Base<T>` → `Base`, `a.b.Name` → `Name` | — |
| `implements` | class or enum → each interface | | — |
| `references` | type → each annotation (`@CompileStatic`, `@ToString(…)`) | type annotations only; `@Override` on a method produces nothing | `attribute` |
| `imports` | file → `slug(last segment)` (static member, wildcard package, aliased type) | every import, `L<n>` location | `import` |
| `calls` | method → method | §26.3 | `call` |

- **Denied supertypes (Rule).** About 170 JDK names never produce an edge or stub (`Comparable`,
  `Serializable`, `Cloneable`, `Runnable`, `Iterable`, `Exception`, `List`, `Map`, …; probed one
  by one, the list is in `extract/groovy.rs`). Annotations are not filtered.
- **Stub binding (Rule).** A stub binds to the class labelled with its name when exactly one
  class of *any* language carries that label and it is a Groovy, Java, Kotlin or Scala class
  (`KIface`, `JIface` bind; a Python-only, Swift or C# class, or a Groovy and a Python class of
  the same name, leave the stub). Annotation stubs bind the same way (`@Anno` → class `Anno`).
- **Imports.** In order: the importer's own stub of that id; the only Java class named so that
  Java code uses as a type (Java quirk J3, which reaches Groovy through the shared stub id:
  `import com.x.Processor` → the Java class); any node with that id; the only file node whose id
  equals it or ends with `_<it>` (§8.1), kept for a Groovy/Java/Kotlin/Scala file and dropped
  for another language; else an external node.

### 26.3 Calls

A method invocation inside a method (closures included) is recorded by its name, whatever the
receiver (`x.foo()`, `this.foo()`, `Format.pretty()`, `super.go()` all give `foo`/`pretty`/`go`).
Juxtaposed calls (`println "x"`), `new X()`, `this(…)`/`super(…)` and the Lua denylist (§28.3)
produce nothing; calls in field initializers or script statements have no caller.

1. **In the file**: the **last** method of that name in the file, any type (quirk GR1:
   `processor.reset()` binds the last `.reset()`; a method calling `x.reset()` inside the last
   `reset` is a self-loop). EXTRACTED, no score; one edge per (caller, callee).
2. **Across files**: the nearest (§19.5) method or function labelled `.name()` or `name()` in a
   Groovy, Java, Kotlin or Scala file (line-read Spock methods included); two candidates in the
   nearest file (a class's and its interface's `.count()`) bind nothing. INFERRED 0.85.

### 26.4 Spock specifications (quirk GR2)

A file with a line `def "…"(` or `def '…'(` is not parsed. Line by line: `import [static] a.b.C`
gives an import of `slug(C)`; `[@Anno] [modifiers] class Name` gives a class without `_callable`
flags (`contains` from the file); `def name(` and `def "text"(` / `def 'text'(` at the start of a
line give methods of the last class seen (`.name()`, or `"text"` with double quotes), no flags;
a `def` before any class is dropped, `void x()`, `private def x()`, interfaces, enums and
`extends` produce nothing.

### 26.5 Parity

Fixture: 100% of nodes and links, communities and array order exact, report goldens exact.
Corpus: Graphify's own fixtures (`sample.groovy`, `sample_spock.groovy`, `--ext .groovy,.gradle`)
100%/100% nodes and links; probes 100% except a Java-side stub (`class Ext2 extends Ext` gives
Graphify a Java stub `ext` that a Groovy import then targets; dijkstral's Java pass has no such
stub). Graphify 0.9.97 changed nothing here: the fixture's goldens are identical, and a probe
with generic classes, typed fields and `Type.m()` calls (`jvm-g1`) gives the same graph under
both versions.

---

## 27. Dart (2026-10-15)

Fixture `tests/fixtures/dart/` (a library with an empty `part` file, models with a mixin,
extension, typedefs and an enum, a service, utilities and a test); goldens `dart.graphify.json`,
`dart.clustered.graphify.json` and the §17.6 report goldens. Graphify reads Dart with regular
expressions, not a grammar: the per-file cache has no `raw_calls` or `parse_errors`, labels keep
line breaks from multi-line matches, and `library shop;` is a "variable" of type `library`.
Derived from the fixture, three probes and the per-file cache of dart-http (322 files), against
which a Python model of the passes was iterated to 99.9% before the Rust port.

### 27.1 Passes

Each pass runs one pattern over the whole text (`\s` crosses lines) and counts a match only when
the line holding its name is at brace depth 0 or 1, braces counted naively per line (strings and
comments included). Edges come out pass by pass, in this order:

1. **Types**: `class`/`mixin`/`enum` declarations by position. `defines` from the file; for a
   class, `inherits` (`extends`), `mixes_in` (`with`), `implements`, each to the type's leading
   `[\w.]` run (`_i1.Mock` kept whole, `jni$_.JObject` → `jni`, `class _$X` → `_`), and
   `references` (no context) from the class to the base of each generic argument of the
   superclass (`extends State<BookList>` → `BookList`; `implements X<Y>` references nothing); a
   mixin's `on` types → `inherits`.
2. **Annotations**: a run of `@name…` lines directly above a class, an extension type or a
   method gives `configures` from that declaration to `@name` (id `annotation_<slug>`,
   `file_type: concept`); `@override` is skipped. Annotations above fields configure nothing.
3. **Typedefs**: `typedef X = T;` → `defines X` and `references` (context `typedef`) from `X` to
   `T`'s last dotted segment (`ffi.Pointer<…>` → `Pointer`); function typedefs (`Function`) are
   not matched. A typedef whose id the file already defines gives nothing at all (0.9.97):
   `final class __CE_GeneralNames …` then `typedef CE_GeneralNames = __CE_GeneralNames;` adds
   neither a `defines` nor the self-reference `…_ce_generalnames -> …_ce_generalnames` that
   0.9.86 emitted; `class Foo {}` then `typedef FOO = Other;` references no `other`.
4. **Extensions**: `extension X on T` → `defines X` and `extends` from `X` to `T`'s first word
   (`objc.ObjCBlock<…>` → `objc`), at the line of `T`; `extension type X` → `defines X`.
   `extension on T` (no name, 0.9.97) → one node per file, id `fileid + "_" + slug(canonical
   absolute path of the file without its extension + "_anonymous_extension")` (quirk DA3: like
   DA1 it depends on the checkout's location), label `Extension on T` of the first, at the
   first one's line, with `defines` at the **last** one's line and `extends` from each to its
   own `T` (`extension on List<int>` and `extension on Map<…>` → `list`, `map`). Its members
   are found by the later passes as usual (`int get total` → `defines total`).
5. **Variables**: `TYPE NAME` followed by `=`, `;` or the end of a line, `TYPE` any run of word
   characters, `<>?,[].` and whitespace (line breaks included): fields, locals of top-level
   functions, getters (`T get x =>`), `library x;`, `return x == y` (type `return`), `as x;`.
   `defines NAME` and `references` (context `variable_type`) from the file to the type: leading
   `late final`/`late`/`final`/`const`/`var` dropped, the part before `<`, then the last dotted
   segment (`late final ffi.Pointer<K>` → `Pointer`, `String? get`, `static const int`,
   `external int` kept as written); `int double bool String num dynamic void Object`, `as`,
   `show` and (0.9.97) `typedef` reference nothing. `typedef X = …` matched here (function
   typedefs included: `typedef Validator = bool Function(User user);` defines `Validator`)
   defines `X` only when the file has no `X` yet (0.9.97).
6. **Methods**: `[modifiers] TYPE [x.]name(` with a lowercase `name` that is not a keyword
   (`get`, `set`, `if`, `Function`…), and `factory C.name(`; `defines`. Constructors, generic
   methods `f<T>(`, `void Name(` and names after a `$` are not matched.
7. **Imports**: `import`/`export 'uri' …;` → `imports`/`exports` to a shared node `slug(uri)`
   labelled with the URI; each `if (…) 'uri'` alternative adds one with context
   `conditional_uri`. URIs are never resolved to files.
8. **Generic calls**: every `f<T>(` in the file (any depth) → `references` (context
   `type_lookup`) from the file to `T`'s last dotted segment (`_lookup<ffi.Pointer<ffi.Char>>(`
   → `Char>`); types with `?` or `$` and the builtins above are skipped.

All edges are EXTRACTED with `confidence_score: 1.0`. A repeated (source, target, relation) in a
file keeps its first occurrence (quirk DA2; which line Graphify keeps for duplicates is not
fully pinned). The first is right in most cases: keeping the last instead moves 1,600+ dart-http
link locations away from Graphify's (28 differ with the first). Graphify keeps the last for two
getters of one type (`String get id`, then `String get label` → `string_get` at the second's
line), for a name a later pass defines again (`class Foo` L1, `int foo = 1` L5 → `defines` at
L5) and for anonymous extensions (rule 4); within one pass a definition keeps its first line
(`class WebSocket$Factory` and `class WebSocket` → the first).

### 27.2 Nodes

- **File**: `source_location: null` even when the file is not empty.
- **Definitions**: `file_slug(name)`, label as written, no flags; one node per id, first wins.
- **Shared nodes** (types, URIs, annotations): `{"id": slug(label), "label": …, "file_type":
  "code"|"concept", "source_file": null, "source_location": null, "_origin": "ast"}`; shared
  across files and exempt from §4.5 renaming. dijkstral writes them as external-kind nodes with
  null fields.
- **Parts (quirk DA1)**: a file with `part of 'lib.dart'` defines its names under the library's
  id from the library file node (`lib_lib -defines-> …`), adds `lib -includes-> part` at the
  `part of` line and `part -contains-> …` for each definition. Graphify then writes those ids with
  the slug of the canonical scan root in front (`users_hades_…_dart_http_pkgs_http2_lib_src_frames_frames_frame`),
  so they depend on where the repository is checked out; dijkstral reproduces this from the
  canonical root (the fixture's part file has no declarations for that reason). `part of name;`
  is only a variable (`references part of`).

### 27.3 Cross-file pass

A shared type node binds to the only Dart definition labelled with its name (`?` ignored), of any
kind (`objc` binds to the `as objc;` "variable"); otherwise it stays. The binding is global per
node id. The label is the kept one: `CE_GeneralNames`, defined only by a typedef that merged into
`class __CE_GeneralNames` (label `__CE_GeneralNames`), stays a shared node. Annotation nodes
(`@ffi`) never bind (0.9.97; 0.9.86 bound `@ffi` to the `ffi` variable, 14 dart-http links).

### 27.4 Parity and open questions

Fixture: 100% of nodes and links, communities exact, report goldens exact; one field
difference remains (a reference location, quirk DA2; the second one was the 0.9.86 `typedef`
reference). Against 0.9.97 (2026-10-29, `--ext .dart`): nodes 99.87/99.75, links 99.57/99.18
(`configures` 20/21, `references` 95.9/94.4), 28 link locations (DA2) and 5 contexts differ.
Earlier figures (0.9.86): Corpus dart-http
(`--ext .dart`, frozen reference): def nodes 99.9% recall / 99.7% precision, links 99.7% / 99.2%
(`defines` 99.9/99.7, `imports`, `exports`, `includes`, `contains`, `extends`, `mixes_in` 100%,
`inherits` 100/98.9, `implements` 100/91.5, `references` 97.2/95.2), NMI 0.964.

- **`defines` collapse (shared rule, reproduced).** When a file both defines `X` and references
  the type `X` (bound to that definition), Graphify keeps the `defines` link without the
  reference's context: `defines` is structural (§8.2). dart-http's 88 context differences are
  gone.
- Same-file preference in type binding (Graphify binds some types to a same-file definition when
  several files define the name), the `bloc_lookup` references (`context.read<T>()`, 12 corpus
  edges), a few parameter-list "definitions" and the first-wins label of shared nodes are not
  reproduced.

---

## 28. Lua (2026-10-15)

Fixture `tests/fixtures/lua/` (a `lib/shop` package with `init.lua`, models with metatables, a
busted spec, an extensionless `#!/usr/bin/env lua` script and a Luau module); goldens
`lua.graphify.json`, `lua.clustered.graphify.json` and the §17.6 report goldens. `.lua`, `.luau`
and extensionless files whose shebang program is exactly `lua` are Lua (`lua5.1`, `luajit` are
not); Luau uses the Lua grammar (tree-sitter-lua; 0.2.0, 0.4.1 and 0.5.0 match Graphify's
`first_error_line` on all of penlight and the probes) and keeps whatever its error recovery
leaves. Derived from the fixture, about twenty probes and penlight.

### 28.1 Nodes

- **File**: as §5.1; nothing else per file (no rationale: comments, `---` doc comments and
  `NOTE:`/`WHY:` are ignored).
- **Function**: every `function name(…)` statement (`local function f`, `function M.f`,
  `function M.sub:f`) that is neither inside a named function's body nor inside a `local`
  declaration, at any depth otherwise (`if`, `do`, loops, and anonymous functions passed as
  arguments, returned or assigned, as in busted `describe`/`it` blocks): id
  `file_slug(name as written)`, label `name()` with the name's text (`M.sub:f()`,
  `M . spaced()`), `_callable`, `contains` from the file at the statement line. Duplicate ids
  (`M:f` and `M.f`) merge, first label kept, one `contains` per statement.
- Not nodes: anonymous functions (`local f = function`, `M.f = function`, table fields), named
  functions inside function bodies or `local` declarations.

### 28.2 Imports (`require`)

- **Where.** Outside named function bodies only. A `local` declaration is searched as text for
  the first `require` (so `myrequire("x")`, `x.require("x")` and `"require('x')"` inside a string
  count, at the declaration's first line, and only the first of `require "a" or require "b"`);
  any other call named `require` (statement, argument, `return`, condition, assignment) is read
  the same way at its own line. Comments do not count.
- **Pattern.** `require`, optional space, then `(` with an optional `[[`, `"` or `'`, or a bare
  `"`/`'`; optional space; then the longest run of word characters, `.`, `-`, `/`. So
  `require(x)` → `x`, `require("a" .. b)` → `a`, `require "s13 x"` → `s13`; `require [[m]]`,
  `require(("m"))` and `require ""` name nothing.
- **Edge (quirk LU1).** `imports`, EXTRACTED, `confidence_score: 1.0`, context `import`, and a
  bare-number `source_location` (`"3"`, not `"L3"`).
- **Target.** With `.` read as `/`, Graphify checks the disk from the importer's directory up to
  the root: `<dir>/<path>.lua`, then `<dir>/<path>/init.lua`, nearest directory first (`pl.` →
  `pl/init.lua`). A hit links that file; an existing file that is not indexed drops the edge.
  Otherwise the target is the external `slug(module)`, which the build then remaps (§8.1) when
  exactly one file node's id equals it or ends with `_<it>` (`pl_utils` → `lua/pl/utils.lua`,
  `zz_top` → `lib/zz/top.lua`): a Lua/Luau file is linked, the importer itself or a file of
  another language drops the edge, two candidates keep the external.

### 28.3 Calls

Calls belong to the enclosing node function (calls in anonymous functions included); calls at
the top level, in anonymous functions outside a node, or in nested named functions are dropped.
The callee is the call's name as written: `f`, `M.f`, `a.b.c`, `f()` for `f()()`. Bare names in
this denylist never bind and are not recorded: the Q15 Python builtins except `object`, the JS
globals, the constructor list except `Uuid`, and `console` (`print`, `type`, `len`, `map`, `next`,
`open`, `str`…; Lua's own `tostring`, `pairs`, `require`, `setmetatable` bind).

1. **In the file** (EXTRACTED, no score, one edge per (caller, callee), first line): a name equal
   to a function's label without `()`; `self:m()` inside a function named on a table `T` binds
   `T:m` or `T.m`. Other method calls (`x:m()`, `Obj:m()`, `self.k:m()`) never bind, in the file
   or across files.
2. **Across files**: a name equal to the label of a file the caller's file requires
   (`lexer.lua(…)` with `require "pl.lexer"`) links that file, EXTRACTED 1.0. Otherwise the
   nearest (§19.5) Lua function labelled with the name, or Lua file named so (`p.lua()` → `p.lua`),
   INFERRED 0.85; a tie binds nothing. Only Lua and Luau files are candidates.

### 28.4 Parity

Fixture: 100% of nodes and links, communities and node order exact, report goldens exact.
Corpus penlight (`--ext .lua`, frozen reference): 905/905 nodes, 1800/1800 links, communities and
array order identical. Graphify's `sample.luau`: 100%.

- **Shared loader.** The `graph.json` loader behind `dijkstral cluster-only` keeps any location
  that is not `L<n>` verbatim (the bare `"3"` of quirk LU1), so re-clustering Graphify's own Lua
  graph reproduces its link order.

## 45. SQL (2026-10-29)

Fixture `tests/fixtures/sql/` (§1); goldens `sql.*` from **Graphify 0.9.97** with tree-sitter-sql
0.3.11 installed (its `postgres` extra; from 0.9.97 also `[sql]`), the only goldens not made with
0.9.86. Implementation: `crates/core/src/extract/sql.rs` and `crates/core/src/resolve/sql.rs`, on
the same grammar (derekstride/tree-sitter-sql 0.3.11, published to crates.io as
`tree-sitter-sequel`). Derived from 293 single-purpose probes, compared as clustered node and link
sets, and checked against the `.sql` files of Graphify's own repository (§45.6).

### 45.1 Which install

Without the grammar, `.sql` counts as code and contributes nothing (§14); 0.9.97 warns `N .sql
file(s) contributed nothing to the graph because a dependency is missing … pip install
"graphifyy[sql]"`. With it, `.sql` files are extracted as below. **Deliberate:** dijkstral always
extracts them, so on `.sql` it matches Graphify with the extra, not the base install. 0.9.86 with
the grammar differs from 0.9.97 only by lacking `writes_to`.

### 45.2 Nodes

- The file node has `source_location: null` even with content (contrast §6). A file without
  definitions (DML only, comments only, empty) keeps its file node.
- A pre-order walk of the whole tree (function bodies and `ERROR` nodes included) gives, each
  `file_slug(label)` with `contains` from the file at the statement's line:

  | Statement | Label |
  |---|---|
  | `create_table` (also `IF NOT EXISTS`, `TEMP`, `AS SELECT`) | the name as written: `"Quoted Table"`, `` `users` ``, `billing.invoices` |
  | `create_view` (also `OR REPLACE`; never `MATERIALIZED`) | the name |
  | `create_function` | `name()` |
  | `create_trigger` | the name |
  | `create_index` with a name (`CREATE INDEX ON t` gives nothing) | the name |

- An `ALTER TABLE` names its table even when it adds no edge: a table not defined in the file
  gets its stub (§45.4) at that point of the walk (`ALTER TABLE z ADD COLUMN y int` alone gives
  node `z`).
- Routines read from the text (procedures, and functions the grammar loses) come after every tree
  node (S2).
- Schemas, types, sequences, materialized views, grants, comments and DML give nothing.
- A name defined twice in a file keeps the first node (§4.5), which collects both definitions'
  edges.

### 45.3 Edges

All EXTRACTED, weight 1.0, in tree order, then S1, then routines.

| Relation | From → to | Line |
|---|---|---|
| `references` | table → the object after each `REFERENCES` of a column definition or table constraint | the table's |
| `references` | the table an `ALTER TABLE [ONLY|IF EXISTS]` names (resolved like any name) → the object after each `ADD CONSTRAINT … REFERENCES` | the `ALTER`'s |
| `reads_from` | view or function → each relation in a `FROM` or `JOIN` (subqueries, CTE bodies, `UNION` arms); CTE names excluded even when a table has that name | the relation's |
| `indexes` | index → its table | the statement's |
| `triggers` | trigger → its `ON` table | the statement's |
| `executes` | trigger → `EXECUTE FUNCTION|PROCEDURE f()` when a **function** with that key was defined **earlier in the same file**; otherwise nothing | the statement's |
| `writes_to` | routine read from the text → each written table (S2; 0.9.97) | the keyword's |
| `reads_from` | routine read from the text → each read table (S2) | the keyword's |

Not emitted: reads of `CREATE TABLE … AS SELECT`, the writes of a function the tree parsed,
top-level DML, and a `REFERENCES` anywhere else: one without a column list
(`REFERENCES "s"."t"`, which the grammar leaves in an `ERROR` node inside the table) or in an
`ALTER TABLE … ADD COLUMN`. Only S1 can still add those, for unquoted words. Repeated tree edges
are all emitted; the build keeps the last (a view reading `u` twice keeps the later line). S2
emits each relation and name once per routine (the first).

**Quirk S1 (references from the raw text).** Each `CREATE\s+TABLE\s+(\w+)\s*\(` in the source
(case-insensitive, anywhere in a line; never `IF NOT EXISTS`, `TEMP`, quoted or schema-qualified
names) owns the text up to the next line that begins, at column 0, with `CREATE` or `ALTER`
(case-insensitive, even inside a block comment). Every `REFERENCES\s+(\w+)` in that text adds
`references` from the table to the word, at the table's line. So `REFERENCES catalog.suppliers (id)`
also references a stub `catalog`, and words in comments and strings count (`-- … REFERENCES
legacy_items`).

**Quirk S2 (routines from the text).** The grammar cannot parse `CREATE PROCEDURE`, and loses a
`CREATE FUNCTION` whose PL/pgSQL body it cannot recover from (`PERFORM … JOIN …`, `NULL;`): the
statement becomes loose tokens in an `ERROR` node, with no `create_function` and no relations.
**Only when the parse has an error anywhere** (the root's `has_error`, a `MISSING` node included),
Graphify reads routines from the source with comments and one-line `'…'` strings blanked (a string
spanning lines stays):

- Each `\bCREATE\s+(OR\s+REPLACE\s+)?(FUNCTION|PROCEDURE)\s+name`, case-insensitive and
  **anywhere in a line** (`$$; CREATE PROCEDURE p()`, and inside a dollar-quoted string), with
  `name` `[^\s(]+` as written (`"s"."P"`, `billing.close_invoice`), gives `name()` at the line of
  `CREATE`, in text order, procedures and functions alike. It is skipped, body included, when that
  id is already defined: by the tree (a function the grammar parsed keeps its tree reads and gets
  no writes) or by an earlier routine. So a file that parses cleanly gives no routine from a
  string, and one with an error anywhere does.
- The body runs to the next line starting (after spaces) with `CREATE` or `ALTER`, so top-level
  DML after the closing `$$` belongs to it, and a `CREATE` later on the same line ends neither the
  body nor the routine before it (whose body then holds the next routine's too).
- In the body, in text order: `writes_to` for `(INSERT\s+INTO|UPDATE|DELETE(\s+FROM)?|MERGE\s+INTO)\s+name`
  and `reads_from` for `(FROM|JOIN)\s+name`, where `name` is `[\w$."]+`, optionally in `[…]`
  (dropped from the label; backticks never match). A read is skipped when its `FROM` belongs to a
  `DELETE FROM`, when the name is a CTE (`x AS (`, `x(cols) AS (`) or is followed by `(` (a
  function), and when its keyword lies in `\b(EXTRACT|SUBSTRING|TRIM|OVERLAY)\s*\(` up to the
  first `)` or in `\bDISTINCT\s+FROM` (**quirk S4**): `EXTRACT(YEAR FROM d)` and `a IS DISTINCT
  FROM b` read nothing, while `SUBSTRING(f(x) FROM t)` reads `t` and `foo(a FROM b)` reads `b`.
  Either is skipped for `ONLY`, `LATERAL`, `SET` and `DUAL`: `UPDATE ONLY t` writes nothing, and
  only the first table of `FROM t1, t2` is read.
- Each relation and name (by key, §45.4) is emitted once per routine, at its first keyword: a
  procedure reading `t` on two lines keeps the first line, and writing `t` twice keeps the first
  write (a read of `t` is still separate).
- A name whose slug is empty (`MERGE … THEN DELETE $$` writes to `$$`) gives a raw node with id
  `""`, which the build drops; dijkstral never emits it.
- A trigger's `executes` never targets a routine read from the text: they come after the walk.

### 45.4 Names and resolution

- A name's key is its text without `"`, `` ` `` and whitespace, lowercased, schema included:
  `s.t` is not `t`, and `public.orders` is not `orders`.
- A definition in the file (any kind, before or after the use) is the target.
- Otherwise the file emits a stub `slug(name)`, label as written, at first use. These stubs take
  part in the §4.5 rename: when two files emit stub `t` they become `a_sql_t` and `b_sql_t`, and a
  stub `t` renames another file's node `t` too, **even when the stub then resolves** (quirk S3, as
  Python's §4.5 case): a file `t.sql` becomes `t_sql_t` once another file reads `t`.
- Across files (`crate::resolve::sql`), a stub whose key has exactly one SQL definition anywhere
  (labels without `()`: tables, views, functions, procedures, triggers, indexes; never another
  language's nodes) is replaced by it. Otherwise, undefined or ambiguous, every file's stub for the
  key folds onto the first file's: two files reading an undefined `zz` share `a_sql_zz`, and in the
  fixture `customers`, defined by a migration and by the MySQL dump, is one shared stub.

### 45.5 Build stage

`reads_from` ranks below `writes_to` (rank 3.7 in §8.2's order): a procedure's
`UPDATE t SET a = (SELECT b FROM t)` keeps `writes_to`. Its place against `uses` and `calls` is
**Open**; no SQL pair has shown it.

### 45.6 Parity

Fixture: 100% of nodes and links, communities exact, report goldens exact. Probes: 293 of 293
match. The seven `.sql` files of Graphify's repository at the bench pin (v0.9.86, `corpus.toml`),
run through both tools as data: 68 of 68 node and link records match. Every quirk above is
reproduced. In the whole `graphify` corpus (2026-10-29, `parity.py --ext .sql` against 0.9.97 with
the grammar) all 34 nodes and 34 links match and one link's line differs: `sample_schema_qualified.sql`
gives `references` both from its `CREATE TABLE` (L6) and from an `ALTER TABLE` (L11), and Graphify
keeps L6 there because the corpus also holds a C++ `Logger.h`/`Logger.cpp` pair, which turns the
whole build to first-wins (quirk ZG2, §33.6, not reproduced). Without that pair it keeps L11, as
dijkstral does. The corpus's non-SQL nodes and links are identical before and after SQL
extraction.

---

## 46. YAML (2026-10-29)

Graphify 0.9.97 (`graphifyy`; YAML extraction arrived in 0.9.95 and was reworked in 0.9.97, per its
changelog) classifies `.yaml` and `.yml` (any case) as code and extracts them structurally, like
JSON manifests (§14.1) but with sequences descended and its own rules for which documents count.
Fixture `tests/fixtures/yaml/`, goldens `yaml.*` (Graphify 0.9.97). Probes were run through
`graphify extract --code-only` on scratch copies, 0.9.86 beside 0.9.97, and checked on the 19 bench
corpora (§46.6). Implementation: `crates/core/src/extract/data/yaml.rs`.

### 46.1 Which files and documents count

- **Loading** (Rule). The text is loaded as a safe YAML 1.1 loader does, every document of the
  stream. If any document fails to load, the file yields nothing at all: a syntax error, a tab
  where YAML forbids one, an undefined alias, a local or foreign tag (`!Ref`, `!Sub`,
  `!!python/name:…`; core tags such as `!!str`, `!!set`, `!!omap`, `!!binary` load), a collection
  used as a key (`? {a: 1}`, an unquoted Helm `{{ .Values.x }}`), a plain `=` value, a `<<` merge
  whose value is not a mapping or a list of mappings, an impossible date (`2019-02-30`), text an
  explicit `!!int`/`!!float`/`!!bool`/`!!binary`/`!!timestamp` cannot read, characters YAML
  refuses (C0 controls), or a file over **1 MiB** (1,048,576 bytes still loads). A UTF-8 BOM, CRLF
  line ends, `%YAML 1.1` and `...` are fine.
- **Configuration files** (Rule): a basename in this list (case-insensitive) or any file below a
  `.github/workflows` directory (any depth, case-insensitive). Every mapping and every sequence
  document counts (scalar and empty documents do not); OpenAPI and JSON-Schema documents count too.
  `mkdocs.yml mkdocs.yaml docker-compose.yml docker-compose.yaml compose.yml compose.yaml
  .gitlab-ci.yml .gitlab-ci.yaml action.yml action.yaml pubspec.yaml chart.yaml chart.yml
  values.yaml values.yml kustomization.yaml kustomization.yml .travis.yml .pre-commit-config.yaml
  serverless.yml serverless.yaml environment.yml cloudbuild.yaml cloudbuild.yml
  azure-pipelines.yml appveyor.yml skaffold.yaml`. Probed and not in it: `pubspec.yml`,
  `.travis.yaml`, `environment.yaml`, `azure-pipelines.yaml`, `.pre-commit-config.yml`,
  `values-prod.yaml`, `docker-compose.override.yml`, `bitbucket-pipelines.yml`, `.drone.yml`,
  `.github/dependabot.yml`, `templates/*.yaml` and some 40 other CI and deployment names.
- **Other files** (Rule): a document counts when it is a mapping whose top-level keys, compared
  case-insensitively, are neither an OpenAPI/Swagger spec (`openapi` or `swagger`, and `paths`) nor
  a JSON Schema (`$schema`, and `definitions`, `$defs` or `$id`, as §14.1), and that has at least
  one collection value (a mapping or sequence, empty or not, even under a key that is then dropped)
  or a structural key: `apiversion dependencies functions hosts jobs kind metadata name networks
  on pipeline provider resources roles scripts services spec stages steps tasks version volumes
  workflows` (found among 496 candidate keys, one file each). So `a: 1` and fastapi's
  `language_names.yml` yield nothing while `kind: x` counts; top-level sequences and scalars never
  count.
- No document counts → no file node either (like a non-manifest JSON file).
- `pnpm-lock.yaml` is dropped silently, and secret-looking names are dropped (§14.5):
  Kubernetes `secret.yaml`, `*-serviceaccount.yaml`, `token-config.yaml`, `core.v1.Secret.yaml`.

### 46.2 Nodes and edges

- **File node** (Rule): `fileid`, label the basename (disambiguated as any file, §5.1), `L1`,
  no extra fields.
- **Key nodes** (Rule): every key of a counting mapping becomes a node `{"id","label",
  "file_type":"code","source_file","source_location","_origin":"ast"}` at the key's line, with a
  `contains` edge (EXTRACTED, no context) from its parent at the same line, in pre-order (node,
  edge, then the key's value). Values are never nodes and no other relation is ever emitted
  (`depends_on`, `$ref`, `INHERIT`, image names: nothing).
- **Ids** (Rule): depth 1 `<fileid>_<slug(key)>`; deeper `<fileid>_<slug(parent key)>_<slug(key)>`,
  only the immediate parent key, as JSON's quirk (§14.1). A sequence is transparent: the mappings
  in it, at any nesting of sequences, are children of the key that holds the sequence
  (`list: [{k: 1}]` → `…_list_k`). A key whose slug is empty (`"@/*"`, `~`, `""`) is skipped with
  its subtree. When a key's id equals its parent's (`items` in a list under `items`), the node
  merges into the parent and no self-loop is written. Same-id keys elsewhere in the file are one
  node (the build keeps the first, §8) with a `contains` edge from each parent.
- **Labels** (Rule): the key's text as written (unquoted, escapes decoded): `yes`, `0x1F`,
  `2001-12-14`, `True`, `x"y`, `Ünïcode`, `a.b/c d`. Keys are compared by that text, so `null` and
  `Null` are two keys (whose slugs then merge the nodes). A key repeated in one mapping keeps its
  first position and takes the last line and value (`dup: {a}` then `dup: {b}` → only `dup_b`), as
  a Python dict does.
- **Aliases and merges** (Rule): an alias walks its anchor's node again under the alias's key, so
  the children keep the anchor's lines (`replica: *db` → `…_replica_driver` at the anchor's line).
  `<<` merges as YAML 1.1 does: merged pairs first (from a list of mappings, the last mapping's
  first), then the mapping's own pairs, which win over merged ones; there is no `<<` node.
- **Bounds** (Rule): at most **500** key nodes per file in pre-order (document nodes do not
  count; the JSON budget, §14.1); keys only for mappings at most **6** collections below the
  document (`a.b.c.d.e.f.g` stops after `g`; each sequence counts as a level), one level fewer
  when the file has document nodes.

### 46.3 Documents

- One counting document: its keys hang off the file node (a sequence document in a configuration
  file: the file node alone).
- Several (Rule): each gets a document node, numbered `i` among the counting documents only,
  id `<fileid>_doc<i>_<slug(label)>`, at the line of the document's first node, with
  `contains` from the file. Its depth-1 keys are `<fileid>_<slug(label)>_<slug(key)>` (the label
  acts as the parent key). The label is `Kind name` when the document's `kind` is a non-empty
  string and `metadata.name` is one too (`Deployment shop`, `RoleBinding system::leader-locking-…`),
  `Kind` with `kind` alone (`ConfigMap`, also with only `metadata.generateName`), else
  `doc <i>` (`kind: 5`, `Kind: Pod`, a sequence document). Two documents with the same label have
  distinct document nodes but share their key ids (`…_pod_p1_spec`, first wins).

### 46.4 Discovery and other paths

- YAML is no longer a document for `update`'s semantic pass or the markdown structure (§3).
- Code references bound to YAML keys: see §46.5.

### 46.5 Quirks not reproduced

- **Y1** A symlinked file is read through the link and its key ids embed the absolute path of the
  checkout (`users_hades_…_docs_content_manual_manual_body`, jq's `manual.yml → v1.8/manual.yml`).
  Graphify does this for every language (a symlinked `.py` too, also in 0.9.86); dijkstral's walk
  does not follow file symlinks, so the 15 nodes are absent.
- **Y2** Other languages' resolvers bind an unresolved name to a YAML key with that label: Python
  `Path(…)` → a workflow's `path` key (51 links in the graphify corpus), `*args` → a pre-commit
  `args` key (fastapi), `@patch` → codecov's `patch` (erpnext), a Rust `use …::version` → a
  workflow's `version` output (ripgrep). These are wrong links and dijkstral makes none. C
  includes are not bound to JSON or YAML keys in either tool (`<sys/types.h>` stays an external);
  `<windows.h>` does reach `windows.yml` through the file-stem rule (§8.1) in both.
- **Y3** Go method edges whose receiver id was renamed attach to a YAML file node ending in the
  same id (`test_images_apparmor_loader_example_pod -method-> example_pod_deepcopy`, Kubernetes,
  3 links); dijkstral makes none.

### 46.6 Parity

Fixture: 100% of nodes and links, communities exact, report goldens exact. All probes match except
a contrived second document with a stray `]` that Graphify's loader refuses and dijkstral's accepts.
Corpora (2026-10-29, `parity.py --ext .yaml,.yml` against 0.9.97): all 19 at 100% of YAML nodes
and links except jq (15 nodes and 14 links of the symlinked `manual.yml`, Y1); Kubernetes
188,804 YAML nodes match.
