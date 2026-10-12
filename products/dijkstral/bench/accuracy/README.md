# Graph accuracy against SCIP

Stage-1 parity (`../parity.py`) measures how close dijkstral's `graph.json` is to Graphify's. This
directory measures which of the two is **right**, against compiler-grade ground truth: SCIP indexes
from the language servers' own indexers (Pyright, the TypeScript compiler, rust-analyzer,
`go/types`). Nothing here runs in `bun run check`. Indexers, indexes, oracles and results live
outside the source tree under `$DIJKSTRAL_BENCH_HOME` (default `~/.cache/dijkstral-bench`);
`results/` is gitignored.

## Files

| File | Purpose |
|---|---|
| `setup.sh` | Installs the pinned indexers into `$DIJKSTRAL_BENCH_HOME/tools/scip/` (release binaries checked against their `.sha256`; npm packages into a private prefix) and writes `tools/scip/VERSIONS` |
| `index.sh <repo> [--force]` | Copies the corpus checkout to `scip/<repo>/src` and runs the right indexer(s) there, writing `scip/<repo>/parts/<part>.scip` + `.log`. Kubernetes is indexed in place (2.4 GB; scip-go writes only `--output`) |
| `graphs.sh [repo…]` | Builds dijkstral's `extract --code-only` graph per repo into `accuracy/dijkstral/<repo>/` and records the commit and binary sha256 in `BUILD`. Graphify's side is the frozen `reference/<repo>.graph.json` |
| `oracle.py <repo>` | SCIP → ground-truth edges in symbol space, cached as `accuracy/oracle/<repo>.json`; `map_defs()` maps definitions to one tool's nodes |
| `score.py` | Precision / recall per repo, tool and relation family, with confidence tiers and collapse accounting → `results/accuracy-<date>.{md,json}` (the JSON carries sample false positives / negatives and the pairs only one tool gets right) |

## Pins

| Indexer | Version | Used for |
|---|---|---|
| `scip` CLI | v0.10.0 | `scip print --json` (the decoder; no protobuf dependency) |
| `scip-python` | 0.6.6 (npm, Pyright inside) | fastapi, graphify, erpnext, monet `.py` |
| `scip-typescript` | 0.4.0 (npm) | excalidraw, monet `.ts/.tsx` |
| `rust-analyzer` | 2026-09-28 release (`0.3.3065`, `rust-analyzer scip`) | ripgrep, monet `products/{butterfly,cocoon}/cli` |
| `scip-go` | v0.2.7 | kubernetes subset |

Java is not indexed: no corpus repo is Java. Toolchain on the measuring machine: node 24.18.1,
go 1.26.5, cargo 1.96.1 (rust-analyzer runs build scripts and proc macros through it).

Indexer quirks handled in `index.sh`: Pyright 0.6.6 excludes `**/.*` by default, which matches the
`.cache` in the bench path and silently yields an empty index, and it rejects Python 3.13, so each
Python scratch copy gets a `pyrightconfig.json` (`include: ["."]`, explicit excludes,
`pythonVersion: "3.12"`). For monet the bun workspace packages are symlinked into
`src/node_modules/@monet/*` so cross-package imports resolve (no dependency is installed).

## Run

```bash
cd products/dijkstral
cli/target/release/dijkstral --version || (cd cli && cargo build --release)
bench/accuracy/setup.sh
for r in fastapi graphify erpnext excalidraw ripgrep monet kubernetes; do bench/accuracy/index.sh $r; done
bench/accuracy/graphs.sh
cd bench/accuracy && uv run --no-project score.py --rebuild-oracle   # all repos; --repos fastapi,ripgrep
```

Indexing all seven takes about 3 minutes on an M5; `score.py --rebuild-oracle` about 20 seconds.

## Method

**Indexed scope.** fastapi, graphify, erpnext: every `.py` file (erpnext's JavaScript is not
indexed). excalidraw: the root `tsconfig.json` project (`packages/*`, `excalidraw-app`; `examples/`
and `dev-docs/` are outside). ripgrep: the cargo workspace. monet: Python, every `tsconfig.json`
project, and the two cargo workspaces. kubernetes: indexing all of it is impractical, so
`./pkg/scheduler/...` and `./staging/src/k8s.io/apimachinery/...` (761 files, dense in
cross-package calls and interface use). Tool edges touching files outside the indexed scope are
reported as unjudged, never as errors.

**Definitions → nodes.** Every non-local SCIP definition (`().` function/method, `#` type, `!`
macro, `.` term) gets (file, 1-based line of its name, name). It maps to a node of a tool's graph
when the node has the same `source_file`, a `source_location` on that line, and a label whose bare
name (`.m()` → `m`, `f()` → `f`, `Pair<T>` → `Pair`) equals the definition's name. Both tools put
`source_location` on the keyword line (`def`, `class`, `fn`, `func`, `const`), never on decorators
or attributes, so the ±3-line fallback almost never fires (≤0.14% of mapped definitions). The mapping is built per tool.
Mapping rates (functions / types):

| repo | functions | types | unmapped functions are… |
|---|---|---|---|
| fastapi | 99.9 / 99.9 | 88.2 / 88.2 | classes defined inside test functions (neither tool emits them) |
| graphify | 99.9 / 99.9 | 63.8 / 63.8 | the same (fake classes inside tests) |
| erpnext | 99.8 / 99.8 | 99.4 / 99.4 | |
| excalidraw | 98.8 / 98.9 | 99.8 / 100.0 | `foo` + `Foo` id collisions (Graphify quirk, reproduced) |
| ripgrep | 78.6 / 78.6 | 92.3 / 92.3 | macro-generated tests (`rgtest!`, globset's test macros), seen by rust-analyzer only |
| monet | 99.1 / 98.8 | 97.7 / 97.7 | nested Rust fns |
| kubernetes | 99.8 / 99.8 | 97.8 / 97.8 | |

(Graphify / dijkstral.) TS function-like locals (arrow consts inside functions) are mapped too and
are not counted here.

**Relation families** (unordered pairs, because `graph.json` keeps one link per pair):

- `calls` — a reference to a function, method, macro or callable term in call position (`f(`,
  `f::<T>(`, `m!(`, TS `f<T>(`, `` f`…` ``, `new C`, JSX `<C`; Python `C(` counts as constructing
  `C`; Go `T(x)` is a conversion and Rust `Tuple(..)` a constructor reference, not calls). The
  caller is the innermost enclosing definition **that the tool models**: the oracle keeps the whole
  chain of enclosing definitions and score.py walks it outwards to the first one that maps to a
  node, so closures, anonymous callbacks, Rust nested fns and TS arrow consts inside functions are
  transparent, as in both tools. Function bodies: Python from `ast` (body only: decorators,
  annotations and defaults run in the enclosing scope, so `Annotated[T, Body()]` calls `Body` from
  the enclosing function, not from the decorated one); TS, Rust, Go from SCIP `enclosing_range`
  where the indexer emits it, otherwise a bracket scanner over comment- and string-masked text
  (TS function-valued consts and `const C = memo(() => …)` wrappers, Go interface methods).
  Decorators applied inside a function count as calls from it.
- `references` — type mentions in signatures (Python annotations from `ast`; TS/Rust/Go from the
  name to the body), fields (Python class-level annotations, TS class/interface members in type
  position, Rust struct/enum bodies, Go struct/interface bodies), Python decorators (the decorator
  name only, `get` in `@app.get(…)`), and Rust tuple-struct / variant constructors (`references`
  with context `constructor` in graph.json).
- `inherits` + `implements` — Python bases (head of each base expression), TS
  `extends`/`implements`, Rust `trait A: B` and `impl Trait for Type` headers, plus SCIP
  `is_implementation` between types (scip-python, scip-typescript). Go has none: implicit
  interface satisfaction is not an edge in either tool's model.
- `imports` — file pairs: a reference inside an import statement (Python `ast`; TS static
  `import`/`export … from`; Rust `use`, leaf segments only; Go import specs) to an in-repo symbol
  or module, mapped to the file defining it. A Go package maps to all its files (both tools'
  convention); a Rust `crate` path to `src/lib.rs`. Tool `imports`, `imports_from` and
  `re_exports` edges are compared through their endpoints' `source_file`.

**Scoring.** Precision = oracle-confirmed tool pairs / tool pairs whose both endpoints are
oracle-known nodes (imports: both files indexed). Recall = confirmed / oracle pairs expressible
in the tool's node space (oracle edges whose endpoints no definition maps to are counted
separately as unmappable). Tool edges to stubs, externals and unindexed files are counted as
unjudged. The confidence split reports precision of a tool's `EXTRACTED` and `INFERRED` edges
separately. `collapsed` counts oracle pairs that the tool does connect, but under another relation
(typically `contains` for a function calling its nested function, or `references` over `calls`):
the one-link-per-pair collapse hides them. The tool-independent count of oracle pairs carrying two
or more families is small: 1.3% of pairs on fastapi, ≤0.2% elsewhere.

## Results — 2026-10-13

dijkstral at `4d1f648a5320` (release build), Graphify 0.9.86 frozen references. P / R in %; tier
columns are precision with the judged edge count in parentheses.

| repo | family | oracle pairs | Graphify P | Graphify R | dijkstral P | dijkstral R | G EXTRACTED P | G INFERRED P | D EXTRACTED P | D INFERRED P | collapsed G / D |
|---|---|---|---|---|---|---|---|---|---|---|---|
| fastapi | calls | 2,511 | 97.1 | 88.0 | **99.1** | **90.2** | 97.0 (1,852) | 97.6 (423) | 99.9 (1,850) | 95.6 (434) | 57 / 37 |
| fastapi | references | 2,842 | 52.3 | 26.7 | **99.6** | **34.5** | 52.3 (1,453) | — | 99.6 (985) | — | 130 / 135 |
| fastapi | inherits | 98 | 98.0 | 99.0 | **100.0** | **100.0** | | | | | 0 / 0 |
| fastapi | imports | 2,756 | 98.4 | 98.4 | **99.3** | **99.0** | | | | | |
| graphify | calls | 15,318 | 99.2 | 94.5 | **100.0** | **97.0** | 99.2 (13,985) | 99.0 (614) | 100.0 (14,795) | 98.6 (74) | 434 / 432 |
| graphify | references | 135 | 97.8 | **67.4** | **100.0** | 31.1 | 97.8 (93) | — | 100.0 (42) | — | 28 / 35 |
| graphify | inherits | 27 | 100.0 | 100.0 | 100.0 | 100.0 | | | | | 0 / 0 |
| graphify | imports | 1,057 | 99.1 | 88.3 | **99.4** | **93.1** | | | | | |
| erpnext | calls | 20,952 | 96.7 | 92.5 | **98.4** | **97.0** | 97.8 (19,380) | 65.2 (650) | 99.1 (20,308) | 57.5 (334) | 355 / 307 |
| erpnext | references | 205 | 31.8 | 65.4 | **100.0** | **65.9** | 31.8 (422) | — | 100.0 (135) | — | 39 / 39 |
| erpnext | inherits | 513 | 99.4 | 99.4 | 99.4 | 99.4 | | | | | 0 / 0 |
| erpnext | imports | 3,219 | **99.9** | 96.7 | 99.8 | **98.9** | | | | | |
| excalidraw | calls | 5,389 | 95.1 | 92.5 | **97.8** | **93.6** | 94.8 (4,999) | 100.0 (247) | 97.7 (4,949) | 99.2 (254) | 2 / 0 |
| excalidraw | references | 2,932 | 92.5 | 14.8 | 92.5 | 14.8 | 92.5 (469) | — | 92.5 (468) | — | 38 / 39 |
| excalidraw | inherits | 9 | 100.0 | 100.0 | 100.0 | 100.0 | | | | | 0 / 0 |
| excalidraw | imports | 3,312 | 99.8 | 99.9 | 99.8 | 99.9 | | | | | |
| ripgrep | calls | 4,835 | 80.2 | 40.0 | **80.3** | **40.1** | 79.3 (2,282) | 97.6 (126) | 79.4 (2,291) | 97.6 (126) | 0 / 0 |
| ripgrep | references | 2,070 | 94.0 | 58.4 | **96.0** | **59.5** | 94.0 (1,285) | 100.0 (1) | 96.0 (1,282) | 100.0 (1) | 350 / 350 |
| ripgrep | inherits | 122 | 99.1 | 93.4 | 99.1 | 93.4 | | | | | 0 / 0 |
| ripgrep | imports | 194 | 76.8 | **61.3** | **94.5** | 53.1 | | | | | |
| monet | calls | 3,116 | 95.5 | 61.6 | **96.3** | **61.7** | 95.8 (1,819) | 92.7 (193) | 96.7 (1,799) | 92.7 (193) | 9 / 8 |
| monet | references | 1,200 | 91.0 | 64.2 | **91.4** | **66.6** | 91.0 (846) | — | 91.4 (874) | — | 17 / 20 |
| monet | inherits | 13 | 91.7 | 84.6 | 91.7 | 84.6 | | | | | 0 / 0 |
| monet | imports | 356 | 97.9 | 90.4 | **99.1** | 90.4 | | | | | |
| kubernetes ‡ | calls | 18,680 | 88.1 | 51.2 | **88.3** | **51.7** | 86.5 (8,946) | 95.7 (1,910) | 86.5 (8,948) | 96.3 (1,992) | 0 / 0 |
| kubernetes ‡ | references | 5,854 | 99.0 | 72.9 | **99.1** | **73.0** | 99.0 (4,311) | — | 99.1 (4,309) | — | 617 / 619 |
| kubernetes ‡ | imports | 21,771 | 91.3 | 100.0 | 91.3 | 100.0 | | | | | |

‡ `pkg/scheduler` + `staging/src/k8s.io/apimachinery` only. Import edges carry no confidence other
than EXTRACTED and `inherits` has no INFERRED edges, so those tier cells are empty.

Against the stage-2 exit criterion (precision ≥ Graphify at recall ≥ Graphify, per relation),
dijkstral meets it in 24 of the 27 rows. The exceptions: graphify-repo `references` (recall 31.1
vs 67.4), ripgrep `imports` (recall 53.1 vs 61.3), and erpnext `imports` (precision 99.8 vs 99.9,
5 false pairs against 4).

### What the disagreements are

- **Graphify binds names, not symbols** (fastapi `references` P 52.3 vs 99.6): `@app.post(…)`
  becomes a reference to an unrelated function `post()` in
  `tests/test_additional_properties_bool.py` (256 edges); `Path()`, `Depends()`, `Body()`,
  `Form()` in annotations reference the classes in `fastapi.params` instead of the functions in
  `fastapi.param_functions` (364 edges); a `websocket: WebSocket` parameter references a function
  `websocket()` in `docs_src/app_testing/tutorial002_py310.py` (15 edges). On erpnext, methods decorated with
  `@ERPNextTestSuite.change_settings(…)` reference the module function
  `erpnext/tests/utils.py change_settings` (284 of Graphify's 288 reference FPs). dijkstral
  resolves all of these through the imports.
- **Rust method calls by bare name** (both tools, ripgrep `calls` P ≈80): `x.update()` binds to
  the last `update` in the file (`count_update -calls-> wordregexp_update`, defs.rs:L1296), and
  `.build()` in `DecompressionMatcher::new` to the wrong builder (decompress.rs:L168). Recall 40%
  is mostly by design: cross-file method calls and calls inside macros are dropped.
- **Go method calls by bare name** (kubernetes `calls` P 88): `c.evictionList.Remove(…)`
  (`container/list`) becomes `LRUExpireCache.add -calls-> LRUExpireCache.Remove`
  (lruexpirecache.go:L89); `handler.OnAdd` on an interface binds to the test file's mock.
- **Python nested functions** (Graphify): calls inside a nested `async def app` are also attributed
  to the outer function (`get_request_handler -calls-> solve_dependencies`, routing.py:L499).
- **dijkstral's INFERRED fallback ignores external imports**: `Response(…)` and `HTTPException(…)`
  imported from `starlette` become `-calls/INFERRED->` the in-repo `fastapi.openapi.models.Response`
  (`fastapi/exception_handlers.py:L14`) and `fastapi.exceptions.HTTPException`
  (`fastapi/security/api_key.py:L41`): 13 of its 20 fastapi call FPs. On erpnext its INFERRED calls
  are 57.5% precise (142 of its 323 call FPs; frappe functions bound to same-named in-repo ones).
- **dijkstral Rust `use` leaves resolved across crates by bare name**: `use termcolor::Color` in
  `crates/printer/src/color.rs` links to `crates/core/flags/defs.rs Color`. Graphify's own import
  errors are the Q6 file-stem remap: `use std::path::Path` → `crates/printer/src/path.rs`
  (24 of its 36 ripgrep import FPs).
- **Go `_test.go` files** (both, kubernetes `imports` P 91.3): importing a package links to its
  external `package x_test` files (quirk G4).
- **dijkstral misses** graphify-repo `references` that Graphify finds (signature types such as
  `-> MinHash`, dataclass-typed parameters), and ripgrep imports Graphify reaches through `pub use`
  re-exports in `lib.rs`.
- Recall gaps shared by both tools are by design and visible per context in the JSON: Python
  decorators resolved through an instance (`@app.get` → `FastAPI.get`: 36 / 1,498 on fastapi),
  Python dataclass field types (0%), TS signature types outside class members (≈10% on
  excalidraw), Rust tuple constructors across files.

## Limitations

- **What SCIP cannot see**: dynamic dispatch (a call through an interface or trait object resolves
  to the interface method, not the implementations), duck typing and unannotated receivers
  (Pyright cannot type `obj.m()`; such tool edges, often INFERRED, are counted as false
  positives, so INFERRED precision is a lower bound), monkey-patching, reflection and `getattr`,
  string-based dispatch (`frappe.call("module.fn")`), and code generated at run time. Conversely,
  rust-analyzer expands macros, so calls inside `println!` and macro-generated functions are in the
  oracle while neither tool can see them.
- **Third-party code is not installed**, so references into dependencies are unresolved and out
  of scope; erpnext's `frappe` is the largest such boundary.
- **Heuristic spans** for TS, Rust and Go signatures, field bodies, TS function-valued consts and
  import statements come from a bracket scanner over masked text, not a parser. After masking,
  1 of 502 excalidraw files and 8 of 98 ripgrep files (macro definitions, raw strings) have
  unbalanced brackets; Python spans come from `ast` and are exact.
- **Call syntax** decides `calls` versus a plain mention; a function passed as a value
  (`map(f, xs)`) is a mention, which both tools model as `indirect_call` (not scored).
- **Granularity conventions** are the tools': imports are file pairs, Go packages expand to all
  their files, field references start at the owning type. Go implicit interface satisfaction and
  Go embedding are not scored.
- **Kubernetes** is a two-directory subset; Java is not covered.
