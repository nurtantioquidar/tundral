#!/usr/bin/env bash
# Launch the butterfly REPL directly (not through Bun's workspace filter) so stdin
# and stdout keep their terminal identity. Root `bun run dev:harness` calls this.
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo run -p butterfly -- "$@"
