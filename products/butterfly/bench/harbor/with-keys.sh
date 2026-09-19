#!/usr/bin/env bash
# Run a command with provider API keys loaded from 1Password (`op` CLI) into the
# environment. Values never touch stdout, logs, or files; each key is read
# straight from the vault into the child process environment.
#
#   bench/harbor/with-keys.sh ./run-matrix.sh
#   bench/harbor/with-keys.sh cargo test -p butterfly-core --test live_deepseek -- --nocapture
#
# Items (vault Personal): api_key_deepseek__monet.harness, api_key_zai__monet.harness,
# api_key_openai__monet.harness, api_key_meta__monet.harness, and — not a model
# provider, used only by optional failure triage —
# api_key_typesafeai__butterfly.harness. Override with OP_VAULT / OP_ITEM_<PROVIDER>.
#
# Unattended runs: the macOS login Keychain is consulted FIRST (no Touch ID per
# read while the user is logged in; encrypted at rest). Add a key once, typing
# it at the prompt so it never appears in shell history or a file:
#   security add-generic-password -U -a monet -s api_key_zai__monet.harness -w
# Then 1Password (`op`) is the fallback for anything the Keychain lacks.
#
# Or an env file OUTSIDE the repository, owner-readable only, one KEY=value per
# line (default ~/.config/monet/bench.env, override MONET_KEYS_FILE). Refused if
# it is group/world readable. Plaintext at rest: fine for prepaid keys with
# small balances, rotate them when the bench is done.
set -euo pipefail
KEYS_FILE="${MONET_KEYS_FILE:-$HOME/.config/monet/bench.env}"
if [[ -f "$KEYS_FILE" ]]; then
  if [[ "$(uname -s)" == "Darwin" ]]; then
    mode="$(/usr/bin/stat -f '%Lp' "$KEYS_FILE")"
  else
    mode="$(stat -c '%a' "$KEYS_FILE")"
  fi
  if [[ "$mode" != "600" && "$mode" != "400" ]]; then
    echo "with-keys: refusing $KEYS_FILE with mode $mode; run: chmod 600 $KEYS_FILE" >&2
    exit 1
  fi
  set -a
  # shellcheck disable=SC1090
  source "$KEYS_FILE"
  set +a
fi
VAULT="${OP_VAULT:-Personal}"
KEYCHAIN_ACCOUNT="${KEYCHAIN_ACCOUNT:-monet}"
read_key() {
  local value=""
  if command -v security >/dev/null 2>&1; then
    value="$(security find-generic-password -a "$KEYCHAIN_ACCOUNT" -s "$1" -w 2>/dev/null || true)"
  fi
  if [[ -z "$value" ]] && command -v op >/dev/null 2>&1; then
    value="$(op read "op://${VAULT}/${1}/password" 2>/dev/null || true)"
  fi
  printf '%s' "$value"
}
DEEPSEEK_API_KEY="${DEEPSEEK_API_KEY:-$(read_key "${OP_ITEM_DEEPSEEK:-api_key_deepseek__monet.harness}")}"
ZAI_API_KEY="${ZAI_API_KEY:-$(read_key "${OP_ITEM_ZAI:-api_key_zai__monet.harness}")}"
OPENAI_API_KEY="${OPENAI_API_KEY:-$(read_key "${OP_ITEM_OPENAI:-api_key_openai__monet.harness}")}"
META_API_KEY="${META_API_KEY:-$(read_key "${OP_ITEM_META:-api_key_meta__monet.harness}")}"
# Not a model provider: TypeSafe's System One endpoint, used only by optional
# failure triage (`--ak triage=true`). Absent is normal, so it is not warned
# about here — but the Butterfly adapter refuses a triage run without it
# rather than silently measuring the control arm.
TYPESAFE_API_KEY="${TYPESAFE_API_KEY:-$(read_key "${OP_ITEM_TYPESAFE:-api_key_typesafeai__butterfly.harness}")}"
for name in DEEPSEEK_API_KEY ZAI_API_KEY OPENAI_API_KEY META_API_KEY; do
  if [[ -z "${!name}" ]]; then echo "with-keys: $name not found in the login Keychain or 1Password (item missing or op not signed in)" >&2; fi
done
export DEEPSEEK_API_KEY ZAI_API_KEY OPENAI_API_KEY META_API_KEY TYPESAFE_API_KEY
# Sandbox-backend credentials ride along when the env file provides them (never printed).
[[ -n "${MODAL_TOKEN_ID:-}" ]] && export MODAL_TOKEN_ID MODAL_TOKEN_SECRET
exec "$@"
