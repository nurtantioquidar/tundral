#!/usr/bin/env bash
# Build a static Linux `butterfly` binary for Terminal-Bench containers.
#
# Uses the official rust:alpine image (musl target, static by default) inside
# Docker so the host toolchain is never involved. Cargo's registry and the
# Linux target dir live in named volumes: the host `target/` is untouched.
#
#   bench/harbor/build-linux.sh                 # host architecture (arm64 on Apple Silicon)
#   PLATFORM=linux/amd64 bench/harbor/build-linux.sh   # for x86 clouds (Modal etc.)
#
# Output: bench/harbor/dist/butterfly-linux-<arch>, plus a .sha256 beside it —
# the hash is the harness version recorded with every benchmark run.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CLI_DIR="$(cd "$HERE/../../cli" && pwd)"
DIST="$HERE/dist"
PLATFORM="${PLATFORM:-}"
RUST_IMAGE="${RUST_IMAGE:-rust:1-alpine}"

mkdir -p "$DIST"
platform_args=()
if [[ -n "$PLATFORM" ]]; then
  platform_args=(--platform "$PLATFORM")
fi

docker run --rm "${platform_args[@]}" \
  -v "$CLI_DIR":/src:ro \
  -v butterfly-cargo-registry:/usr/local/cargo/registry \
  -v butterfly-linux-target:/target \
  -v "$DIST":/dist \
  -e CARGO_TARGET_DIR=/target \
  -w /src \
  "$RUST_IMAGE" \
  sh -euc '
    apk add --no-cache build-base musl-dev perl >/dev/null
    cargo build --release --locked -p butterfly
    arch="$(uname -m)"
    cp /target/release/butterfly "/dist/butterfly-linux-$arch"
    (cd /dist && sha256sum "butterfly-linux-$arch" > "butterfly-linux-$arch.sha256")
    echo "built /dist/butterfly-linux-$arch"
  '

ls -la "$DIST"
