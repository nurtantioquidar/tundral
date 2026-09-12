"""Harbor (Terminal-Bench) adapter for Butterfly.

Runs the `butterfly` CLI headlessly inside the task container:

    butterfly --sandboxed --json --no-save --root /app --model <id> \
        --max-attempts N [--require-diff] [--detect-checks] -p <instruction>

The binary is a host-built static Linux executable uploaded at install time
(build it with `bench/harbor/build-linux.sh`, then pass `--ak binary=<path>`).
Nothing is installed from the network inside the container, so setup is fast and
reproducible: the binary hash is the harness version.

Usage (from `products/butterfly/bench/harbor`):

    PYTHONPATH=. harbor run -d terminal-bench/terminal-bench@4.0.0 \
        -a butterfly_agent:Butterfly -m deepseek/deepseek-v4-flash \
        --ak binary=dist/butterfly-linux-aarch64 --ak max_attempts=3 \
        -i bun-sourcemap-leak

`completion_gate` is on by default: the run ends only when the model calls `complete_task`
with evidence per deliverable after reviewing the task statement (text-only replies are nudged,
never accepted — observed 2026-09-04: DeepSeek trials ending on "Now I'll implement…").

`independent_verifier` (default off; `--ak independent_verifier=true`): every accepted claim is
reviewed by a fresh-context verifier session before the loop treats it as finished, and unmet
requirements become the next attempt's evidence. Measured 2026-09-09 on the fast subset: a
same-model verifier on Muse Spark passed 4/4 claims (3 wrong); GLM verifying Muse caught one real
bug (a VLQ corruption) but changed no verdicts, and cost more than the worker ($0.02–0.08 per
review). Off until an A/B on the dev subset shows it pays for itself. `verifier_model`
(Harbor form, e.g. `zai/glm-5.3-flash`) runs the verifier on a different model so it does not
share the worker's blind spots; the container gets that provider's key too.

`lean` (default on; `--ak lean=false`): passes `--lean`, the lean-turns prompt block (no read-back
after a successful edit, scoped orientation, batched reads). Private-suite A/B 2026-09-12, 12 tasks
× 2 runs: GLM 21/24 vs 20/24 at 24 vs 28 calls and $0.060 vs $0.087 per success; DeepSeek 20/24 vs
19/24 at 22 vs 28 calls and $0.028 vs $0.037. Wall clock on GLM is reasoning-latency noise.

`max_calls` raises Butterfly's per-attempt provider-call budget (CLI default 50). Measured
2026-09-08: 54% of GLM attempts and 67% of Muse Spark attempts hit the cap, so most "retries" were
budget cut-offs rather than finished tries; chatty, cheap models need 200-400.

`require_diff` is off by default: Terminal-Bench deliverables can live outside the
workspace (a remote GUI, a database), so a "no files changed" signal produces false retries
(observed on legacy-utility-triage). Enable it per run with `--ak require_diff=true`.

Model names use Harbor's `provider/model` form; the provider prefix selects the
API-key variable forwarded into the container (see PROVIDER_ENV). Keys never
enter logs: Butterfly strips credential-looking variables from every command it
runs and its telemetry carries no secrets.
"""

from __future__ import annotations

import json
import shlex
from pathlib import Path, PurePosixPath
from typing import Any

from harbor.agents.installed.base import BaseInstalledAgent
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

REMOTE_BINARY = PurePosixPath("/installed-agent/butterfly")
# Task name -> coarse Nimble task type (bench/harbor/task-types.json). Forwarded as
# --task-type so a router keeps per-type history; absent entries send nothing.
TASK_TYPES_FILE = Path(__file__).with_name("task-types.json")

# Harbor provider prefix -> (API key variable, optional base-URL variable).
PROVIDER_ENV: dict[str, tuple[str, str | None]] = {
    "deepseek": ("DEEPSEEK_API_KEY", "DEEPSEEK_BASE_URL"),
    "zai": ("ZAI_API_KEY", "ZAI_BASE_URL"),
    "z-ai": ("ZAI_API_KEY", "ZAI_BASE_URL"),
    "openai": ("OPENAI_API_KEY", "OPENAI_BASE_URL"),
    "meta": ("META_API_KEY", "META_BASE_URL"),
    "inver": ("INVER_API_KEY", "INVER_BASE_URL"),
    "nimble": ("NIMBLE_API_KEY", "NIMBLE_BASE_URL"),
}


class Butterfly(BaseInstalledAgent):
    """Butterfly coding harness, driven through its `-p --json` headless mode."""

    SUPPORTS_ATIF = False

    def __init__(
        self,
        *args: Any,
        binary: str | None = None,
        max_attempts: int = 3,
        max_calls: int | None = None,
        require_diff: bool = False,
        detect_checks: bool = False,
        completion_gate: bool = True,
        lean: str | bool = True,
        independent_verifier: bool = False,
        verifier_model: str | None = None,
        max_cost_usd: str = "2.00",
        effort: str | None = None,
        api_key_env: str | None = None,
        base_url: str | None = None,
        extra_args: str = "",
        **kwargs: Any,
    ) -> None:
        super().__init__(*args, **kwargs)
        self._binary = Path(binary).expanduser() if binary else None
        self._max_attempts = int(max_attempts)
        self._max_calls = int(max_calls) if max_calls is not None else None
        self._require_diff = _truthy(require_diff)
        self._detect_checks = _truthy(detect_checks)
        self._completion_gate = _truthy(completion_gate)
        self._lean = _truthy(lean)
        self._independent_verifier = _truthy(independent_verifier)
        self._verifier_model = (
            verifier_model  # Harbor form provider/model, e.g. zai/glm-5.3-flash
        )
        self._max_cost_usd = str(max_cost_usd)
        self._effort = effort
        self._api_key_env = api_key_env
        self._base_url = base_url
        self._extra_args = shlex.split(extra_args) if extra_args else []
        self._version: str | None = None

    @staticmethod
    def name() -> str:
        return "butterfly"

    def version(self) -> str | None:
        return self._version

    # --- install -----------------------------------------------------------

    async def install(self, environment: BaseEnvironment) -> None:
        if self._binary is None or not self._binary.is_file():
            raise RuntimeError(
                "Butterfly needs a prebuilt Linux binary: run bench/harbor/build-linux.sh "
                "and pass --ak binary=<path> (got %r)"
                % (str(self._binary) if self._binary else None)
            )
        await environment.upload_file(self._binary, REMOTE_BINARY.as_posix())
        await self.exec_as_root(
            environment,
            command=f"chmod 755 {shlex.quote(REMOTE_BINARY.as_posix())}",
        )
        result = await self.exec_as_agent(
            environment, command=f"{REMOTE_BINARY.as_posix()} --version"
        )
        if result.return_code != 0:
            raise RuntimeError(
                f"uploaded butterfly binary does not run in the task image: {result.stderr}"
            )
        self._version = (result.stdout or "").strip().replace("butterfly ", "") or None

    # --- run ---------------------------------------------------------------

    def _provider_prefix(self) -> str:
        return (self._parsed_model_provider or "deepseek").lower()

    def _model_id(self) -> str:
        if not self._parsed_model_name:
            raise ValueError("pass -m provider/model, e.g. deepseek/deepseek-v4-flash")
        return self._parsed_model_name

    def _runtime_env(self) -> dict[str, str]:
        key_var, url_var = PROVIDER_ENV.get(self._provider_prefix(), (None, None))
        if self._api_key_env:
            key_var = self._api_key_env
        if key_var is None:
            raise ValueError(
                f"unknown provider prefix {self._provider_prefix()!r}; pass --ak api_key_env=VAR"
            )
        key = self._get_env(key_var)
        if not key:
            raise ValueError(f"{key_var} is not set on the host")
        env = {key_var: key}
        if url_var:
            url = self._base_url or self._get_env(url_var)
            if url:
                env[url_var] = url
        # A cross-model verifier needs its own provider's key in the container.
        if self._verifier_model and "/" in self._verifier_model:
            prefix = self._verifier_model.split("/", 1)[0].lower()
            v_key_var, v_url_var = PROVIDER_ENV.get(prefix, (None, None))
            if v_key_var is None:
                raise ValueError(f"unknown verifier provider prefix {prefix!r}")
            v_key = self._get_env(v_key_var)
            if not v_key:
                raise ValueError(f"{v_key_var} is not set on the host (verifier_model)")
            env[v_key_var] = v_key
            if v_url_var:
                v_url = self._get_env(v_url_var)
                if v_url:
                    env[v_url_var] = v_url
        return env

    def _task_type(self) -> str | None:
        try:
            table = json.loads(TASK_TYPES_FILE.read_text())
        except (OSError, json.JSONDecodeError):
            return None
        # Harbor names the trial directory <task>__<id>; the host logs dir lives inside it.
        try:
            name = Path(self.logs_dir).parent.name.split("__")[0]
        except (AttributeError, TypeError):
            return None
        for key, value in table.items():
            if key != "_comment" and name.endswith(key) and isinstance(value, str):
                return value
        return None

    def _command(self, instruction: str) -> str:
        logs = self.environment_logs_dir
        argv = [
            REMOTE_BINARY.as_posix(),
            "--sandboxed",
            "--json",
            "--no-save",
            "--quiet-reasoning",
            "--root",
            "/app",
            "--model",
            self._model_id(),
            "--max-attempts",
            str(self._max_attempts),
            *(["--max-calls", str(self._max_calls)] if self._max_calls else []),
            "--max-cost-usd",
            self._max_cost_usd,
            "--telemetry",
            (logs / "telemetry.jsonl").as_posix(),
        ]
        if self._require_diff:
            argv.append("--require-diff")
        if self._detect_checks:
            argv.append("--detect-checks")
        if self._lean:
            argv.append("--lean")
        if self._completion_gate:
            argv.append("--completion-gate")
            if self._independent_verifier:
                argv.append("--independent-verifier")
                if self._verifier_model:
                    argv += ["--verifier-model", self._verifier_model.split("/", 1)[-1]]
        if self._effort:
            argv += ["--effort", self._effort]
        task_type = self._task_type()
        if task_type:
            argv += ["--task-type", task_type]
        argv += self._extra_args
        argv += ["-p", instruction]
        quoted = " ".join(shlex.quote(part) for part in argv)
        # HOME must be writable for the best-effort state file; /app is the
        # task's working directory by Terminal-Bench convention.
        return (
            f"mkdir -p {shlex.quote(logs.as_posix())} && cd /app && "
            f"HOME=${{HOME:-/tmp}} {quoted} "
            f"> {shlex.quote((logs / 'butterfly.json').as_posix())} "
            f"2> {shlex.quote((logs / 'butterfly.stderr').as_posix())}"
        )

    async def run(
        self, instruction: str, environment: BaseEnvironment, context: AgentContext
    ) -> None:
        env = self._runtime_env()
        await self.exec_as_agent(
            environment, command=self._command(instruction), env=env
        )

    # --- post-run ----------------------------------------------------------

    def populate_context_post_run(self, context: AgentContext) -> None:
        report_path = self.logs_dir / "butterfly.json"
        if not report_path.is_file():
            return
        report = _last_json_object(report_path.read_text(errors="replace"))
        if report is None:
            return
        context.n_input_tokens = _as_int(report.get("input_tokens"))
        context.n_cache_tokens = _as_int(report.get("cache_hit_input_tokens"))
        context.n_output_tokens = _as_int(report.get("output_tokens"))
        micro = _as_int(report.get("actual_cost_microusd"))
        if micro is not None and report.get("price_verified", False):
            context.cost_usd = micro / 1_000_000
        metadata = {k: v for k, v in report.items() if k != "response"}
        metadata["response_chars"] = len(report.get("response") or "")
        context.metadata = metadata


def _truthy(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    return str(value).strip().lower() in {"1", "true", "yes", "on"}


def _as_int(value: Any) -> int | None:
    try:
        return int(value) if value is not None else None
    except (TypeError, ValueError):
        return None


def _last_json_object(text: str) -> dict[str, Any] | None:
    stripped = text.strip()
    try:
        parsed = json.loads(stripped)
        return parsed if isinstance(parsed, dict) else None
    except json.JSONDecodeError:
        pass
    for line in reversed(stripped.splitlines()):
        try:
            parsed = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(parsed, dict):
            return parsed
    return None
