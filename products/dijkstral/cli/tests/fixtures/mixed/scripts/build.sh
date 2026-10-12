#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common.sh"
. ./common.sh

clean() {
  log "cleaning"
  rm -rf dist
}

build() {
  clean
  log "building"
  compile_all
}

function compile_all {
  python3 app/main.py
}

build "$@"
