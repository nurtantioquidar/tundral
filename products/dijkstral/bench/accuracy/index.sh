#!/usr/bin/env bash
# Index one corpus repo with the pinned SCIP indexers (see setup.sh):
#
#   bench/accuracy/index.sh <repo> [--force]
#
# Output: $DIJKSTRAL_BENCH_HOME/scip/<repo>/parts/<part>.scip, one file per indexer run, plus
# parts/<part>.log. Each part's metadata.project_root says which directory its document paths are
# relative to; oracle.py maps them back to repo-relative paths. Indexers never run inside the
# corpus checkout: the repo is copied to $DIJKSTRAL_BENCH_HOME/scip/<repo>/src first (indexers
# write pyrightconfig.json, Cargo target/ and caches there). The only exception is kubernetes,
# which is too large to copy and whose Go indexer writes nothing into the module.
set -euo pipefail

repo="${1:?usage: index.sh <repo> [--force]}"
force="${2:-}"
bench_home="${DIJKSTRAL_BENCH_HOME:-$HOME/.cache/dijkstral-bench}"
tools="$bench_home/tools/scip"
corpus="$bench_home/corpus/$repo"
out="$bench_home/scip/$repo"
src="$out/src"
parts="$out/parts"

[[ -d "$corpus" ]] || { echo "no corpus checkout at $corpus (run bench/setup.sh $repo)" >&2; exit 1; }
[[ -x "$tools/bin/scip" ]] || { echo "indexers missing (run bench/accuracy/setup.sh)" >&2; exit 1; }
commit="$(git -C "$corpus" rev-parse HEAD)"
mkdir -p "$parts"

scip_python="$tools/node/node_modules/.bin/scip-python"
scip_typescript="$tools/node/node_modules/.bin/scip-typescript"
rust_analyzer="$tools/bin/rust-analyzer"
scip_go="$tools/bin/scip-go"

copy_src() {
  echo "copying $repo -> $src"
  mkdir -p "$src"
  rsync -a --delete \
    --exclude '/.git' --exclude 'graphify-out' --exclude 'dijkstral-out' --exclude 'node_modules' \
    "$corpus/" "$src/"
}

# run_part <name> <dir> <cmd...>: run an indexer in <dir> with output parts/<name>.scip.
run_part() {
  local name="$1" dir="$2"; shift 2
  local target="$parts/$name.scip"
  if [[ -s "$target" && "$force" != "--force" ]]; then
    echo "ok       $repo/$name ($(du -h "$target" | cut -f1))"; return
  fi
  echo "indexing $repo/$name in ${dir#"$out/"}"
  local start=$SECONDS
  if (cd "$dir" && "$@") > "$parts/$name.log" 2>&1; then
    echo "  done in $((SECONDS - start)) s, $(du -h "$target" | cut -f1)"
  else
    echo "  FAILED after $((SECONDS - start)) s; see $parts/$name.log" >&2
    tail -5 "$parts/$name.log" >&2
  fi
}

# Pyright (inside scip-python 0.6.6) excludes `**/.*` by default, which also matches the
# `.cache` in the bench home path, and it only knows Python <= 3.12. Pin both in a
# pyrightconfig.json in the scratch copy (it overrides any [tool.pyright] in pyproject.toml).
python_part() {
  local name="$1" dir="$2"
  cat > "$dir/pyrightconfig.json" <<'JSON'
{
  "include": ["."],
  "exclude": ["**/node_modules", "**/__pycache__", "**/.venv", "**/target", "dijkstral-out", "graphify-out"],
  "pythonVersion": "3.12"
}
JSON
  run_part "$name" "$dir" "$scip_python" index --project-name "$repo" --project-version "${commit:0:12}" \
    --cwd "$dir" --output "$parts/$name.scip" --show-progress-rate-limit 30
}

typescript_part() {
  local name="$1" dir="$2"; shift 2
  run_part "$name" "$dir" "$scip_typescript" index --cwd "$dir" --output "$parts/$name.scip" --no-progress-bar "$@"
}

rust_part() {
  local name="$1" dir="$2"
  # rust-analyzer writes the index into the current directory unless --output is given; it runs
  # build scripts and proc macros through cargo, which writes target/ inside the scratch copy.
  run_part "$name" "$dir" "$rust_analyzer" scip "$dir" --output "$parts/$name.scip"
}

case "$repo" in
  fastapi|graphify|erpnext)
    copy_src
    python_part python "$src"
    ;;
  excalidraw)
    copy_src
    # Root tsconfig.json covers packages/* and excalidraw-app through `paths`; examples/ and
    # dev-docs/ are excluded by it and stay outside the oracle's scope.
    typescript_part typescript "$src"
    ;;
  ripgrep)
    copy_src
    rust_part rust "$src"
    ;;
  monet)
    copy_src
    python_part python "$src"
    # One TypeScript project per tsconfig.json. No dependencies are installed, but the bun
    # workspace packages are linked into node_modules so cross-package `@monet/*` imports resolve
    # (third-party packages stay unresolved and outside the oracle's scope).
    python3 - "$src" <<'PY'
import json, os, sys
src = sys.argv[1]
for root, dirs, files in os.walk(src):
    dirs[:] = [d for d in dirs if d not in ("node_modules", "target", ".git")]
    if "package.json" in files and root != src:
        name = json.load(open(os.path.join(root, "package.json"))).get("name")
        if name:
            link = os.path.join(src, "node_modules", name)
            os.makedirs(os.path.dirname(link), exist_ok=True)
            if not os.path.lexists(link):
                os.symlink(os.path.relpath(root, os.path.dirname(link)), link)
PY
    ts_projects=()
    while IFS= read -r p; do ts_projects+=("$p"); done < <(
      cd "$src" && find . -name tsconfig.json -not -path '*/node_modules/*' | sed 's|/tsconfig.json$||; s|^\./||' | sort
    )
    typescript_part typescript "$src" "${ts_projects[@]}"
    for ws in products/butterfly/cli products/cocoon/cli; do
      [[ -f "$src/$ws/Cargo.toml" ]] && rust_part "rust-$(echo "$ws" | tr / -)" "$src/$ws"
    done
    ;;
  kubernetes)
    # 2.4 GB checkout: index in place (scip-go only reads the module and writes the index to
    # --output). Indexing all of kubernetes is impractical, so index the scheduler and
    # apimachinery, a dense subset with many cross-package calls and interface implementations.
    patterns=(./pkg/scheduler/... ./staging/src/k8s.io/apimachinery/...)
    run_part go "$corpus" "$scip_go" index --output "$parts/go.scip" --module-version "${commit:0:12}" "${patterns[@]}"
    ;;
  *)
    echo "unknown repo $repo" >&2; exit 1 ;;
esac

echo "$commit" > "$out/COMMIT"
ls -la "$parts"
