#!/usr/bin/env bash
# Smoke test of a running service image (compose.yaml), the container counterpart of
# tests/e2e.rs: install, build, MCP queries, a push rebuild and uninstall, over HTTP.
#
#   export DIJKSTRAL_SERVICE_WEBHOOK_SECRET=… DIJKSTRAL_SERVICE_INTERNAL_TOKEN=…
#   export DIJKSTRAL_LOCAL_SOURCE=$(mktemp -d)
#   scripts/smoke.sh seed                  # bare repository acme/demo, before the service starts
#   docker compose up -d --build
#   scripts/smoke.sh                       # against http://127.0.0.1:8095, or $DIJKSTRAL_URL
set -euo pipefail

: "${DIJKSTRAL_SERVICE_WEBHOOK_SECRET:?}" "${DIJKSTRAL_SERVICE_INTERNAL_TOKEN:?}" "${DIJKSTRAL_LOCAL_SOURCE:?}"
url=${DIJKSTRAL_URL:-http://127.0.0.1:8095}
here=$(cd "$(dirname "$0")/.." && pwd)
bare=$DIJKSTRAL_LOCAL_SOURCE/acme/demo.git
work=$DIJKSTRAL_LOCAL_SOURCE/.work

git_() {
  GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.com \
    GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.com git -C "$work" "$@"
}

if [[ ${1:-} == seed ]]; then
  rm -rf "$bare" "$work"
  mkdir -p "$bare"
  git init -q --bare -b main "$bare"
  cp -R "$here/../cli/tests/fixtures/python" "$work"
  git_ init -q -b main
  git_ add -A
  git_ commit -q -m one
  git_ push -q "$bare" main
  # The service runs as another user; it reads the repository through a read-only mount.
  chmod -R a+rX "$DIJKSTRAL_LOCAL_SOURCE"
  echo "seeded $bare"
  exit 0
fi

fail() { echo "FAIL: $*" >&2; exit 1; }

# POST a signed webhook; prints the HTTP status.
webhook() {
  local sig
  sig=$(printf '%s' "$2" | openssl dgst -sha256 -hmac "$DIJKSTRAL_SERVICE_WEBHOOK_SECRET" -r | cut -d' ' -f1)
  curl -s -o /dev/null -w '%{http_code}' -X POST "$url/github/webhook" \
    -H "x-github-event: $1" -H "x-hub-signature-256: sha256=$sig" --data-binary "$2"
}

internal() {
  curl -sf -X "$1" "$url$2" -H "authorization: Bearer $DIJKSTRAL_SERVICE_INTERNAL_TOKEN" \
    -H 'content-type: application/json' ${3:+--data-binary "$3"}
}

# MCP call with token $1 to repository $2; prints the HTTP status, the body goes to $out.
out=$(mktemp)
trap 'rm -f "$out"' EXIT
mcp() {
  curl -s -o "$out" -w '%{http_code}' -X POST "$url/mcp/$2" \
    ${1:+-H "authorization: Bearer $1"} -H 'content-type: application/json' \
    -H 'accept: application/json, text/event-stream' --data-binary "$3"
}

# Waits until repository 100 is ready at a commit other than $1; prints the commit.
built() {
  for _ in $(seq 600); do
    local r
    r=$(internal GET "/internal/accounts/$account/repos" | jq -c '.repos[] | select(.id == 100)')
    [[ $(jq -r .state <<<"$r") == failed ]] && fail "build failed: $r"
    if [[ $(jq -r .state <<<"$r") == ready && $(jq -r .built_commit <<<"$r") != "$1" ]]; then
      jq -r .built_commit <<<"$r"
      return
    fi
    sleep 0.2
  done
  fail "no build"
}

[[ $(curl -sf "$url/healthz") == ok ]] || fail "healthz"

install='{"action":"created","installation":{"id":7,"account":{"login":"acme"}},"repositories":[{"id":100,"full_name":"acme/demo"}]}'
[[ $(DIJKSTRAL_SERVICE_WEBHOOK_SECRET=wrong webhook installation "$install") == 401 ]] ||
  fail "a wrongly signed webhook was accepted"
[[ $(webhook installation "$install") == 202 ]] || fail "installation webhook"
account=$(internal POST /internal/accounts '{"github_id":1,"login":"alice","installation_ids":[7]}' | jq -r .account_id)
first=$(built "")
echo "built $first"

token=$(internal POST "/internal/accounts/$account/tokens" '{"name":"smoke"}' | jq -r .token)
init='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}'
[[ $(mcp "" acme/demo "$init") == 401 ]] || fail "MCP without a token"
[[ $(mcp "$token" acme/other "$init") == 404 ]] || fail "MCP on an unknown repository"
[[ $(curl -s -o /dev/null -w '%{http_code}' "$url/internal/accounts/$account/repos") == 404 ]] ||
  fail "private API without the internal token"
[[ $(mcp "$token" acme/demo "$init") == 200 ]] || fail "MCP initialize: $(cat "$out")"
[[ $(mcp "$token" acme/demo '{"jsonrpc":"2.0","id":2,"method":"tools/list"}') == 200 ]] || fail "tools/list"
[[ $(jq -r '.result.tools[0].name' "$out") == context ]] || fail "context is not the first tool"
ask='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"context","arguments":{"question":"Who calls slugify and what does it call?"}}}'
mcp "$token" acme/demo "$ask" >/dev/null
jq -r '.result.content[0].text' "$out" | grep -q '^## slugify()  pkg/util.py:14$' || fail "context: $(cat "$out")"

# A push to the default branch rebuilds.
printf 'from pkg.util import slugify\n\n\ndef shout_title(text):\n    return slugify(text).upper()\n' >"$work/pkg/extra.py"
git_ add -A
git_ commit -q -m two
git_ push -q "$bare" main
push='{"ref":"refs/heads/main","after":"x","installation":{"id":7},"repository":{"id":100,"default_branch":"main"}}'
[[ $(webhook push "$push") == 202 ]] || fail "push webhook"
second=$(built "$first")
echo "rebuilt $second"
ask='{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"context","arguments":{"question":"Who calls `shout_title`?"}}}'
mcp "$token" acme/demo "$ask" >/dev/null
grep -q shout_title "$out" || fail "rebuilt graph: $(cat "$out")"

# Uninstalling removes the repository.
[[ $(webhook installation '{"action":"deleted","installation":{"id":7}}') == 202 ]] || fail "uninstall webhook"
[[ $(mcp "$token" acme/demo "$init") == 404 ]] || fail "MCP after uninstall"
echo "smoke: ok"
