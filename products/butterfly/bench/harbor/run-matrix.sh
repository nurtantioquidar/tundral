#!/usr/bin/env bash
# Terminal-Bench matrix: {models} × {harnesses} on one dataset (or subset).
#
#   MODELS="deepseek/deepseek-v4-flash zai/glm-5.3-flash openai/gpt-5.6-luna" \
#   AGENTS="butterfly mini-swe-agent" \
#   bench/harbor/run-matrix.sh
#
# Environment:
#   TB_DATASET   dataset spec (default terminal-bench/terminal-bench@4.0.0)
#   SUBSET       task-name file, one per line (default dev-subset.txt; SUBSET=full for everything)
#   AGENTS       space-separated: butterfly | mini-swe-agent | any Harbor agent name
#   MODELS       space-separated provider/model ids (Harbor form)
#   BINARY       static Linux butterfly (default dist/butterfly-linux-$(uname -m))
#   K            attempts per trial (default 1)      N_CONCURRENT (default 2)
#   MAX_ATTEMPTS Butterfly attempts per task (default 3)
#   EFFORT       reasoning effort passed to Butterfly (--effort), e.g. xhigh
#   MAX_CALLS    Butterfly provider calls per attempt (--max-calls; CLI default 50)
#   VERIFIER     independent verifier on accepted claims (default off; VERIFIER=true to enable)
#   VERIFIER_MODEL  run the verifier on another model (Harbor form, e.g. zai/glm-5.3-flash)
#   AGENT_TIMEOUT_MULT  Harbor agent wall-clock multiplier (tasks allow 8h; 0.125 = 1h cap)
#   SUBSET=fast-subset.txt  five quick, discriminating tasks for iteration
#   SETUP_TIMEOUT_MULT  Harbor agent-setup timeout multiplier (mini-swe-agent installs from the
#                network inside the container; 3 when many containers share the link)
#   HARBOR_ENV   Harbor environment backend: docker (default) | modal | daytona | e2b | … (-e).
#                Remote backends run x86 containers: pair with BINARY=dist/butterfly-linux-x86_64.
#   JOBS_DIR     where Harbor writes results (default jobs)
#   HARBOR       harbor invocation (default: uvx --from harbor harbor)
#
# Provider keys are read from the host environment (DEEPSEEK_API_KEY, ZAI_API_KEY,
# OPENAI_API_KEY, NIMBLE_API_KEY, and the matching *_BASE_URL); `with-keys.sh` loads them
# from 1Password without ever printing them. mini-swe-agent is the
# "minimal harness" baseline on the same model; it needs the LiteLLM-style key names it
# expects (e.g. DEEPSEEK_API_KEY works as-is).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"

TB_DATASET="${TB_DATASET:-terminal-bench/terminal-bench@4.0.0}"
SUBSET="${SUBSET:-dev-subset.txt}"
AGENTS="${AGENTS:-butterfly mini-swe-agent}"
MODELS="${MODELS:-deepseek/deepseek-v4-flash}"
# Linux arch naming: macOS says arm64, the container build says aarch64.
host_arch="$(uname -m)"; [[ "$host_arch" == "arm64" ]] && host_arch="aarch64"
BINARY="${BINARY:-dist/butterfly-linux-${host_arch}}"
K="${K:-1}"
N_CONCURRENT="${N_CONCURRENT:-2}"
MAX_ATTEMPTS="${MAX_ATTEMPTS:-3}"
JOBS_DIR="${JOBS_DIR:-jobs}"
if [[ -n "${HARBOR_ENV:-}" && "$HARBOR_ENV" != "docker" ]]; then
  # Remote backends need their client SDK next to Harbor: the matching extra (harbor[modal], …).
  HARBOR="${HARBOR:-uvx --from harbor[$HARBOR_ENV] harbor}"
else
  HARBOR="${HARBOR:-uvx --from harbor harbor}"
fi

include_args=()
# GPU-only tasks cannot run in the plain Docker environment; Harbor aborts the whole job
# at validation time if one slips in, so always exclude them (they need Modal or nvidia-docker).
for gpu_task in fp8-rmsnorm-gemm jax-speedrun-gpu math-eval-grader; do
  include_args+=(-x "*${gpu_task}")
done
if [[ "$SUBSET" != "full" ]]; then
  while IFS= read -r line; do
    line="${line%%#*}"; line="$(echo "$line" | xargs)"
    [[ -z "$line" ]] && continue
    include_args+=(-i "*${line}")
  done < "$SUBSET"
fi

# Fail fast when a model's provider key is missing: a Harbor job without it burns
# container time and reports every trial as an exception.
key_for() {
  case "$1" in
    deepseek/*) echo DEEPSEEK_API_KEY ;;
    zai/*|z-ai/*) echo ZAI_API_KEY ;;
    openai/*) echo OPENAI_API_KEY ;;
    meta/*) echo META_API_KEY ;;
    nimble/*) echo NIMBLE_BASE_URL ;;
    *) echo "" ;;
  esac
}
for model in $MODELS; do
  var="$(key_for "$model")"
  if [[ -n "$var" && -z "${!var:-}" ]]; then
    echo "run-matrix: $var is not set for $model (use with-keys.sh; if op hangs, unlock 1Password and approve CLI access)" >&2
    exit 1
  fi
done

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
for model in $MODELS; do
  safe_model="$(echo "$model" | tr '/.' '__')"
  for agent in $AGENTS; do
    job="${agent}__${safe_model}__${stamp}"
    echo "=== $job ($TB_DATASET, subset=$SUBSET)"
    agent_args=()
    if [[ "$agent" == "butterfly" ]]; then
      if [[ ! -f "$BINARY" ]]; then
        echo "missing $BINARY — run bench/harbor/build-linux.sh first" >&2
        exit 1
      fi
      agent_args=(-a butterfly_agent:Butterfly
        --ak "binary=$BINARY" --ak "max_attempts=$MAX_ATTEMPTS")
      if [[ -n "${EFFORT:-}" ]]; then agent_args+=(--ak "effort=$EFFORT"); fi
      if [[ -n "${MAX_CALLS:-}" ]]; then agent_args+=(--ak "max_calls=$MAX_CALLS"); fi
      if [[ -n "${VERIFIER:-}" ]]; then agent_args+=(--ak "independent_verifier=$VERIFIER"); fi
      if [[ -n "${VERIFIER_MODEL:-}" ]]; then
        agent_args+=(--ak "verifier_model=$VERIFIER_MODEL")
        vvar="$(key_for "$VERIFIER_MODEL")"
        if [[ -n "$vvar" && -z "${!vvar:-}" ]]; then
          echo "run-matrix: $vvar is not set for verifier model $VERIFIER_MODEL" >&2; exit 1
        fi
      fi
    else
      agent_args=(-a "$agent")
    fi
    env_args=()
    if [[ -n "${HARBOR_ENV:-}" && "$HARBOR_ENV" != "docker" ]]; then env_args=(-e "$HARBOR_ENV"); fi
    PYTHONPATH="$HERE${PYTHONPATH:+:$PYTHONPATH}" $HARBOR run \
      -d "$TB_DATASET" \
      "${env_args[@]}" \
      "${agent_args[@]}" \
      -m "$model" \
      -k "$K" \
      -n "$N_CONCURRENT" \
      --job-name "$job" \
      --jobs-dir "$JOBS_DIR" \
      --agent-setup-timeout-multiplier "${SETUP_TIMEOUT_MULT:-1}" \
      --agent-timeout-multiplier "${AGENT_TIMEOUT_MULT:-1}" \
      "${include_args[@]}" \
      -y || echo "run-matrix: job $job failed (see above); continuing with the next cell" >&2
  done
done

echo
echo "summarize with: bun summarize.ts $JOBS_DIR"
