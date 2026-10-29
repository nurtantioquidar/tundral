#!/usr/bin/env bash
# Prepare the benchmark environment outside the source tree:
#   $DIJKSTRAL_BENCH_HOME/tools/graphify-<ver>/   pinned Graphify in an isolated venv
#   $DIJKSTRAL_BENCH_HOME/corpus/<name>/          each corpus repo at its pinned commit
# Idempotent: existing tools and checkouts at the right commit are left alone.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(git -C "$here" rev-parse --show-toplevel)"
bench_home="${DIJKSTRAL_BENCH_HOME:-$HOME/.cache/dijkstral-bench}"
graphify_version="${GRAPHIFY_VERSION:-0.9.86}"
only="${1:-}"

mkdir -p "$bench_home/tools" "$bench_home/corpus"

# From 0.9.97 the `sql` extra installs tree-sitter-sql; without it Graphify gives `.sql` files no
# nodes, and dijkstral always extracts them (cli/docs/graphify-format.md §45.1).
extras="leiden,mcp"
[[ "$(printf '%s\n' 0.9.97 "$graphify_version" | sort -V | head -1)" == 0.9.97 ]] && extras+=",sql"
tool="$bench_home/tools/graphify-$graphify_version"
if [[ ! -x "$tool/bin/graphify" || "$(cat "$tool/.bench-extras" 2>/dev/null)" != "$extras" ]]; then
  echo "installing graphifyy[$extras]==$graphify_version into $tool"
  # Python 3.13 so the leiden extra uses graspologic-native, the backend Graphify prefers.
  [[ -x "$tool/bin/python" ]] || uv venv -q --python 3.13 "$tool"
  VIRTUAL_ENV="$tool" uv pip install -q "graphifyy[$extras]==$graphify_version"
  echo "$extras" >"$tool/.bench-extras"
fi

# name|url|commit triples from corpus.toml (flat format, parsed without extra dependencies).
python3 - "$here/corpus.toml" <<'PY' |
import sys, tomllib
for r in tomllib.load(open(sys.argv[1], "rb"))["repo"]:
    print(f'{r["name"]}|{r["url"]}|{r["commit"]}')
PY
while IFS='|' read -r name url commit; do
  [[ -n "$only" && "$only" != "$name" ]] && continue
  dest="$bench_home/corpus/$name"
  if [[ -d "$dest/.git" ]] && [[ "$(git -C "$dest" rev-parse HEAD)" == "$(git -C "$dest" rev-parse "$commit^{commit}" 2>/dev/null || true)" ]]; then
    echo "ok       $name @ ${commit:0:12}"
    continue
  fi
  echo "fetching $name @ ${commit:0:12}"
  rm -rf "$dest"
  git init -q "$dest"
  if [[ "$url" == local:monet ]]; then
    git -C "$dest" fetch -q --depth 1 "file://$repo_root" "$commit" 2>/dev/null \
      || git -C "$dest" fetch -q "file://$repo_root" "$commit"
  else
    git -C "$dest" fetch -q --depth 1 "$url" "$commit"
  fi
  git -C "$dest" -c advice.detachedHead=false checkout -q FETCH_HEAD
  # Keep both tools' output out of each other's corpus and out of `git status`.
  printf 'graphify-out/\ndijkstral-out/\n' >> "$dest/.git/info/exclude"
done

echo "bench home: $bench_home"
