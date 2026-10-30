#!/usr/bin/env python3
"""Stage 7 agent-outcome benchmark: the same Codex model answers each task in three arms.

    uv run --no-project bench/agent/run.py prepare --tasks bench/agent/questions/fastapi.jsonl
    uv run --no-project bench/agent/run.py run     --tasks ... [--arms none,graphify,dijkstral] [--jobs 2]
    uv run --no-project bench/agent/run.py judge   --tasks ...
    uv run --no-project bench/agent/run.py report  --tasks ...
    (every command takes --limit N, --ids a,b and --dry-run; run/judge take --fake-provider)

Arms (identical model, prompt, sandbox and budget; each tool gets what its own installer gives):
  none       a plain copy of the corpus
  graphify   copy + Graphify's graph (`extract --code-only`, or the semantic pass through
             bench/llm_shim.py) + `graphify codex install` + graphify-mcp attached for the call
  dijkstral  copy + `dijkstral extract --code-only` (semantic: its Codex backend) +
             `dijkstral codex install` + `dijkstral mcp` attached for the call

`prepare` builds one workspace per (corpus_id, arm) under codebench/ws next to the bench home and
records build and install time, model calls and tokens in agent/manifests/. `run` executes one
`codex exec --json` session per (task, arm), read-only, and stores the raw events and a result
per session under agent/sessions/<arm>/ (raw events can contain repository text: they never go
into the source tree). A session whose result exists is skipped, so an interrupted run resumes.
Transient failures are retried a bounded number of times; a Codex usage-limit error stops the
launch of new sessions, lets the running ones finish and exits with status 75. `judge` asks a
blind judge (same model, empty directory, no MCP, JSON schema) to grade every answer, shuffled
and without the arm name; verdicts are cached by content. `report` writes a markdown table and a
JSON summary to bench/results/agent-<epoch>.{md,json}.

Never modifies ~/.codex: everything is per-call flags. Never reads API keys. Stdlib only.
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import json
import os
import random
import shlex
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from functools import cache
from pathlib import Path

HERE = Path(__file__).resolve().parent
BENCH = HERE.parent
CLI = BENCH.parent / "cli"
sys.path.insert(0, str(HERE))

import events
import memory
import score

ARMS = ("none", "graphify", "dijkstral")
MODEL = "gpt-6.1-sol"
EFFORT = "medium"
GRAPHIFY_PIN = "0.9.86"
# The Graphify the graphify arm runs; `GRAPHIFY_VERSION=0.9.97` compares another release (installed
# by `bench/setup.sh` under the same variable). See `Ctx.slot`.
GRAPHIFY_VERSION = os.environ.get("GRAPHIFY_VERSION", GRAPHIFY_PIN)
QUOTA_EXIT = 75  # EX_TEMPFAIL: rerun the same command later to resume
PROMPT_VERSION = "agent-prompt-v1"
OUT_DIRS = ("graphify-out", "dijkstral-out")  # never copied from a corpus checkout

# Flags every Codex call gets (sessions and judge). The ChatGPT-account apps (GitHub, Slack, ...)
# and web search would let an answer come from outside the corpus, so both are off for all arms.
COMMON_FLAGS = ["--disable", "apps", "-c", 'web_search="disabled"', "--color", "never"]

INSTRUCTION = {
    "keyfacts": "Answer from this repository. Cite file paths and symbol names where relevant. Be concise.",
    # v2: models such as gpt-6-luna often claimed they could not read the notes and asked for
    # them to be pasted (all arms, 4-33 of 60-120 sessions); naming the shell tools fixes that
    # without mentioning either graph tool.
    "notes": (
        "Answer from these notes: the Markdown files in the current directory, which you can list, "
        "search and read with shell commands such as ls, rg and sed. Cite file paths where relevant. "
        "Be concise."
    ),
}

JUDGE_VERSION = "judge-v1"
JUDGE_PROMPT = """You are grading one answer to a question about a code repository or a set of notes.
You are given the question, a reference answer written by the benchmark authors{facts_clause}, and one
candidate answer. You cannot see the repository or the notes: do not run commands or read files.
Grade only whether the candidate answers the question correctly and agrees with the reference.
Extra correct detail is fine. A contradiction of the reference, a wrong file, symbol or value, or
missing the core of the reference makes it incorrect. Paths and symbol names that differ only in
formatting count as the same.

Return JSON with:
- correct: true if the candidate's answer is right in substance
- score: 1 for fully correct and complete, 0.5 for partly correct or missing key parts, 0 for wrong or no answer (values in between are allowed)
- reason: one sentence

Question: {question}
{date_line}Reference answer: {reference}
{facts_block}Candidate answer:
<<<
{answer}
>>>
"""
JUDGE_SCHEMA = {
    "type": "object",
    "properties": {
        "correct": {"type": "boolean"},
        "score": {"type": "number"},
        "reason": {"type": "string"},
    },
    "required": ["correct", "score", "reason"],
    "additionalProperties": False,
}


# ---------------------------------------------------------------------------------------------
# Tasks


def load_tasks(path: Path) -> list[dict]:
    tasks, seen = [], set()
    for n, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip():
            continue
        t = json.loads(line)
        for key in ("id", "suite", "corpus_id", "corpus", "question", "reference"):
            if t.get(key) in (None, ""):
                sys.exit(f"{path}:{n}: task is missing {key!r}")
        if t["id"] in seen:
            sys.exit(f"{path}:{n}: duplicate task id {t['id']!r}")
        seen.add(t["id"])
        t["corpus"] = str(Path(os.path.expanduser(t["corpus"])))
        t.setdefault("semantic", False)
        t.setdefault("question_date", None)
        t.setdefault("kind", "other")
        t.setdefault("facts", [])
        tasks.append(t)
    corpora: dict[str, tuple] = {}
    for t in tasks:
        sig = (t["corpus"], bool(t["semantic"]))
        if corpora.setdefault(t["corpus_id"], sig) != sig:
            sys.exit(
                f"corpus_id {t['corpus_id']!r} is used with different corpus/semantic values"
            )
    return tasks


def select(tasks: list[dict], ids: str, limit: int) -> list[dict]:
    if ids:
        want = [i for i in ids.split(",") if i]
        missing = set(want) - {t["id"] for t in tasks}
        if missing:
            sys.exit(f"unknown task ids: {', '.join(sorted(missing))}")
        tasks = [t for t in tasks if t["id"] in set(want)]
    return tasks[:limit] if limit else tasks


def build_prompt(task: dict) -> str:
    lines = [f"Question: {task['question'].strip()}"]
    if task.get("question_date"):
        lines.append(f"Question date: {task['question_date']}")
    lines.append("")
    lines.append(INSTRUCTION["keyfacts" if task["suite"] == "keyfacts" else "notes"])
    return "\n".join(lines)


# ---------------------------------------------------------------------------------------------
# Context: paths, tools and the per-call Codex configuration


def sha(*parts: object, n: int = 16) -> str:
    h = hashlib.sha256()
    for p in parts:
        h.update(json.dumps(p, sort_keys=True, default=str).encode())
        h.update(b"\0")
    return h.hexdigest()[:n]


@dataclass
class Ctx:
    home: Path
    codex: str
    model: str
    effort: str
    timeout: float
    retries: int
    backoff: float
    jobs: int
    dry_run: bool
    graphify_venv: Path
    graphify_semantic_venv: Path
    dijkstral: Path
    sessions_dir: Path
    judge_dir: Path
    fake: bool = False  # --fake-provider: local fake model, separate result directories
    # The judge and the semantic graph builds keep their own model, whatever model answers.
    judge_model: str = MODEL
    judge_effort: str = EFFORT
    build_model: str = MODEL
    build_effort: str = EFFORT
    fake_provider_flags: list[str] = field(default_factory=list)
    runner: Callable | None = None
    stop: threading.Event = field(default_factory=threading.Event)
    print_lock: threading.Lock = field(default_factory=threading.Lock)

    @property
    def effective_model(self) -> str:
        return "fake-model" if self.fake else self.model

    @property
    def agent(self) -> Path:
        return self.home / "agent"

    def slot(self, arm: str) -> str:
        """The arm's name in stored paths (workspace, manifest, homes, build state, sessions, tool
        dir): a graphify arm on another Graphify than the pin is `graphify-<version>`, so its graph,
        executables and answers never mix with the pin's; the pin keeps `graphify`, so earlier
        results stay valid."""
        if arm == "graphify" and GRAPHIFY_VERSION != GRAPHIFY_PIN:
            return f"graphify-{GRAPHIFY_VERSION}"
        return arm

    def pair_dir(self, corpus_id: str, arm: str) -> Path:
        # Opaque directory name under a neutral root: the session's working directory names
        # neither the arm nor either tool (the bench home is called dijkstral-bench).
        return ws_root() / sha("ws", corpus_id, self.slot(arm), n=10)

    def workspace(self, corpus_id: str, arm: str) -> Path:
        return self.pair_dir(corpus_id, arm) / corpus_id

    def manifest(self, corpus_id: str, arm: str) -> Path:
        return self.agent / "manifests" / f"{corpus_id}.{self.slot(arm)}.json"

    def graphify_bin(self, semantic: bool) -> Path:
        return (self.graphify_semantic_venv if semantic else self.graphify_venv) / "bin"

    def tool_dir(self, arm: str, semantic: bool) -> Path | None:
        """A directory holding only the arm's own executables, put first on the session PATH."""
        if arm == "none":
            return None
        name = self.slot(arm)
        return (
            self.agent
            / "bin"
            / (f"{name}-semantic" if arm == "graphify" and semantic else name)
        )

    def log(self, msg: str) -> None:
        with self.print_lock:
            print(msg, flush=True)


def mcp_flags(ctx: Ctx, arm: str, ws: Path, semantic: bool) -> list[str]:
    """Attach the arm's MCP server for this call only. Tool calls are pre-approved
    (`codex exec` cannot ask), with the same startup and tool timeouts for both servers."""
    if arm == "none":
        return []
    if arm == "graphify":
        name = "graphify"
        command = str(ctx.graphify_bin(semantic) / "graphify-mcp")
        args = [str(ws / "graphify-out" / "graph.json")]
        env = {"GRAPHIFY_QUERY_LOG_DISABLE": "1"}
    else:
        name = "dijkstral"
        command = str(ctx.dijkstral)
        args = ["mcp", str(ws / "dijkstral-out" / "graph.dij")]
        env = {}
    flags = [
        "-c", f"mcp_servers.{name}.command={json.dumps(command)}",
        "-c", f"mcp_servers.{name}.args={json.dumps(args)}",
        "-c", f"mcp_servers.{name}.startup_timeout_sec=120",
        "-c", f"mcp_servers.{name}.tool_timeout_sec=120",
        "-c", f'mcp_servers.{name}.default_tools_approval_mode="approve"',
    ]  # fmt: skip
    if env:
        inner = ", ".join(f"{k} = {json.dumps(v)}" for k, v in env.items())
        flags += ["-c", f"mcp_servers.{name}.env={{{inner}}}"]
    return flags


def codex_base(ctx: Ctx, cwd: Path, judge: bool = False) -> list[str]:
    model = "fake-model" if ctx.fake else (ctx.judge_model if judge else ctx.model)
    effort = ctx.judge_effort if judge else ctx.effort
    return [
        ctx.codex, "exec", "--ephemeral", "--ignore-user-config",
        "-m", model, "-c", f'model_reasoning_effort="{effort}"',
        "-s", "read-only", "-C", str(cwd), "--json", "--skip-git-repo-check",
        *COMMON_FLAGS, *ctx.fake_provider_flags,
    ]  # fmt: skip


def session_command(ctx: Ctx, task: dict, arm: str, last_message: Path) -> list[str]:
    ws = ctx.workspace(task["corpus_id"], arm)
    return [
        *codex_base(ctx, ws),
        *mcp_flags(ctx, arm, ws, bool(task["semantic"])),
        "-o", str(last_message),
        build_prompt(task),
    ]  # fmt: skip


_DROP_ENV_PREFIXES = ("HERDR_", "GRAPHIFY_", "DIJKSTRAL_")


def _drop_env(key: str) -> bool:
    # API keys are never passed on (Codex uses the ChatGPT login); tool and terminal-multiplexer
    # variables from the caller's shell would differ between machines, not between arms.
    return (
        key.startswith(_DROP_ENV_PREFIXES)
        or key.endswith("_API_KEY")
        or key == "OPENAI_BASE_URL"
    )


def base_path(path: str | None = None) -> tuple[list[str], list[str]]:
    """PATH without any directory that already provides `graphify` or `dijkstral`."""
    keep, dropped = [], []
    for d in (path if path is not None else os.environ.get("PATH", "")).split(
        os.pathsep
    ):
        if not d:
            continue
        if any(
            os.access(os.path.join(d, b), os.X_OK)
            for b in ("graphify", "graphify-mcp", "dijkstral")
        ):
            dropped.append(d)
        else:
            keep.append(d)
    return keep, dropped


def session_env(ctx: Ctx, arm: str, semantic: bool) -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if not _drop_env(k)}
    keep, _ = base_path()
    tool_dir = ctx.tool_dir(arm, semantic)
    env["PATH"] = os.pathsep.join(([str(tool_dir)] if tool_dir else []) + keep)
    env["GRAPHIFY_QUERY_LOG_DISABLE"] = "1"
    return env


# ---------------------------------------------------------------------------------------------
# Processes


def run_proc(
    cmd: list[str],
    env: dict[str, str],
    cwd: Path,
    timeout: float,
    stdout: Path,
    stderr: Path,
) -> tuple[int | None, bool]:
    """Run to completion or timeout (then the whole process group is killed)."""
    with open(stdout, "wb") as out, open(stderr, "wb") as err:
        p = subprocess.Popen(
            cmd,
            env=env,
            cwd=cwd,
            stdin=subprocess.DEVNULL,
            stdout=out,
            stderr=err,
            start_new_session=True,
        )
        try:
            return p.wait(timeout=timeout), False
        except subprocess.TimeoutExpired:
            for sig, wait in ((signal.SIGTERM, 10), (signal.SIGKILL, 10)):
                try:
                    os.killpg(p.pid, sig)
                except ProcessLookupError:
                    break
                try:
                    p.wait(timeout=wait)
                    break
                except subprocess.TimeoutExpired:
                    continue
            return None, True


def write_json(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(data, indent=1, ensure_ascii=False))
    os.replace(tmp, path)


def read_json(path: Path) -> dict | None:
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return None


def rm_tree(path: Path) -> None:
    """`rm -rf` restricted to the bench home's agent directory (or the temp dir, for tests).
    Corpus checkouts are outside it, so they can never be deleted from here."""
    p = path.resolve()
    agent = (bench_home() / "agent").resolve()
    roots = [agent, ws_root().resolve(), Path(tempfile.gettempdir()).resolve()]
    if not any(str(p).startswith(str(r) + os.sep) for r in roots):
        raise SystemExit(f"refusing to delete {p}: outside {agent}")
    shutil.rmtree(p, ignore_errors=True)


def ws_root() -> Path:
    """Where arm workspaces live: a neutral path, so no arm sees a tool's name in its cwd."""
    explicit = os.environ.get("AGENT_BENCH_WS")
    return Path(explicit) if explicit else bench_home().parent / "codebench" / "ws"


def bench_home() -> Path:
    return Path(
        os.environ.get("DIJKSTRAL_BENCH_HOME") or Path.home() / ".cache/dijkstral-bench"
    )


# ---------------------------------------------------------------------------------------------
# Sessions


def session_key(ctx: Ctx, task: dict) -> str:
    """What makes a stored result reusable: model, effort and the exact prompt."""
    model = ctx.effective_model
    return sha(PROMPT_VERSION, model, ctx.effort, build_prompt(task))


def session_paths(ctx: Ctx, arm: str, task_id: str) -> dict[str, Path]:
    d = ctx.sessions_dir / ctx.slot(arm)
    safe = task_id.replace("/", "_")
    return {
        "dir": d,
        "result": d / f"{safe}.json",
        "failed": d / f"{safe}.failed.json",
        "events": d / f"{safe}.events.jsonl",
        "stderr": d / f"{safe}.stderr.txt",
        "last": d / f"{safe}.last.txt",
    }


def graded_answer(result: dict) -> str:
    """The answer as scored and judged: the session's workspace prefix is removed from absolute
    paths (Codex often cites `/abs/workspace/pkg/x.go`), so paths read repo-relative for the
    key-fact match and the judge never sees the workspace location."""
    answer = result.get("answer") or ""
    ws = result.get("workspace")
    if ws:
        for prefix in {ws, os.path.realpath(ws)}:
            answer = answer.replace(prefix.rstrip("/") + "/", "")
    return answer


def session_done(ctx: Ctx, task: dict, arm: str) -> bool:
    r = read_json(session_paths(ctx, arm, task["id"])["result"])
    return (
        bool(r) and r.get("status") == "ok" and r.get("key") == session_key(ctx, task)
    )


def execute_session(ctx: Ctx, task: dict, arm: str) -> str:
    """Run one (task, arm) session with bounded retries. Returns ok|skipped|failed|quota|stopped."""
    if session_done(ctx, task, arm):
        return "skipped"
    paths = session_paths(ctx, arm, task["id"])
    paths["dir"].mkdir(parents=True, exist_ok=True)
    ws = ctx.workspace(task["corpus_id"], arm)
    cmd = session_command(ctx, task, arm, paths["last"])
    env = session_env(ctx, arm, bool(task["semantic"]))
    runner = ctx.runner or run_proc
    attempts = []
    for attempt in range(ctx.retries + 1):
        if attempt and ctx.stop.is_set():
            return "stopped"
        ev_path = paths["dir"] / f"{paths['result'].stem}.a{attempt}.events.jsonl"
        err_path = paths["dir"] / f"{paths['result'].stem}.a{attempt}.stderr.txt"
        paths["last"].unlink(missing_ok=True)
        t0 = time.monotonic()
        rc, timed_out = runner(cmd, env, ws, ctx.timeout, ev_path, err_path)
        wall = time.monotonic() - t0
        with open(ev_path, errors="replace") as fh:
            parsed = events.parse_events(fh)
        stderr = err_path.read_text(errors="replace") if err_path.exists() else ""
        if not parsed["answer"] and paths["last"].exists():
            parsed["answer"] = paths["last"].read_text(errors="replace").strip()
        status = events.classify(parsed, rc, stderr, timed_out)
        attempts.append(
            {
                "attempt": attempt,
                "returncode": rc,
                "timed_out": timed_out,
                "wall_s": round(wall, 2),
                "status": status,
                "fatal": parsed["fatal"][:3],
                "stderr_errors": [
                    ln[-300:] for ln in stderr.splitlines() if "ERROR" in ln
                ][-3:],
            }
        )
        if status == "ok":
            os.replace(ev_path, paths["events"])
            os.replace(err_path, paths["stderr"])
            result = {
                "status": "ok",
                "key": session_key(ctx, task),
                "task_id": task["id"],
                "arm": arm,
                "suite": task["suite"],
                "kind": task["kind"],
                "corpus_id": task["corpus_id"],
                "answer": parsed["answer"],
                "usage": parsed["usage"],
                "tool_calls": parsed["tool_calls"],
                "tool_calls_by_type": parsed["tool_calls_by_type"],
                "commands": parsed["commands"],
                "commands_by_program": parsed["commands_by_program"],
                "graph_cli_commands": parsed["graph_cli_commands"],
                "mcp_calls": parsed["mcp_calls"],
                "mcp_calls_by_tool": parsed["mcp_calls_by_tool"],
                "mcp_failed": parsed["mcp_failed"],
                "warnings": parsed["errors"],
                "thread_id": parsed["thread_id"],
                "wall_s": round(wall, 2),
                "attempts": attempts,
                "workspace": str(ws),
                "command": cmd[:-1] + ["<prompt>"],
                "finished_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
            }
            write_json(paths["result"], result)
            paths["failed"].unlink(missing_ok=True)
            u = parsed["usage"] or {}
            ctx.log(
                f"ok       {arm:9} {task['id']:24} {wall:6.0f}s tokens {u.get('input_tokens', 0)}"
                f"/{u.get('output_tokens', 0)} tools {parsed['tool_calls']} mcp {parsed['mcp_calls']}"
            )
            return "ok"
        if status == "quota":
            ctx.stop.set()
            ctx.log(
                f"QUOTA    {arm:9} {task['id']:24} usage limit reached: {(parsed['fatal'] or [''])[0][:160]}"
            )
            return "quota"
        ctx.log(
            f"retry    {arm:9} {task['id']:24} attempt {attempt + 1}/{ctx.retries + 1} "
            f"rc={rc} timeout={timed_out} {(parsed['fatal'] or [''])[0][:120]}"
        )
        if attempt < ctx.retries and not ctx.stop.wait(ctx.backoff * (attempt + 1)):
            continue
    write_json(
        paths["failed"],
        {
            "status": "failed",
            "key": session_key(ctx, task),
            "task_id": task["id"],
            "arm": arm,
            "attempts": attempts,
        },
    )
    ctx.log(f"FAILED   {arm:9} {task['id']}")
    return "failed"


def run_queue(
    items: list, execute: Callable, jobs: int, stop: threading.Event
) -> dict[str, int]:
    """Run `execute(item)` with at most `jobs` in flight; once `stop` is set, launch nothing new
    and let the running ones finish."""
    counts: dict[str, int] = {}
    it = iter(items)
    with cf.ThreadPoolExecutor(max_workers=max(1, jobs)) as ex:
        inflight: set[cf.Future] = set()
        exhausted = False
        while True:
            while not stop.is_set() and not exhausted and len(inflight) < max(1, jobs):
                try:
                    item = next(it)
                except StopIteration:
                    exhausted = True
                    break
                inflight.add(ex.submit(execute, item))
            if not inflight:
                break
            done, inflight = cf.wait(inflight, return_when=cf.FIRST_COMPLETED)
            for f in done:
                status = f.result()
                counts[status] = counts.get(status, 0) + 1
    if stop.is_set():
        counts["not_started"] = sum(1 for _ in it)
    return counts


def quota_exit(ctx: Ctx, counts: dict) -> None:
    if ctx.stop.is_set():
        argv = shlex.join(sys.argv)
        print(
            f"\nQUOTA EXHAUSTED — resume with the same command: uv run --no-project {argv}\n"
            f"(finished {counts}; sessions without a result are retried on the next run)",
            file=sys.stderr,
            flush=True,
        )
        sys.exit(QUOTA_EXIT)


# ---------------------------------------------------------------------------------------------
# Judge


def judge_prompt(task: dict, answer: str) -> str:
    facts = task.get("facts") or []
    facts_block = ""
    if facts:
        lines = []
        for f in facts:
            aliases = f.get("aliases") or []
            lines.append(
                f"- {f.get('fact')}"
                + (f" (also written: {', '.join(aliases)})" if aliases else "")
            )
        facts_block = (
            "Key facts a complete answer mentions:\n" + "\n".join(lines) + "\n"
        )
    return JUDGE_PROMPT.format(
        facts_clause=", key facts a complete answer mentions" if facts else "",
        question=task["question"].strip(),
        date_line=f"Question date: {task['question_date']}\n"
        if task.get("question_date")
        else "",
        reference=str(task["reference"]).strip(),
        facts_block=facts_block,
        answer=answer.strip() or "(no answer)",
    )


def judge_spec(task: dict, answer: str) -> tuple[str, bool]:
    """(prompt, structured). LongMemEval is graded with its official yes/no answer-check prompt
    (`memory.longmemeval_judge_prompt`, abstention from `adversarial`); every other suite with
    JUDGE_PROMPT and the JSON schema. Neither shows `source_id` or `distractor`."""
    if task["suite"] == "longmemeval":
        prompt = memory.longmemeval_judge_prompt(
            task["kind"],
            task["question"].strip(),
            str(task["reference"]).strip(),
            answer.strip() or "(no answer)",
            bool(task.get("adversarial")),
        )
        return prompt, False
    return judge_prompt(task, answer), True


def judge_key(ctx: Ctx, task: dict, answer: str) -> str:
    model = "fake-model" if ctx.fake else ctx.judge_model
    return sha(JUDGE_VERSION, model, ctx.judge_effort, *judge_spec(task, answer), n=24)


def judge_command(
    ctx: Ctx, scratch: Path, schema: Path | None, last: Path, prompt: str
) -> list[str]:
    schema_flags = ["--output-schema", str(schema)] if schema else []
    return [
        *codex_base(ctx, scratch, judge=True),
        *schema_flags,
        "-o",
        str(last),
        prompt,
    ]


def parse_verdict(text: str) -> dict | None:
    try:
        v = json.loads(text.strip())
    except ValueError:
        return None
    if not isinstance(v, dict) or not isinstance(v.get("correct"), bool):
        return None
    try:
        s = float(v.get("score"))
    except (TypeError, ValueError):
        return None
    return {
        "correct": v["correct"],
        "score": min(1.0, max(0.0, s)),
        "reason": str(v.get("reason", ""))[:500],
    }


def parse_yes_no(text: str) -> dict | None:
    """LongMemEval's rule: the judge's reply counts as correct when it contains "yes"."""
    if not text.strip():
        return None
    correct = "yes" in text.lower()
    return {"correct": correct, "score": float(correct), "reason": text.strip()[:200]}


def execute_judge(ctx: Ctx, task: dict, answer: str) -> str:
    key = judge_key(ctx, task, answer)
    out = ctx.judge_dir / f"{key}.json"
    r = read_json(out)
    if r and r.get("status") == "ok":
        return "skipped"
    ctx.judge_dir.mkdir(parents=True, exist_ok=True)
    tmp_root = ctx.agent / "tmp"
    tmp_root.mkdir(parents=True, exist_ok=True)
    attempts = []
    runner = ctx.runner or run_proc
    env = session_env(ctx, "none", False)
    for attempt in range(ctx.retries + 1):
        if attempt and ctx.stop.is_set():
            return "stopped"
        scratch_parent = Path(tempfile.mkdtemp(prefix="judge-", dir=tmp_root))
        try:
            scratch = scratch_parent / "empty"
            scratch.mkdir()
            prompt, structured = judge_spec(task, answer)
            schema = scratch_parent / "schema.json" if structured else None
            if schema:
                schema.write_text(json.dumps(JUDGE_SCHEMA))
            last = scratch_parent / "last.txt"
            ev_path = ctx.judge_dir / f"{key}.a{attempt}.events.jsonl"
            err_path = ctx.judge_dir / f"{key}.a{attempt}.stderr.txt"
            cmd = judge_command(ctx, scratch, schema, last, prompt)
            t0 = time.monotonic()
            rc, timed_out = runner(cmd, env, scratch, ctx.timeout, ev_path, err_path)
            wall = time.monotonic() - t0
            with open(ev_path, errors="replace") as fh:
                parsed = events.parse_events(fh)
            if not parsed["answer"] and last.exists():
                parsed["answer"] = last.read_text(errors="replace")
        finally:
            rm_tree(scratch_parent)
        stderr = err_path.read_text(errors="replace") if err_path.exists() else ""
        status = events.classify(parsed, rc, stderr, timed_out)
        parse = parse_verdict if structured else parse_yes_no
        verdict = parse(parsed["answer"]) if status == "ok" else None
        if status == "ok" and verdict is None:
            status = "transient"
        attempts.append(
            {
                "attempt": attempt,
                "returncode": rc,
                "timed_out": timed_out,
                "status": status,
            }
        )
        if status == "ok" and verdict is not None:
            os.replace(ev_path, ctx.judge_dir / f"{key}.events.jsonl")
            err_path.unlink(missing_ok=True)
            write_json(
                out,
                {
                    "status": "ok",
                    "key": key,
                    "task_id": task["id"],
                    "verdict": verdict,
                    "usage": parsed["usage"],
                    "tool_calls": parsed["tool_calls"],
                    "wall_s": round(wall, 2),
                    "attempts": attempts,
                },
            )
            ctx.log(
                f"judged   {task['id']:24} correct={verdict['correct']} score={verdict['score']}"
            )
            return "ok"
        if status == "quota":
            ctx.stop.set()
            ctx.log(f"QUOTA    judge     {task['id']:24} usage limit reached")
            return "quota"
        if attempt < ctx.retries and not ctx.stop.wait(ctx.backoff * (attempt + 1)):
            continue
    ctx.log(f"FAILED   judge {task['id']}")
    return "failed"


# ---------------------------------------------------------------------------------------------
# Prepare: one workspace per (corpus_id, arm)


def copy_command(src: Path, dst: Path) -> list[str]:
    if sys.platform == "darwin":
        return [
            "/bin/cp",
            "-cR",
            str(src),
            str(dst),
        ]  # APFS clone (BSD cp): no data copied
    return ["cp", "-a", str(src), str(dst)]


def scrubbed_env(home: Path, extra: dict[str, str] | None = None) -> dict[str, str]:
    env = {
        "HOME": str(home),
        "PATH": "/usr/bin:/bin",
        "LANG": "en_US.UTF-8",
        "GRAPHIFY_QUERY_LOG_DISABLE": "1",
        "TMPDIR": os.environ.get("TMPDIR", "/tmp"),
    }
    env.update(extra or {})
    return env


def tree_files(root: Path) -> list[str]:
    return (
        sorted(str(p.relative_to(root)) for p in root.rglob("*"))
        if root.exists()
        else []
    )


def snapshot(ws: Path) -> dict[str, str]:
    """sha256 of the workspace's top-level files and everything under .codex/ (install footprint)."""
    out = {}
    paths = [p for p in ws.iterdir() if p.is_file()] if ws.exists() else []
    if (ws / ".codex").is_dir():
        paths += [p for p in (ws / ".codex").rglob("*") if p.is_file()]
    for p in paths:
        out[str(p.relative_to(ws))] = hashlib.sha256(p.read_bytes()).hexdigest()[:16]
    return out


def hook_commands(ws: Path) -> list[str]:
    data = read_json(ws / ".codex" / "hooks.json") or {}
    cmds = []
    for event, entries in (data.get("hooks") or {}).items():
        for e in entries or []:
            for h in e.get("hooks") or []:
                cmds.append(f"{event}[{e.get('matcher', '')}]: {h.get('command')}")
    return cmds


def graph_counts(path: Path) -> dict:
    data = read_json(path)
    if not data:
        return {}
    return {
        "nodes": len(data.get("nodes", [])),
        "edges": len(data.get("links", data.get("edges", []))),
    }


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def fmt_cmd(
    cmd: list[str], env: dict[str, str] | None = None, cwd: Path | None = None
) -> str:
    s = shlex.join(cmd)
    if env:
        s = (
            "env -i "
            + " ".join(f"{k}={shlex.quote(v)}" for k, v in env.items())
            + " "
            + s
        )
    return f"(cd {shlex.quote(str(cwd))} && {s})" if cwd else s


def codex_home() -> str:
    return os.environ.get("CODEX_HOME") or str(Path.home() / ".codex")


def write_codex_counter(ctx: Ctx) -> Path:
    """A `codex` wrapper for dijkstral's semantic pass: counts `codex exec` runs and keeps their
    JSON events (to detect a usage limit). Other subcommands (`login status`) pass through."""
    path = ctx.agent / "bin" / "codex-count.sh"
    path.parent.mkdir(parents=True, exist_ok=True)
    codex = shlex.quote(ctx.codex)
    path.write_text(
        "#!/bin/bash\n"
        'if [ "$1" = exec ] && [ -n "$DIJKSTRAL_BENCH_CALL_LOG" ]; then\n'
        '  echo exec >> "$DIJKSTRAL_BENCH_CALL_LOG"\n'
        f'  {codex} "$@" | tee -a "$DIJKSTRAL_BENCH_CALL_LOG.events.jsonl"\n'
        '  exit "${PIPESTATUS[0]}"\n'
        "fi\n"
        f'exec {codex} "$@"\n'
    )
    path.chmod(0o755)
    return path


def build_quota_hit(
    texts: list[str], transcripts: Path | None, events_log: Path | None
) -> bool:
    """Whether a semantic build ran into a Codex usage limit: the tool's own output, the shim's
    transcripts (Graphify) or the Codex events of dijkstral's backend."""
    found = list(texts)
    if transcripts and transcripts.is_dir():
        for f in transcripts.rglob("req-*.json"):
            resp = (read_json(f) or {}).get("response") or {}
            found += [str(resp.get("note") or ""), str(resp.get("stderr_tail") or "")]
            found += [str(e) for e in resp.get("errors") or []]
    if events_log and events_log.exists():
        with open(events_log, errors="replace") as fh:
            parsed = events.parse_events(fh)
        found += parsed["fatal"] + parsed["errors"]
    return any(events.QUOTA_PATTERNS.search(t) for t in found if t)


def build_command(ctx: Ctx, arm: str, ws: Path, semantic: bool) -> list[str]:
    if arm == "graphify":
        g = str(ctx.graphify_bin(semantic) / "graphify")
        return (
            [g, "extract", str(ws), "--backend", "openai"]
            if semantic
            else [g, "extract", str(ws), "--code-only"]
        )
    if semantic:
        return [str(ctx.dijkstral), "extract", str(ws), "--backend", "codex",
                "--model", ctx.build_model, "--reasoning-effort", ctx.build_effort]  # fmt: skip
    return [str(ctx.dijkstral), "extract", str(ws), "--code-only"]


def install_command(ctx: Ctx, arm: str, semantic: bool) -> list[str]:
    tool = (
        str(ctx.graphify_bin(semantic) / "graphify")
        if arm == "graphify"
        else str(ctx.dijkstral)
    )
    return [tool, "codex", "install"]


def shim_command(
    ctx: Ctx, port: str, transcripts: Path, max_requests: int
) -> list[str]:
    return [
        sys.executable, str(BENCH / "llm_shim.py"), "--port", port, "--transcript-dir", str(transcripts),
        "--max-requests", str(max_requests), "--parallel", "2", "--responder", "codex", "--codex", ctx.codex,
        "--model", ctx.build_model, "--reasoning-effort", ctx.build_effort,
    ]  # fmt: skip


def print_prepare_plan(
    ctx: Ctx, corpus_id: str, src: Path, semantic: bool, arm: str, max_requests: int
) -> None:
    pair, ws = ctx.pair_dir(corpus_id, arm), ctx.workspace(corpus_id, arm)
    home = ctx.agent / "homes" / f"{corpus_id}.{ctx.slot(arm)}"
    builds = ctx.agent / "builds" / f"{corpus_id}.{ctx.slot(arm)}"
    state = (
        "prepared (skipped without --force)"
        if ctx.manifest(corpus_id, arm).exists()
        else "pending"
    )
    print(f"# prepare {corpus_id} [{arm}{', semantic' if semantic else ''}]: {state}")
    print(
        f"rm -rf {shlex.quote(str(pair))}   # only with --force or when the copy did not finish"
    )
    print(
        shlex.join(copy_command(src, ws))
        + " && rm -rf "
        + " ".join(shlex.quote(str(ws / d)) for d in OUT_DIRS)
    )
    if arm == "none":
        print()
        return
    env = scrubbed_env(home / "build")
    if semantic and arm == "graphify":
        print(
            shlex.join(
                shim_command(ctx, "<port>", builds / "attempt-<epoch>", max_requests)
            )
            + " &"
        )
        env.update(
            {
                "OPENAI_BASE_URL": "http://127.0.0.1:<port>/v1",
                "OPENAI_API_KEY": "dummy",
                "OPENAI_MODEL": "codex-shim",
            }
        )
    if semantic and arm == "dijkstral":
        env.update(
            {
                "CODEX_HOME": codex_home(),
                "DIJKSTRAL_CODEX": str(ctx.agent / "bin" / "codex-count.sh"),
                "DIJKSTRAL_BENCH_CALL_LOG": str(builds) + ".calls",
            }
        )
    print(fmt_cmd(build_command(ctx, arm, ws, semantic), env, ws))
    print(
        fmt_cmd(install_command(ctx, arm, semantic), scrubbed_env(home / "install"), ws)
        + "   # HOME must stay empty"
    )
    print()


def prepare_pair(
    ctx: Ctx,
    corpus_id: str,
    src: Path,
    semantic: bool,
    arm: str,
    force: bool,
    max_requests: int,
) -> str:
    """Build one arm's workspace. Returns ok|skipped|failed|quota|home_touched|stopped.

    Resumable: the manifest is written last, so an unfinished pair is redone on the next run;
    a finished copy (`.copied`) is kept so the tools' own caches (semantic chunks) are reused."""
    manifest_path = ctx.manifest(corpus_id, arm)
    pair, ws = ctx.pair_dir(corpus_id, arm), ctx.workspace(corpus_id, arm)
    home = ctx.agent / "homes" / f"{corpus_id}.{ctx.slot(arm)}"
    builds = ctx.agent / "builds"
    label = f"{corpus_id} [{arm}{', semantic' if semantic else ''}]"
    if manifest_path.exists() and not force:
        return "skipped"
    if ctx.stop.is_set():
        return "stopped"
    state_path = builds / f"{corpus_id}.{ctx.slot(arm)}.state.json"
    if force:
        manifest_path.unlink(missing_ok=True)
        state_path.unlink(missing_ok=True)
        rm_tree(pair)
    state = read_json(state_path) or {"attempts": [], "model_calls_total": 0}
    steps: list[dict] = []
    if not (pair / ".copied").exists():
        rm_tree(pair)
        pair.mkdir(parents=True)
        t0 = time.monotonic()
        cmd = copy_command(src, ws)
        cp = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if cp.returncode != 0:
            ctx.log(f"FAILED   prepare {label}: copy failed: {cp.stderr[-300:]}")
            return "failed"
        for d in OUT_DIRS:
            if (ws / d).exists():
                rm_tree(ws / d)
        (pair / ".copied").write_text(str(src))
        steps.append(
            {"step": "copy", "cmd": cmd, "wall_s": round(time.monotonic() - t0, 2)}
        )
    manifest: dict = {
        "corpus_id": corpus_id,
        "arm": arm,
        "corpus": str(src),
        "semantic": semantic,
        "workspace": str(ws),
        "model_calls": 0,
        "tokens": {"input": 0, "output": 0},
    }
    if arm != "none":
        builds.mkdir(parents=True, exist_ok=True)
        rm_tree(home)
        (home / "build").mkdir(parents=True)
        (home / "install").mkdir(parents=True)
        env = scrubbed_env(home / "build")
        shim = shim_log = None
        transcripts = calls_log = None
        if semantic and arm == "graphify":
            transcripts = (
                builds
                / f"{corpus_id}.{ctx.slot(arm)}"
                / f"attempt-{int(time.time() * 1000)}"
            )
            port = free_port()
            transcripts.parent.mkdir(parents=True, exist_ok=True)
            shim_log = open(transcripts.parent / "shim.log", "a")  # noqa: SIM115 - closed below
            shim = subprocess.Popen(
                shim_command(ctx, str(port), transcripts, max_requests),
                stdout=shim_log,
                stderr=shim_log,
            )
            for _ in range(100):
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                        break
                except OSError:
                    time.sleep(0.1)
            env.update(
                {
                    "OPENAI_BASE_URL": f"http://127.0.0.1:{port}/v1",
                    "OPENAI_API_KEY": "dummy",
                    "OPENAI_MODEL": "codex-shim",
                }
            )
        if semantic and arm == "dijkstral":
            calls_log = builds / f"{corpus_id}.{ctx.slot(arm)}.calls"
            calls_log.unlink(missing_ok=True)
            Path(str(calls_log) + ".events.jsonl").unlink(missing_ok=True)
            env.update(
                {
                    "CODEX_HOME": codex_home(),
                    "DIJKSTRAL_CODEX": str(write_codex_counter(ctx)),
                    "DIJKSTRAL_BENCH_CALL_LOG": str(calls_log),
                }
            )
        build = build_command(ctx, arm, ws, semantic)
        t0 = time.monotonic()
        try:
            p = subprocess.run(
                build, env=env, cwd=ws, capture_output=True, text=True, check=False
            )
        finally:
            if shim is not None:
                shim.terminate()
                shim.wait(timeout=10)
            if shim_log is not None:
                shim_log.close()
        wall = round(time.monotonic() - t0, 2)
        log_path = builds / f"{corpus_id}.{ctx.slot(arm)}.build.log"
        log_path.write_text(
            p.stdout[-20000:] + "\n--- stderr ---\n" + p.stderr[-20000:]
        )
        if transcripts is not None:
            calls = (
                len(list(transcripts.glob("req-*.json"))) if transcripts.is_dir() else 0
            )
        elif calls_log is not None:
            calls = len(calls_log.read_text().splitlines()) if calls_log.exists() else 0
        else:
            calls = 0
        events_log = Path(str(calls_log) + ".events.jsonl") if calls_log else None
        quota = semantic and build_quota_hit(
            [p.stdout[-5000:], p.stderr[-5000:]], transcripts, events_log
        )
        state["attempts"].append(
            {"wall_s": wall, "exit": p.returncode, "model_calls": calls, "quota": quota}
        )
        state["model_calls_total"] += calls
        write_json(state_path, state)
        steps.append(
            {"step": "build", "cmd": build, "wall_s": wall, "exit": p.returncode}
        )
        if quota:
            ctx.stop.set()
            ctx.log(
                f"QUOTA    prepare {label}: the semantic build hit a Codex usage limit ({calls} calls); see {log_path}"
            )
            return "quota"
        if p.returncode != 0:
            ctx.log(
                f"FAILED   prepare {label}: build exit {p.returncode}; see {log_path}"
            )
            return "failed"
        out = ws / ("graphify-out" if arm == "graphify" else "dijkstral-out")
        tok = (read_json(out / ".graphify_analysis.json") or {}).get("tokens") or {}
        manifest["tokens"] = {
            "input": tok.get("input", 0),
            "output": tok.get("output", 0),
        }
        manifest["model_calls"] = calls
        manifest["model_calls_all_attempts"] = state["model_calls_total"]
        manifest["build_attempts"] = state["attempts"]
        manifest["graph"] = graph_counts(out / "graph.json")

        install = install_command(ctx, arm, semantic)
        before = snapshot(ws)
        t0 = time.monotonic()
        p = subprocess.run(
            install,
            env=scrubbed_env(home / "install"),
            cwd=ws,
            capture_output=True,
            text=True,
            check=False,
        )
        steps.append(
            {
                "step": "install",
                "cmd": install,
                "wall_s": round(time.monotonic() - t0, 2),
                "exit": p.returncode,
            }
        )
        if p.returncode != 0:
            ctx.log(
                f"FAILED   prepare {label}: `{shlex.join(install)}` exit {p.returncode}: {p.stderr[-300:]}"
            )
            return "failed"
        touched = tree_files(home / "install")
        if touched:
            ctx.stop.set()
            ctx.log(
                f"STOP     prepare {label}: `{shlex.join(install)}` wrote into HOME: {touched[:10]}"
            )
            return "home_touched"
        after = snapshot(ws)
        footprint: dict[str, str | None] = {
            k: after[k] for k in sorted(after) if before.get(k) != after[k]
        }
        footprint.update({k: None for k in before if k not in after})
        manifest["install_footprint"] = footprint
        manifest["hook_commands"] = hook_commands(ws)
        manifest["home_untouched"] = True
        link_tool_dir(ctx, arm, semantic)
    manifest["steps"] = steps
    manifest["versions"] = tool_versions(ctx, arm, semantic)
    manifest["prepared_at"] = time.strftime("%Y-%m-%dT%H:%M:%S")
    write_json(manifest_path, manifest)
    build_s = sum(s["wall_s"] for s in steps if s["step"] == "build")
    ctx.log(
        f"prepared {label}: build {build_s:.1f}s calls {manifest['model_calls']} graph {manifest.get('graph', {})} "
        f"footprint {list(manifest.get('install_footprint', {}))}"
    )
    return "ok"


def link_tool_dir(ctx: Ctx, arm: str, semantic: bool) -> None:
    d = ctx.tool_dir(arm, semantic)
    if d is None:
        return
    d.mkdir(parents=True, exist_ok=True)
    if arm == "graphify":
        targets = {
            n: ctx.graphify_bin(semantic) / n for n in ("graphify", "graphify-mcp")
        }
    else:
        targets = {
            "dijkstral": ctx.dijkstral,
            "dijkstral-live": ctx.dijkstral.parent / "dijkstral-live",
        }
    for name, target in targets.items():
        link = d / name
        if link.is_symlink() and os.readlink(link) == str(target):
            continue
        # Parallel prepares share this directory: make the new link aside and rename it over the
        # old one, so no prepare sees the name missing or finds it taken.
        tmp = d / f".{name}.{os.getpid()}.{threading.get_ident()}"
        tmp.unlink(missing_ok=True)
        tmp.symlink_to(target)
        os.replace(tmp, link)


def tool_versions(ctx: Ctx, arm: str, semantic: bool) -> dict:
    v = {"codex": run_text([ctx.codex, "--version"])}
    if arm == "graphify":
        v["graphify"] = GRAPHIFY_VERSION
        v["graphify_venv"] = str(ctx.graphify_bin(semantic).parent)
    if arm == "dijkstral":
        v["dijkstral"] = run_text([str(ctx.dijkstral), "--version"])
        v["dijkstral_path"] = str(ctx.dijkstral)
        v["dijkstral_sha256"] = binary_sha256(str(ctx.dijkstral))
        v["git_head"] = run_text(["git", "-C", str(BENCH), "rev-parse", "HEAD"])
    return v


@cache
def binary_sha256(path: str) -> str:
    """The build's identity: `dijkstral --version` reads the same for every build."""
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def stale_dijkstral(ctx: Ctx, corpus_id: str, arm: str) -> str | None:
    """Why the arm's prepared workspace came from another dijkstral than `ctx.dijkstral`, or None.

    Its graph, hooks and MCP command come from the build that prepared it: another path is named
    in its hooks, and a rebuild at the same path is caught by the content hash in its manifest
    (manifests written before the hash was recorded are checked by path only)."""
    if arm != "dijkstral":
        return None
    v = (read_json(ctx.manifest(corpus_id, arm)) or {}).get("versions") or {}
    built, sha = v.get("dijkstral_path"), v.get("dijkstral_sha256")
    if built and built != str(ctx.dijkstral):
        return (
            f"{corpus_id} [dijkstral] was prepared with {built} (its hooks.json names it); "
            f"pass --dijkstral {built} or re-run prepare --force"
        )
    if sha and ctx.dijkstral.exists():
        now = binary_sha256(str(ctx.dijkstral))
        if now != sha:
            return (
                f"{corpus_id} [dijkstral] was prepared with another build of {ctx.dijkstral} "
                f"(sha256 {sha[:12]}, now {now[:12]}); re-run prepare --force to rebuild it "
                f"with this one (a semantic workspace calls the model again)"
            )
    return None


def run_text(cmd: list[str]) -> str:
    try:
        return subprocess.run(
            cmd, capture_output=True, text=True, check=False, timeout=30
        ).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return ""


# ---------------------------------------------------------------------------------------------
# Report


def session_rows(ctx: Ctx, tasks: list[dict], arms: list[str]) -> list[dict]:
    rows = []
    for t in tasks:
        for arm in arms:
            paths = session_paths(ctx, arm, t["id"])
            r = read_json(paths["result"])
            row = {
                "task_id": t["id"],
                "suite": t["suite"],
                "kind": t["kind"],
                "corpus_id": t["corpus_id"],
                "arm": arm,
            }
            if not r or r.get("status") != "ok" or r.get("key") != session_key(ctx, t):
                row["status"] = "failed" if paths["failed"].exists() else "missing"
                rows.append(row)
                continue
            answer = graded_answer(r)
            u = r.get("usage") or {}
            row.update(
                {
                    "status": "ok",
                    "input_tokens": u.get("input_tokens", 0),
                    "cached_input_tokens": u.get("cached_input_tokens", 0),
                    "output_tokens": u.get("output_tokens", 0),
                    "reasoning_output_tokens": u.get("reasoning_output_tokens", 0),
                    "tool_calls": r.get("tool_calls", 0),
                    "commands": r.get("commands", 0),
                    "mcp_calls": r.get("mcp_calls", 0),
                    "graph_cli_commands": sum(
                        (r.get("graph_cli_commands") or {}).values()
                    ),
                    "wall_s": r.get("wall_s"),
                    "answer_chars": len(answer),
                }
            )
            if t.get("facts"):
                fs = score.score_facts(t["facts"], answer, t.get("min_facts"))
                row.update(
                    {
                        "recall": fs["recall"],
                        "pass": fs["pass"],
                        "facts_missing": fs["missing"],
                    }
                )
            if t["suite"] != "keyfacts":
                row["score"] = suite_score(t, answer)
            j = read_json(ctx.judge_dir / f"{judge_key(ctx, t, answer)}.json")
            if j and j.get("status") == "ok":
                row["judge_score"] = j["verdict"]["score"]
                row["judge_correct"] = j["verdict"]["correct"]
            rows.append(row)
    return rows


def suite_score(task: dict, answer: str) -> float:
    """The suite's own deterministic score: LOCOMO's per-category rule, LongMemEval's abstention
    rule or F1 proxy, LOCOMO token F1 for anything else."""
    ref = str(task["reference"])
    if task["suite"] == "locomo":
        return memory.locomo_score(answer, ref, task["kind"])
    if task["suite"] == "longmemeval":
        return memory.longmemeval_score(answer, ref, bool(task.get("adversarial")))
    return memory.f1(answer, ref)


def _mean(xs: list) -> float | None:
    xs = [x for x in xs if x is not None]
    return sum(xs) / len(xs) if xs else None


def _median(xs: list) -> float | None:
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def summarise(rows: list[dict]) -> dict:
    ok = [r for r in rows if r["status"] == "ok"]
    tokens = [r["input_tokens"] + r["output_tokens"] for r in ok]
    passes = [r["pass"] for r in ok if r.get("pass") is not None]
    judged = [r for r in ok if "judge_score" in r]
    return {
        "tasks": len(rows),
        "done": len(ok),
        "failed": sum(1 for r in rows if r["status"] == "failed"),
        "missing": sum(1 for r in rows if r["status"] == "missing"),
        "pass_rate": (sum(passes) / len(passes)) if passes else None,
        "mean_recall": _mean([r.get("recall") for r in ok]),
        "mean_score": _mean([r.get("score") for r in ok]),
        "judged": len(judged),
        "judge_mean": _mean([r["judge_score"] for r in judged]),
        "judge_correct_rate": (sum(r["judge_correct"] for r in judged) / len(judged))
        if judged
        else None,
        "tokens_median": _median(tokens),
        "tokens_total": sum(tokens),
        "input_total": sum(r["input_tokens"] for r in ok),
        "cached_total": sum(r["cached_input_tokens"] for r in ok),
        "output_total": sum(r["output_tokens"] for r in ok),
        "reasoning_total": sum(r["reasoning_output_tokens"] for r in ok),
        "tool_calls_mean": _mean([r["tool_calls"] for r in ok]),
        "commands_mean": _mean([r["commands"] for r in ok]),
        "mcp_calls_mean": _mean([r["mcp_calls"] for r in ok]),
        "graph_cli_mean": _mean([r["graph_cli_commands"] for r in ok]),
        "wall_median_s": _median([r["wall_s"] for r in ok]),
        "wall_total_s": sum(r["wall_s"] or 0 for r in ok),
    }


def _f(x, pct: bool = False, nd: int = 2) -> str:
    if x is None:
        return "–"
    if pct:
        return f"{100 * x:.0f}%"
    if isinstance(x, float):
        return f"{x:,.{nd}f}"
    return f"{x:,}"


def render(summary: dict, prep: list[dict]) -> str:
    out = ["## Agent outcome", ""]
    m = summary["meta"]
    out.append(
        f"Model `{m['model']}` (reasoning effort {m['effort']}), {m['codex']}, "
        + (f"Graphify {m['graphify']}, " if m.get("graphify") else "")
        + f"tasks `{m['tasks_file']}`, {m['date']}. Pass = key facts matched (all, or `min_facts`); score = the suite's own rule (LOCOMO per "
        "category, LongMemEval abstention or F1 proxy); judge = blind judge score (0–1; judge "
        f"`{m.get('judge_model', m['model'])}`, effort {m.get('judge_effort', m['effort'])}). "
        "Tokens = input (incl. cached) + output per session."
    )
    out += [
        "",
        "| suite | arm | done | pass | recall | score | judge | judge correct | tokens median | tokens total | cached in | tool calls | MCP calls | graph CLI | wall median s |",
    ]
    out.append("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for g in summary["by_suite"]:
        s = g["stats"]
        out.append(
            f"| {g['suite']} | {g['arm']} | {s['done']}/{s['tasks']} | {_f(s['pass_rate'], True)} | "
            f"{_f(s['mean_recall'])} | {_f(s['mean_score'])} | {_f(s['judge_mean'])} | {_f(s['judge_correct_rate'], True)} | "
            f"{_f(s['tokens_median'], nd=0)} | {_f(s['tokens_total'])} | {_f(s['cached_total'])} | "
            f"{_f(s['tool_calls_mean'], nd=1)} | {_f(s['mcp_calls_mean'], nd=1)} | {_f(s['graph_cli_mean'], nd=1)} | "
            f"{_f(s['wall_median_s'], nd=0)} |"
        )
    out += [
        "",
        "Per kind:",
        "",
        "| suite | kind | arm | done | pass | recall | score | judge | tokens median | tool calls |",
    ]
    out.append("|---|---|---|---|---|---|---|---|---|---|")
    for g in summary["by_kind"]:
        s = g["stats"]
        out.append(
            f"| {g['suite']} | {g['kind']} | {g['arm']} | {s['done']}/{s['tasks']} | {_f(s['pass_rate'], True)} | "
            f"{_f(s['mean_recall'])} | {_f(s['mean_score'])} | {_f(s['judge_mean'])} | {_f(s['tokens_median'], nd=0)} | "
            f"{_f(s['tool_calls_mean'], nd=1)} |"
        )
    if prep:
        out += [
            "",
            "Graph builds (once per corpus and arm, outside the answering sessions):",
            "",
        ]
        out.append(
            "| corpus | arm | semantic | build s | install s | model calls | tokens in / out | nodes | edges |"
        )
        out.append("|---|---|---|---|---|---|---|---|---|")
        for p in prep:
            st = {s["step"]: s for s in p.get("steps", [])}
            g = p.get("graph") or {}
            out.append(
                f"| {p['corpus_id']} | {p['arm']} | {'yes' if p['semantic'] else 'no'} | "
                f"{_f(st.get('build', {}).get('wall_s'))} | {_f(st.get('install', {}).get('wall_s'))} | "
                f"{p.get('model_calls', 0)} | {_f(p['tokens']['input'])} / {_f(p['tokens']['output'])} | "
                f"{_f(g.get('nodes'))} | {_f(g.get('edges'))} |"
            )
    return "\n".join(out) + "\n"


def report(ctx: Ctx, tasks: list[dict], arms: list[str], tasks_file: Path) -> None:
    rows = session_rows(ctx, tasks, arms)
    by_suite, by_kind = [], []
    for suite in sorted({r["suite"] for r in rows}):
        for arm in arms:
            g = [r for r in rows if r["suite"] == suite and r["arm"] == arm]
            by_suite.append({"suite": suite, "arm": arm, "stats": summarise(g)})
        for kind in sorted({r["kind"] for r in rows if r["suite"] == suite}):
            for arm in arms:
                g = [
                    r
                    for r in rows
                    if r["suite"] == suite and r["kind"] == kind and r["arm"] == arm
                ]
                by_kind.append(
                    {"suite": suite, "kind": kind, "arm": arm, "stats": summarise(g)}
                )
    prep = []
    for cid in sorted({t["corpus_id"] for t in tasks}):
        for arm in arms:
            m = read_json(ctx.manifest(cid, arm))
            if m:
                prep.append(
                    {
                        k: m.get(k)
                        for k in (
                            "corpus_id",
                            "arm",
                            "semantic",
                            "steps",
                            "model_calls",
                            "tokens",
                            "graph",
                            "versions",
                            "hook_commands",
                            "install_footprint",
                        )
                    }
                )
    summary = {
        "meta": {
            "date": time.strftime("%Y-%m-%d"),
            "model": ctx.effective_model,
            "effort": ctx.effort,
            "judge_model": "fake-model" if ctx.fake else ctx.judge_model,
            "judge_effort": ctx.judge_effort,
            "build_model": ctx.build_model,
            "build_effort": ctx.build_effort,
            "codex": run_text([ctx.codex, "--version"]),
            "tasks_file": str(tasks_file),
            "arms": arms,
            "graphify": GRAPHIFY_VERSION if "graphify" in arms else None,
            "prompt_version": PROMPT_VERSION,
            "judge_version": JUDGE_VERSION,
            "sessions_dir": str(ctx.sessions_dir),
        },
        "by_suite": by_suite,
        "by_kind": by_kind,
        "builds": prep,
        # Per-session metrics only: answers and raw events stay under the bench home.
        "sessions": [
            {k: v for k, v in r.items() if k != "facts_missing"}
            | {"facts_missing": r.get("facts_missing")}
            for r in rows
        ],
    }
    md = render(summary, prep)
    results = BENCH / "results"
    results.mkdir(exist_ok=True)
    stem = results / f"agent-{int(time.time())}"
    stem.with_suffix(".json").write_text(json.dumps(summary, indent=1))
    stem.with_suffix(".md").write_text(md)
    print(md)
    print(f"wrote {stem.with_suffix('.json')} and {stem.with_suffix('.md')}")


# ---------------------------------------------------------------------------------------------
# CLI


def make_ctx(args: argparse.Namespace) -> Ctx:
    home = bench_home()
    codex = args.codex or shutil.which("codex") or str(Path.home() / ".local/bin/codex")
    dij = Path(
        args.dijkstral
        or os.environ.get("DIJKSTRAL_BIN")
        or CLI / "target/release/dijkstral"
    ).resolve()
    sessions = (
        Path(args.sessions_dir)
        if args.sessions_dir
        else home
        / "agent"
        / ("sessions-fake" if getattr(args, "fake_provider", False) else "sessions")
        / f"{args.model}-{args.effort}"
    )
    judge = (
        Path(args.judge_dir)
        if args.judge_dir
        else home
        / "agent"
        / ("judge-fake" if getattr(args, "fake_provider", False) else "judge")
    )
    return Ctx(
        home=home,
        codex=codex,
        model=args.model,
        effort=args.effort,
        judge_model=args.judge_model,
        judge_effort=args.judge_effort,
        build_model=args.build_model,
        build_effort=args.build_effort,
        timeout=args.timeout,
        retries=args.retries,
        backoff=args.backoff,
        jobs=args.jobs,
        dry_run=args.dry_run,
        graphify_venv=home / "tools" / f"graphify-{GRAPHIFY_VERSION}",
        graphify_semantic_venv=home / "tools" / f"graphify-{GRAPHIFY_VERSION}-semantic",
        dijkstral=dij,
        sessions_dir=sessions,
        judge_dir=judge,
    )


def parse_arms(s: str) -> list[str]:
    arms = [a for a in s.split(",") if a]
    bad = [a for a in arms if a not in ARMS]
    if bad:
        sys.exit(f"unknown arms {bad}; choose from {ARMS}")
    return arms


def check_tools(ctx: Ctx, arms: list[str], tasks: list[dict]) -> None:
    missing = []
    if not Path(ctx.codex).exists():
        missing.append(ctx.codex)
    semantic = {bool(t["semantic"]) for t in tasks}
    if "graphify" in arms:
        for s in semantic:
            for b in ("graphify", "graphify-mcp"):
                if not (ctx.graphify_bin(s) / b).exists():
                    missing.append(str(ctx.graphify_bin(s) / b))
    if "dijkstral" in arms and not ctx.dijkstral.exists():
        missing.append(
            f"{ctx.dijkstral} (cargo build --release in products/dijkstral/cli)"
        )
    if missing and not ctx.dry_run:
        sys.exit("missing tools:\n  " + "\n  ".join(missing))


def cmd_prepare(ctx: Ctx, args, tasks: list[dict], arms: list[str]) -> None:
    check_tools(ctx, arms, tasks)
    pairs = {}
    for t in tasks:
        pairs.setdefault(t["corpus_id"], (Path(t["corpus"]), bool(t["semantic"])))
    for cid, (src, _) in pairs.items():
        if not src.is_dir() and not ctx.dry_run:
            sys.exit(
                f"corpus {src} for {cid!r} not found (bench/setup.sh or memory.py build creates it)"
            )
    items = [
        (cid, src, semantic, arm)
        for cid, (src, semantic) in pairs.items()
        for arm in arms
    ]
    if ctx.dry_run:
        for it in items:
            print_prepare_plan(ctx, *it, args.max_requests)
        return
    pending = [
        it for it in items if args.force or not ctx.manifest(it[0], it[3]).exists()
    ]
    stale = [
        why
        for it in items
        if it not in pending and (why := stale_dijkstral(ctx, it[0], it[3]))
    ]
    if stale:
        sys.exit("\n".join(stale))
    ctx.log(
        f"{len(items)} workspaces, {len(items) - len(pending)} already prepared, {len(pending)} to build, jobs {ctx.jobs}"
    )
    counts = run_queue(
        pending,
        lambda it: prepare_pair(ctx, *it, args.force, args.max_requests),
        ctx.jobs,
        ctx.stop,
    )
    ctx.log(f"finished: {counts}")
    if counts.get("home_touched"):
        sys.exit("stopped: an installer wrote into HOME (see above)")
    quota_exit(ctx, counts)
    if counts.get("failed"):
        sys.exit(1)


def cmd_run(ctx: Ctx, args, tasks: list[dict], arms: list[str]) -> None:
    check_tools(ctx, arms, tasks)
    items = [(t, arm) for t in tasks for arm in arms]
    if ctx.dry_run:
        keep, dropped = base_path()
        print(
            f"# base PATH (dirs providing graphify/dijkstral removed: {dropped or 'none'}):"
        )
        print(f"#   {os.pathsep.join(keep)}")
        print(
            "# env: inherited, minus HERDR_*, GRAPHIFY_*, DIJKSTRAL_*, *_API_KEY, OPENAI_BASE_URL; "
            "plus GRAPHIFY_QUERY_LOG_DISABLE=1"
        )
        for t, arm in items:
            paths = session_paths(ctx, arm, t["id"])
            state = "done (skipped)" if session_done(ctx, t, arm) else "pending"
            prepared = (
                "prepared"
                if ctx.manifest(t["corpus_id"], arm).exists()
                else "NOT PREPARED"
            )
            tool_dir = ctx.tool_dir(arm, bool(t["semantic"]))
            print(
                f"\n# {t['id']} [{arm}] {state}; workspace {prepared}; result -> {paths['result']}"
            )
            path_prefix = (
                f"PATH={tool_dir}:$BASE_PATH " if tool_dir else "PATH=$BASE_PATH "
            )
            print(
                path_prefix
                + "GRAPHIFY_QUERY_LOG_DISABLE=1 "
                + shlex.join(session_command(ctx, t, arm, paths["last"]))
            )
        return
    unprepared = sorted(
        {
            (t["corpus_id"], a)
            for t, a in items
            if not ctx.manifest(t["corpus_id"], a).exists()
        }
    )
    if unprepared:
        sys.exit(
            f"not prepared: {unprepared}; run `prepare` with the same --tasks/--arms first"
        )
    stale = [
        why
        for cid, arm in sorted({(t["corpus_id"], a) for t, a in items})
        if (why := stale_dijkstral(ctx, cid, arm))
    ]
    if stale:
        sys.exit("\n".join(stale))
    pending = [(t, a) for t, a in items if not session_done(ctx, t, a)]
    ctx.log(
        f"{len(items)} sessions, {len(items) - len(pending)} already done, {len(pending)} to run, jobs {ctx.jobs}"
    )
    counts = run_queue(
        pending, lambda it: execute_session(ctx, it[0], it[1]), ctx.jobs, ctx.stop
    )
    ctx.log(f"finished: {counts}")
    quota_exit(ctx, counts)
    if counts.get("failed"):
        sys.exit(1)


def cmd_judge(ctx: Ctx, args, tasks: list[dict], arms: list[str]) -> None:
    items = []
    for t in tasks:
        for arm in arms:
            r = read_json(session_paths(ctx, arm, t["id"])["result"])
            if r and r.get("status") == "ok" and r.get("key") == session_key(ctx, t):
                items.append((t, graded_answer(r)))
    # Blind: the judge sees one answer at a time, no arm name, in shuffled order; identical
    # answers share one verdict.
    uniq = {judge_key(ctx, t, a): (t, a) for t, a in items}
    order = list(uniq.values())
    random.Random(args.seed).shuffle(order)
    if ctx.dry_run:
        if not order:
            t = tasks[0]
            order = [(t, "<candidate answer>")]
            print(
                "# no session results yet; showing the judge command for a placeholder answer"
            )
        t, a = order[0]
        print(f"# {len(uniq)} distinct answers to judge; first ({t['id']}):")
        prompt, structured = judge_spec(t, a)
        schema = Path("<schema.json>") if structured else None
        print(
            shlex.join(
                judge_command(
                    ctx, Path("<empty scratch dir>"), schema, Path("<last.txt>"), prompt
                )
            )
        )
        print(
            "# schema: "
            + (
                json.dumps(JUDGE_SCHEMA)
                if structured
                else "none (LongMemEval yes/no prompt)"
            )
        )
        return
    pending = [
        (t, a)
        for t, a in order
        if (read_json(ctx.judge_dir / f"{judge_key(ctx, t, a)}.json") or {}).get(
            "status"
        )
        != "ok"
    ]
    ctx.log(
        f"{len(uniq)} distinct answers, {len(uniq) - len(pending)} already judged, {len(pending)} to judge"
    )
    counts = run_queue(
        pending, lambda it: execute_judge(ctx, it[0], it[1]), ctx.jobs, ctx.stop
    )
    ctx.log(f"finished: {counts}")
    quota_exit(ctx, counts)
    if counts.get("failed"):
        sys.exit(1)


def main(argv: list[str] | None = None) -> None:
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--tasks", required=True, type=Path, help="task file (JSONL)")
    common.add_argument("--arms", default=",".join(ARMS))
    common.add_argument("--limit", type=int, default=0, help="first N tasks only")
    common.add_argument("--ids", default="", help="comma-separated task ids")
    common.add_argument("--jobs", type=int, default=2, help="concurrent Codex sessions")
    common.add_argument(
        "--dry-run", action="store_true", help="print the exact commands; call no model"
    )
    common.add_argument("--model", default=MODEL, help="the answering model")
    common.add_argument("--effort", default=EFFORT, help="model_reasoning_effort")
    common.add_argument("--judge-model", default=MODEL)
    common.add_argument("--judge-effort", default=EFFORT)
    common.add_argument(
        "--build-model", default=MODEL, help="model of the semantic graph builds"
    )
    common.add_argument("--build-effort", default=EFFORT)
    common.add_argument(
        "--timeout", type=float, default=900, help="seconds per Codex call"
    )
    common.add_argument(
        "--retries", type=int, default=2, help="retries after a transient failure"
    )
    common.add_argument(
        "--backoff", type=float, default=20, help="seconds before retry N (times N)"
    )
    common.add_argument("--codex", default="", help="codex binary (default: on PATH)")
    common.add_argument(
        "--dijkstral", default="", help="dijkstral binary (default: cli/target/release)"
    )
    common.add_argument(
        "--sessions-dir",
        default="",
        help="default: $DIJKSTRAL_BENCH_HOME/agent/sessions",
    )
    common.add_argument(
        "--judge-dir", default="", help="default: $DIJKSTRAL_BENCH_HOME/agent/judge"
    )
    p = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = p.add_subparsers(dest="cmd", required=True)
    sp = sub.add_parser(
        "prepare", parents=[common], help="build one workspace per (corpus_id, arm)"
    )
    sp.add_argument("--force", action="store_true", help="rebuild prepared workspaces")
    sp.add_argument(
        "--max-requests",
        type=int,
        default=200,
        help="Codex call cap for a Graphify semantic build",
    )
    for name, help_ in (
        ("run", "answering sessions"),
        ("judge", "blind judge over stored answers"),
    ):
        sp = sub.add_parser(name, parents=[common], help=help_)
        sp.add_argument(
            "--fake-provider",
            action="store_true",
            help="local fake model: exercises codex, MCP and parsing without a model call",
        )
        if name == "judge":
            sp.add_argument(
                "--seed", type=int, default=7, help="shuffle seed for the judging order"
            )
    sp = sub.add_parser(
        "report", parents=[common], help="markdown table and JSON summary"
    )
    sp.add_argument(
        "--fake-provider",
        action="store_true",
        help="report the --fake-provider results",
    )
    args = p.parse_args(argv)
    tasks = select(load_tasks(args.tasks), args.ids, args.limit)
    arms = parse_arms(args.arms)
    ctx = make_ctx(args)
    fake = None
    ctx.fake = bool(getattr(args, "fake_provider", False))
    if ctx.fake and args.cmd in ("run", "judge"):
        import fake_provider

        fake = fake_provider.FakeProvider()
        ctx.fake_provider_flags = fake.codex_flags()
    try:
        {"prepare": cmd_prepare, "run": cmd_run, "judge": cmd_judge}.get(
            args.cmd, lambda c, a, t, r: report(c, t, r, args.tasks)
        )(ctx, args, tasks, arms)
    finally:
        if fake:
            fake.stop()


if __name__ == "__main__":
    main()
