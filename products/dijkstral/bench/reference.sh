#!/usr/bin/env bash
# Freeze Graphify's cold `extract --code-only` graph for each corpus repo as the parity reference:
#   $DIJKSTRAL_BENCH_HOME/reference/<repo>.graph.json            the pinned Graphify (0.9.86)
#   $DIJKSTRAL_BENCH_HOME/reference-<version>/<repo>.graph.json  GRAPHIFY_VERSION=<version>
# Usage: [GRAPHIFY_VERSION=0.9.97] bench/reference.sh [repo ...]   (default: every repo in corpus.toml)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
bench_home="${DIJKSTRAL_BENCH_HOME:-$HOME/.cache/dijkstral-bench}"
version="${GRAPHIFY_VERSION:-0.9.86}"
graphify="$bench_home/tools/graphify-$version/bin/graphify"
ref="$bench_home/reference"
[[ "$version" == 0.9.86 ]] || ref+="-$version"
mkdir -p "$ref"

repos=("$@")
if [[ ${#repos[@]} -eq 0 ]]; then
  while IFS= read -r r; do repos+=("$r"); done < <(python3 -c '
import sys, tomllib
for r in tomllib.load(open(sys.argv[1], "rb"))["repo"]: print(r["name"])' "$here/corpus.toml")
fi

for repo in "${repos[@]}"; do
  dir="$bench_home/corpus/$repo"
  rm -rf "$dir/graphify-out"
  GRAPHIFY_QUERY_LOG_DISABLE=1 GRAPHIFY_MAX_GRAPH_BYTES=8GB "$graphify" extract "$dir" --code-only >/dev/null 2>&1
  cp "$dir/graphify-out/graph.json" "$ref/$repo.graph.json"
  echo "reference $repo ($version): $(du -h "$ref/$repo.graph.json" | cut -f1)"
done
