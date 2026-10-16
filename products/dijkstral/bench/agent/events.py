"""Parse `codex exec --json` event streams and classify how a run ended. Stdlib only.

Event shapes observed on codex-cli 0.162.1 (one JSON object per line):

    {"type": "thread.started", "thread_id": "..."}
    {"type": "turn.started"}
    {"type": "item.started" | "item.updated" | "item.completed", "item": {"id": "item_N", "type": T, ...}}
        T = "agent_message"      {"text"}
            "reasoning"          {"text"}
            "command_execution"  {"command": "/bin/zsh -lc '...'", "aggregated_output", "exit_code", "status"}
            "mcp_tool_call"      {"server", "tool", "arguments", "result", "error", "status"}
            "error"              {"message"}   (also non-fatal warnings, e.g. hook-trust bypass)
            others (file_change, web_search, todo_list, ...) are counted as tool calls by type
    {"type": "turn.completed", "usage": {"input_tokens", "cached_input_tokens",
        "cache_write_input_tokens", "output_tokens", "reasoning_output_tokens"}}
    {"type": "turn.failed", "error": {"message"}}
    {"type": "error", "message": "..."}

`input_tokens` includes the cached part. Only completed items are counted.
"""

from __future__ import annotations

import json
import os
import re
import shlex
from collections import Counter

USAGE_KEYS = (
    "input_tokens",
    "cached_input_tokens",
    "cache_write_input_tokens",
    "output_tokens",
    "reasoning_output_tokens",
)
NON_TOOL_ITEMS = {"agent_message", "reasoning", "error"}

# Usage-limit / quota exhaustion (Codex's own messages and error codes). On any of these the
# runner stops launching sessions instead of retrying: a retry cannot succeed until the limit
# resets, and a rerun of the same command resumes where it stopped.
QUOTA_PATTERNS = re.compile(
    r"hit your usage limit|usage[ _-]limit|usage_limit_reached|quota exceeded|insufficient_quota"
    r"|credits_depleted|purchase more credits|rate[ _-]limit(?:ed| reached|_reached)?"
    r"|too many requests|\b429\b",
    re.IGNORECASE,
)
# Lines of stderr that can carry a fatal error (Codex logs ERROR lines at its default level).
_STDERR_ERROR = re.compile(r"error|fatal|exceeded", re.IGNORECASE)


def parse_events(lines) -> dict:
    """Summarise an iterable of JSONL lines (str) into answer, usage, tool counts and errors."""
    usage = dict.fromkeys(USAGE_KEYS, 0)
    usage_seen = False
    messages: list[str] = []
    errors: list[str] = []
    fatal: list[str] = []
    by_type: Counter[str] = Counter()
    mcp: Counter[str] = Counter()
    mcp_failed = 0
    programs: Counter[str] = Counter()
    graph_cli = Counter()
    thread_id = None
    turns_completed = 0
    bad_lines = 0
    for line in lines:
        line = line.strip()
        if not line:
            continue
        try:
            ev = json.loads(line)
        except json.JSONDecodeError:
            bad_lines += 1
            continue
        if not isinstance(ev, dict):
            bad_lines += 1
            continue
        t = ev.get("type")
        if t == "thread.started":
            thread_id = ev.get("thread_id")
        elif t == "turn.completed":
            turns_completed += 1
            u = ev.get("usage") or {}
            usage_seen = usage_seen or bool(u)
            for k in USAGE_KEYS:
                v = u.get(k)
                if isinstance(v, int):
                    usage[k] += v
        elif t == "turn.failed":
            fatal.append(_msg((ev.get("error") or {}).get("message")))
        elif t == "error":
            fatal.append(_msg(ev.get("message")))
        elif t == "item.completed":
            item = ev.get("item") or {}
            it = item.get("type") or "unknown"
            if it == "agent_message":
                messages.append(item.get("text") or "")
            elif it == "error":
                errors.append(_msg(item.get("message")))
            elif it not in NON_TOOL_ITEMS:
                by_type[it] += 1
                if it == "mcp_tool_call":
                    mcp[f"{item.get('server')}.{item.get('tool')}"] += 1
                    if item.get("status") == "failed" or item.get("error"):
                        mcp_failed += 1
                elif it == "command_execution":
                    cmd = item.get("command") or ""
                    programs[command_program(cmd)] += 1
                    for tool in ("graphify", "dijkstral"):
                        if re.search(rf"(?<![\w-]){tool}(?![\w-])", cmd):
                            graph_cli[tool] += 1
    return {
        "thread_id": thread_id,
        "answer": messages[-1].strip() if messages else "",
        "messages": len(messages),
        "usage": usage if usage_seen else None,
        "turns_completed": turns_completed,
        "tool_calls": sum(by_type.values()),
        "tool_calls_by_type": dict(by_type),
        "commands": by_type.get("command_execution", 0),
        "commands_by_program": dict(programs),
        "graph_cli_commands": dict(graph_cli),
        "mcp_calls": sum(mcp.values()),
        "mcp_calls_by_tool": dict(mcp),
        "mcp_failed": mcp_failed,
        "errors": errors,
        "fatal": fatal,
        "bad_lines": bad_lines,
    }


def _msg(m) -> str:
    return str(m or "")[:500]


def command_program(command: str) -> str:
    """The first program of a Codex shell command (`/bin/zsh -lc 'rg x | head'` -> `rg`)."""
    try:
        argv = shlex.split(command)
    except ValueError:
        argv = command.split()
    if (
        len(argv) >= 3
        and os.path.basename(argv[0]) in ("bash", "zsh", "sh")
        and argv[1].startswith("-")
    ):
        try:
            inner = shlex.split(argv[2])
        except ValueError:
            inner = argv[2].split()
        argv = inner
    # Skip leading environment assignments and `cd dir &&`.
    while argv and ("=" in argv[0] and not argv[0].startswith("=")):
        argv = argv[1:]
    if len(argv) >= 3 and argv[0] == "cd" and argv[2] in ("&&", ";"):
        argv = argv[3:]
    return os.path.basename(argv[0]) if argv else ""


def classify(
    parsed: dict, returncode: int | None, stderr: str, timed_out: bool = False
) -> str:
    """`ok`, `quota` (stop the run, do not retry) or `transient` (bounded retry).

    A run is `ok` when Codex exited 0 with a final answer and no failed turn. Otherwise the
    fatal messages (turn.failed, top-level error events), error items and stderr error lines
    are checked for usage-limit wording first.
    """
    if (
        not timed_out
        and returncode == 0
        and parsed.get("answer")
        and not parsed.get("fatal")
    ):
        return "ok"
    texts = list(parsed.get("fatal") or []) + list(parsed.get("errors") or [])
    texts += [ln for ln in (stderr or "").splitlines() if _STDERR_ERROR.search(ln)]
    if any(QUOTA_PATTERNS.search(t) for t in texts):
        return "quota"
    return "transient"
