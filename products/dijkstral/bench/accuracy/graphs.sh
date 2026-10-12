#!/usr/bin/env bash
# Build dijkstral's `extract --code-only` graph for each corpus repo, outside the source tree:
#
#   bench/accuracy/graphs.sh [repo ...]        (default: every repo in corpus.toml)
#
# Writes $DIJKSTRAL_BENCH_HOME/accuracy/dijkstral/<repo>/dijkstral-out/graph.json and records the
# dijkstral commit and binary sha256 next to it (BUILD). Graphify's side is the frozen reference
# graph from bench/reference.sh. Uses the release binary at cli/target/release/dijkstral.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
bench_home="${DIJKSTRAL_BENCH_HOME:-$HOME/.cache/dijkstral-bench}"
bin="${DIJKSTRAL_BIN:-$here/../../cli/target/release/dijkstral}"
[[ -x "$bin" ]] || { echo "build dijkstral first: cd products/dijkstral/cli && cargo build --release" >&2; exit 1; }

repos=("$@")
if [[ ${#repos[@]} -eq 0 ]]; then
  while IFS= read -r r; do repos+=("$r"); done < <(
    python3 -c 'import sys, tomllib; [print(r["name"]) for r in tomllib.load(open(sys.argv[1], "rb"))["repo"]]' \
      "$here/../corpus.toml"
  )
fi

commit="$(git -C "$here" rev-parse --short=12 HEAD)"
dirty="$(git -C "$here/../../cli" status --porcelain . | head -1)"
sha="$(shasum -a 256 "$bin" | cut -c1-16)"
for repo in "${repos[@]}"; do
  out="$bench_home/accuracy/dijkstral/$repo"
  mkdir -p "$out"
  rm -rf "$out/dijkstral-out"
  start=$SECONDS
  "$bin" extract "$bench_home/corpus/$repo" --code-only --out "$out" > "$out/extract.log" 2>&1
  printf 'commit %s%s\nbinary_sha256 %s\n' "$commit" "${dirty:+ (cli dirty)}" "$sha" > "$out/BUILD"
  echo "$repo: $((SECONDS - start)) s -> $out/dijkstral-out/graph.json"
done
