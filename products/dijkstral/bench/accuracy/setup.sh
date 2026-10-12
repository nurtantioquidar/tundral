#!/usr/bin/env bash
# Install pinned SCIP indexers outside the source tree, under
#   $DIJKSTRAL_BENCH_HOME/tools/scip/        (default ~/.cache/dijkstral-bench)
# and record what was installed in tools/scip/VERSIONS. Idempotent.
#
#   scip            sourcegraph/scip release binary (`scip print --json`, `scip stats`)
#   scip-go         sourcegraph/scip-go release binary
#   rust-analyzer   rust-lang/rust-analyzer release binary (`rust-analyzer scip`)
#   scip-python     npm @sourcegraph/scip-python, installed locally into node/
#   scip-typescript npm @sourcegraph/scip-typescript, installed locally into node/
#
# Java is not indexed: no corpus repo is Java.
set -euo pipefail

SCIP_VERSION="${SCIP_VERSION:-v0.10.0}"
SCIP_GO_VERSION="${SCIP_GO_VERSION:-v0.2.7}"
RUST_ANALYZER_VERSION="${RUST_ANALYZER_VERSION:-2026-09-28}"
SCIP_PYTHON_VERSION="${SCIP_PYTHON_VERSION:-0.6.6}"
SCIP_TYPESCRIPT_VERSION="${SCIP_TYPESCRIPT_VERSION:-0.4.0}"

bench_home="${DIJKSTRAL_BENCH_HOME:-$HOME/.cache/dijkstral-bench}"
dest="$bench_home/tools/scip"
mkdir -p "$dest/bin" "$dest/node"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) goarch=darwin-arm64; ratriple=aarch64-apple-darwin ;;
  Linux-x86_64) goarch=linux-amd64; ratriple=x86_64-unknown-linux-gnu ;;
  Linux-aarch64) goarch=linux-arm64; ratriple=aarch64-unknown-linux-gnu ;;
  *) echo "unsupported platform $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

# fetch_tarball <name> <version> <url>: verify the release's .sha256 and unpack <name> into bin/.
fetch_tarball() {
  local name="$1" version="$2" url="$3"
  local stamp="$dest/bin/.$name.version"
  if [[ -x "$dest/bin/$name" && "$(cat "$stamp" 2>/dev/null)" == "$version" ]]; then
    echo "ok       $name $version"; return
  fi
  echo "fetching $name $version"
  local tmp; tmp="$(mktemp -d)"
  curl -fsSL -o "$tmp/a.tar.gz" "$url"
  curl -fsSL -o "$tmp/a.sha256" "$url.sha256"
  local want got
  want="$(awk '{print $1}' "$tmp/a.sha256")"
  got="$(shasum -a 256 "$tmp/a.tar.gz" | awk '{print $1}')"
  [[ "$want" == "$got" ]] || { echo "sha256 mismatch for $name: $got != $want" >&2; exit 1; }
  mkdir "$tmp/x"
  tar -xzf "$tmp/a.tar.gz" -C "$tmp/x"
  install -m 0755 "$(find "$tmp/x" -type f -name "$name" | head -1)" "$dest/bin/$name"
  echo "$version" > "$stamp"
  rm -rf "$tmp"
}

fetch_tarball scip "$SCIP_VERSION" \
  "https://github.com/sourcegraph/scip/releases/download/$SCIP_VERSION/scip-$goarch.tar.gz"
fetch_tarball scip-go "$SCIP_GO_VERSION" \
  "https://github.com/sourcegraph/scip-go/releases/download/$SCIP_GO_VERSION/scip-go-$goarch.tar.gz"

ra_stamp="$dest/bin/.rust-analyzer.version"
if [[ -x "$dest/bin/rust-analyzer" && "$(cat "$ra_stamp" 2>/dev/null)" == "$RUST_ANALYZER_VERSION" ]]; then
  echo "ok       rust-analyzer $RUST_ANALYZER_VERSION"
else
  echo "fetching rust-analyzer $RUST_ANALYZER_VERSION"
  curl -fsSL "https://github.com/rust-lang/rust-analyzer/releases/download/$RUST_ANALYZER_VERSION/rust-analyzer-$ratriple.gz" \
    | gunzip > "$dest/bin/rust-analyzer.tmp"
  chmod 0755 "$dest/bin/rust-analyzer.tmp"
  mv "$dest/bin/rust-analyzer.tmp" "$dest/bin/rust-analyzer"
  echo "$RUST_ANALYZER_VERSION" > "$ra_stamp"
fi

# npm indexers: exact versions, installed into an isolated package directory.
npm_pkgs=("@sourcegraph/scip-python@$SCIP_PYTHON_VERSION" "@sourcegraph/scip-typescript@$SCIP_TYPESCRIPT_VERSION")
if [[ "$(cat "$dest/node/.versions" 2>/dev/null)" == "${npm_pkgs[*]}" && -x "$dest/node/node_modules/.bin/scip-python" ]]; then
  echo "ok       ${npm_pkgs[*]}"
else
  echo "installing ${npm_pkgs[*]}"
  [[ -f "$dest/node/package.json" ]] || echo '{"name":"dijkstral-bench-scip","private":true}' > "$dest/node/package.json"
  npm install --prefix "$dest/node" --save-exact --no-audit --no-fund --silent "${npm_pkgs[@]}"
  echo "${npm_pkgs[*]}" > "$dest/node/.versions"
fi

{
  echo "# installed by bench/accuracy/setup.sh on $(date -u +%Y-%m-%d)"
  echo "scip            $("$dest/bin/scip" --version 2>&1 | head -1)"
  echo "scip-go         $("$dest/bin/scip-go" --version 2>&1 | head -1)"
  echo "rust-analyzer   $("$dest/bin/rust-analyzer" --version 2>&1 | head -1)"
  echo "scip-python     $("$dest/node/node_modules/.bin/scip-python" --version 2>&1 | head -1)"
  echo "scip-typescript $("$dest/node/node_modules/.bin/scip-typescript" --version 2>&1 | head -1)"
  echo "node            $(node --version)"
  echo "go              $(go version 2>/dev/null || echo missing)"
  echo "cargo           $(cargo --version 2>/dev/null || echo missing)"
} > "$dest/VERSIONS"
cat "$dest/VERSIONS"
