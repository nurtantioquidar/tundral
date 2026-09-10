#!/usr/bin/env bash
# Butterfly → Nimble → fake upstream, all on loopback, no keys, no money.
# Proves the nimble/v1 wiring end to end: metadata out, routing facts back,
# verification, and feedback joined into Nimble's stats.
#
#   products/nimble/router/scripts/loopback-smoke.sh   # needs a built butterfly (cargo build)
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
BUTTERFLY="${BUTTERFLY_BIN:-$HERE/../../butterfly/cli/target/debug/butterfly}"
WORK="$(mktemp -d)"
UPSTREAM_PID=""; NIMBLE_PID=""
cleanup() {
  [[ -n "$UPSTREAM_PID" ]] && kill "$UPSTREAM_PID" 2>/dev/null || true
  [[ -n "$NIMBLE_PID" ]] && kill "$NIMBLE_PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

python3 - "$HERE/nimble.config.json" "$WORK/nimble.config.json" "$WORK/nimble.db" <<'PY'
import json, sys
cfg = json.load(open(sys.argv[1]))
for p in cfg["providers"]:
    p["base_url"] = "http://127.0.0.1:8093/v1"; p.pop("base_url_env", None)
cfg["db_path"] = sys.argv[3]
json.dump(cfg, open(sys.argv[2], "w"))
PY

cd "$HERE"
bun scripts/fake-upstream.ts > "$WORK/upstream.log" 2>&1 < /dev/null &
UPSTREAM_PID=$!
DEEPSEEK_API_KEY=x ZAI_API_KEY=x OPENAI_API_KEY=x NIMBLE_CONFIG="$WORK/nimble.config.json" bun src/main.ts > "$WORK/nimble.log" 2>&1 < /dev/null &
NIMBLE_PID=$!
for _ in $(seq 1 40); do
  curl -sf http://127.0.0.1:8092/health >/dev/null 2>&1 && curl -sf http://127.0.0.1:8093/seen >/dev/null 2>&1 && break
  sleep 0.5
done

mkdir -p "$WORK/repo" && (cd "$WORK/repo" && git init -q)
report="$(cd "$WORK/repo" && env -u DEEPSEEK_API_KEY -u NIMBLE_API_KEY HOME="$WORK/home" "$BUTTERFLY" \
  --model nimble --json --auto --no-save --quiet-reasoning --task-type debugging \
  --max-attempts 2 --verify-cmd pwd -p "say ok")"
python3 - "$report" <<'PY'
import json, sys
r = json.loads(sys.argv[1])
assert r["provider"] == "nimble" and r["verified"] is True, r
facts = r["routing"]
assert facts["version"] == "nimble/v1" and facts["selected_model"], facts
print(f"butterfly: response={r['response']!r} verified={r['verified']} routed_to={facts['selected_model']} reason={facts['routing_reason']!r}")
PY
curl -s "http://127.0.0.1:8092/v1/stats" | python3 -c "
import json, sys
stats = json.load(sys.stdin)['stats']
assert stats and stats[0]['n'] == 1 and stats[0]['successes'] == 1, stats
print('nimble: feedback joined ->', stats[0]['model'], stats[0]['task_type'], 'success_rate', stats[0]['success_rate'])"
curl -s http://127.0.0.1:8093/seen | python3 -c "
import json, sys
seen = json.load(sys.stdin)
assert seen and not seen[0]['has_nimble'], seen
print('upstream: received model', seen[0]['model'], 'via', seen[0].get('api', 'chat'), 'without the nimble extension')"
echo "loopback smoke OK"
