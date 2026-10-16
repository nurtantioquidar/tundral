#!/usr/bin/env python3
"""Bench-only OpenAI-compatible chat server that answers through `codex exec`.

    uv run --no-project bench/llm_shim.py --port 8765 --transcript-dir /tmp/run1 --corpus <dir>
    OPENAI_BASE_URL=http://127.0.0.1:8765/v1 OPENAI_API_KEY=dummy OPENAI_MODEL=codex-shim \\
      graphify extract <dir> --backend openai

Graphify (and later dijkstral) has no Codex backend; this lets the semantic pass run on a
ChatGPT-subscription Codex login instead of a paid API key. It serves `GET /v1/models` and
`POST /v1/chat/completions` (non-streaming and SSE streaming) on localhost only.

Responders (`--responder`):

- `codex` (default): each request becomes one `codex exec` run in a fresh empty temp directory
  (read-only sandbox, ephemeral session, user config, rules and project docs ignored), with the
  model and reasoning effort passed explicitly (`--model`, default `gpt-6.1-sol`;
  `--reasoning-effort`, default `medium`). Image parts (`data:` URLs) are attached with `-i`;
  `response_format` json_object/json_schema adds a JSON-only instruction (json_schema also
  `--output-schema`). Codex's final message becomes the assistant reply; its token usage is
  reported as the response `usage`.
- `replay`: answers from a transcript directory written earlier, matched by request fingerprint
  (sha256 of the canonical messages and response_format), else in recorded order. No Codex call.
- `static`: answers every request with `--static-response` (text or `@file`). No Codex call; for
  probing request shape, batching and concurrency for free. `--fail-first N` returns HTTP 500 to
  the first N requests (retry probes).

Clean room: with `--transcript-dir` every request is saved as `req-NNNN.json` holding only its
structure (roles, part types, sizes, sha256, which `--corpus` files appear in it and where, the
quoted identifier tokens and JSON keys it names, request parameters, response_format with
descriptions redacted) plus the full model reply, usage and latency. Request text itself is never
written: it appears as `<prompt text: N chars>`. `--max-requests` caps Codex calls per run (default
40); later requests get HTTP 429.

Stdlib only.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

PREAMBLE = (
    "You are acting as a plain chat-completion model behind an API. Do not run commands, read or "
    "write files, or use any tool. Read the conversation below and reply with exactly the "
    "assistant message that should come next, with nothing before or after it."
)
JSON_ONLY = (
    "Your reply must be a single valid JSON object and nothing else (no code fences)."
)


class State:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.lock = threading.Lock()
        self.count = 0
        self.codex_calls = 0
        self.fail_left = args.fail_first
        self.sem = threading.Semaphore(args.parallel)
        self.markers = (
            [m for m in Path(args.markers).read_text().splitlines() if m.strip()]
            if args.markers
            else []
        )
        self.replay: list[dict[str, Any]] = []
        self.replay_used: set[int] = set()
        if args.responder == "replay":
            if not args.replay_dir:
                sys.exit("--responder replay needs --replay-dir")
            for p in sorted(Path(args.replay_dir).glob("req-*.json")):
                self.replay.append(json.loads(p.read_text()))
        if args.transcript_dir:
            Path(args.transcript_dir).mkdir(parents=True, exist_ok=True)


def load_corpus(root: str) -> list[tuple[str, str]]:
    """(relative path, text) for every corpus file; binary files get "" so only their path is located."""
    out = []
    base = Path(root)
    for p in sorted(base.rglob("*")):
        rel = p.relative_to(base)
        out_dir = rel.parts[0] in ("graphify-out", "dijkstral-out")
        if not p.is_file() or (out_dir and "converted" not in rel.parts):
            continue
        try:
            text = p.read_text()
        except (UnicodeDecodeError, OSError):
            text = ""
        out.append((rel.as_posix(), text))
    return out


def fingerprint(body: dict[str, Any]) -> str:
    key = {
        "messages": body.get("messages"),
        "response_format": body.get("response_format"),
    }
    return hashlib.sha256(json.dumps(key, sort_keys=True).encode()).hexdigest()


def redact_schema(obj: Any) -> Any:
    if isinstance(obj, dict):
        return {
            k: (
                f"<description: {len(v)} chars>"
                if k == "description" and isinstance(v, str)
                else redact_schema(v)
            )
            for k, v in obj.items()
        }
    if isinstance(obj, list):
        return [redact_schema(v) for v in obj]
    return obj


QUOTED = re.compile(r"""["'`]([A-Za-z_][A-Za-z0-9_]*)["'`]""")
JSON_KEY = re.compile(r""""([A-Za-z_][A-Za-z0-9_]*)"\s*:""")
HEX64 = re.compile(r"\b[0-9a-f]{64}\b")
MIN_CONTENT = 8


def find_all(text: str, needle: str) -> list[int]:
    out, i = [], text.find(needle)
    while needle and i >= 0:
        out.append(i)
        i = text.find(needle, i + 1)
    return out


def describe_text(
    text: str, corpus: list[tuple[str, str]], markers: list[str]
) -> dict[str, Any]:
    """Structure of one text: sizes, hash, and where known corpus content, paths and markers sit.

    `layout` lists, in text order, the spans that are corpus data (file contents, path mentions,
    marker strings, sha256 of a file); everything between them is Graphify framing and is only
    counted (`gap` = number of characters), never stored.
    """
    d: dict[str, Any] = {
        "text": f"<prompt text: {len(text)} chars>",
        "chars": len(text),
        "lines": text.count("\n") + 1,
        "sha256": hashlib.sha256(text.encode()).hexdigest(),
        "md5": hashlib.md5(text.encode()).hexdigest(),
        "sha1": hashlib.sha1(text.encode()).hexdigest(),
    }
    spans: list[tuple[int, int, dict[str, Any]]] = []
    digests = {}
    for rel, content in corpus:
        for off in find_all(text, rel):
            spans.append((off, off + len(rel), {"kind": "path", "path": rel}))
        if len(content.strip()) < MIN_CONTENT:
            continue  # binary or too short to locate without false hits
        digests[hashlib.sha256(content.encode()).hexdigest()] = rel
        found = find_all(text, content)
        body = content
        if not found and content.strip():
            body = content.strip()
            found = find_all(text, body)
        for off in found:
            spans.append(
                (
                    off,
                    off + len(body),
                    {"kind": "file_content", "path": rel, "stripped": body != content},
                )
            )
        if not found:
            # Report the longest prefix of the file that does appear (content altered or truncated).
            lo, hi = 0, len(content)
            while lo < hi:
                mid = (lo + hi + 1) // 2
                if content[:mid] in text:
                    lo = mid
                else:
                    hi = mid - 1
            if lo >= 40:
                off = text.find(content[:lo])
                spans.append(
                    (
                        off,
                        off + lo,
                        {"kind": "file_prefix", "path": rel, "of": len(content)},
                    )
                )
    for m in markers:
        for off in find_all(text, m):
            spans.append((off, off + len(m), {"kind": "marker", "marker": m}))
    for mt in HEX64.finditer(text):
        if mt.group() in digests:
            spans.append(
                (
                    mt.start(),
                    mt.end(),
                    {"kind": "sha256_of_file", "path": digests[mt.group()]},
                )
            )
    spans.sort(key=lambda s: (s[0], -(s[1] - s[0])))
    layout, pos = [], 0
    for start, end, info in spans:
        if start < pos and info["kind"] in ("path", "marker", "sha256_of_file"):
            info = {**info, "inside_previous": True}
        elif start > pos:
            layout.append({"gap": start - pos})
        layout.append({"at": start, "len": end - start, **info})
        pos = max(pos, end)
    if pos < len(text) and spans:
        layout.append({"gap": len(text) - pos})
    if layout:
        d["layout"] = layout
    d["json_keys"] = sorted(set(JSON_KEY.findall(text)))
    d["quoted_tokens"] = sorted(
        set(QUOTED.findall(text)) - set(d["json_keys"]) - set(HEX64.findall(text))
    )
    return d


def describe_request(
    body: dict[str, Any], corpus: list[tuple[str, str]], markers: list[str]
) -> dict[str, Any]:
    msgs = []
    for m in body.get("messages", []):
        c = m.get("content")
        entry: dict[str, Any] = {"role": m.get("role"), "keys": sorted(m)}
        if isinstance(c, str):
            entry["content"] = describe_text(c, corpus, markers)
        elif isinstance(c, list):
            parts = []
            for part in c:
                t = part.get("type")
                if t == "text":
                    parts.append(
                        {
                            "type": "text",
                            **describe_text(part.get("text", ""), corpus, markers),
                        }
                    )
                elif t == "image_url":
                    url = (part.get("image_url") or {}).get("url", "")
                    head = (
                        url.split(",", 1)[0]
                        if url.startswith("data:")
                        else "<remote url>"
                    )
                    parts.append(
                        {
                            "type": "image_url",
                            "url_head": head,
                            "url_chars": len(url),
                            "detail": (part.get("image_url") or {}).get("detail"),
                            "sha256": hashlib.sha256(url.encode()).hexdigest(),
                        }
                    )
                else:
                    parts.append({"type": t, "keys": sorted(part)})
            entry["content"] = parts
        else:
            entry["content"] = repr(type(c).__name__)
        msgs.append(entry)
    params = {
        k: v
        for k, v in body.items()
        if k not in ("messages", "response_format", "tools")
    }
    out: dict[str, Any] = {"params": params, "messages": msgs}
    if "response_format" in body:
        out["response_format"] = redact_schema(body["response_format"])
    if "tools" in body:
        out["tools"] = redact_schema(body["tools"])
    return out


def flatten(body: dict[str, Any], tmp: Path) -> tuple[str, list[str], Any]:
    lines = [PREAMBLE, ""]
    images: list[str] = []
    for m in body.get("messages", []):
        lines.append(f"<<{m.get('role', 'user')}>>")
        c = m.get("content")
        if isinstance(c, str):
            lines.append(c)
        elif isinstance(c, list):
            for part in c:
                if part.get("type") == "text":
                    lines.append(part.get("text", ""))
                elif part.get("type") == "image_url":
                    url = (part.get("image_url") or {}).get("url", "")
                    if url.startswith("data:"):
                        head, data = url.split(",", 1)
                        ext = head.split("/")[1].split(";")[0] if "/" in head else "png"
                        p = tmp / f"image{len(images)}.{ext}"
                        p.write_bytes(base64.b64decode(data))
                        images.append(str(p))
                        lines.append(f"[attached image {len(images)}]")
        lines.append("")
    rf = body.get("response_format") or {}
    schema = None
    if rf.get("type") in ("json_object", "json_schema"):
        lines.append(JSON_ONLY)
        if rf.get("type") == "json_schema":
            schema = (rf.get("json_schema") or {}).get("schema")
    lines.append("<<assistant>>")
    return "\n".join(lines), images, schema


def run_codex(st: State, body: dict[str, Any]) -> dict[str, Any]:
    a = st.args
    tmp = Path(tempfile.mkdtemp(prefix="llm-shim-"))
    try:
        prompt, images, schema = flatten(body, tmp)
        cmd = [
            a.codex,
            "exec",
            "--ephemeral",
            "--skip-git-repo-check",
            "--ignore-user-config",
            "--ignore-rules",
            "-s",
            "read-only",
            "-C",
            str(tmp),
            "-m",
            a.model,
            "-c",
            f'model_reasoning_effort="{a.reasoning_effort}"',
            "-c",
            "project_doc_max_bytes=0",
            "--json",
            "-o",
            str(tmp / "last.txt"),
            "--color",
            "never",
        ]
        for img in images:
            cmd += ["-i", img]
        if schema is not None and a.output_schema:
            (tmp / "schema.json").write_text(json.dumps(schema))
            cmd += ["--output-schema", str(tmp / "schema.json")]
        cmd.append("-")
        t0 = time.monotonic()
        proc = subprocess.run(
            cmd,
            check=False,
            input=prompt,
            capture_output=True,
            text=True,
            timeout=a.timeout,
            cwd=tmp,
        )
        latency = time.monotonic() - t0
        usage: dict[str, Any] = {}
        tool_items = 0
        # Codex's own fatal messages (e.g. a usage limit); never request text.
        errors: list[str] = []
        for line in proc.stdout.splitlines():
            try:
                ev = json.loads(line)
            except json.JSONDecodeError:
                continue
            if ev.get("type") == "turn.completed":
                usage = ev.get("usage", {})
            elif ev.get("type") == "turn.failed":
                errors.append(str((ev.get("error") or {}).get("message", ""))[:300])
            elif ev.get("type") == "error":
                errors.append(str(ev.get("message", ""))[:300])
            item = ev.get("item") or {}
            if ev.get("type") == "item.completed" and item.get("type") not in (
                "agent_message",
                "reasoning",
            ):
                tool_items += 1
        last = tmp / "last.txt"
        text = last.read_text() if last.exists() else ""
        return {
            "ok": proc.returncode == 0 and bool(text),
            "text": text,
            "usage": usage,
            "latency_s": round(latency, 2),
            "returncode": proc.returncode,
            "tool_items": tool_items,
            "stderr_tail": proc.stderr[-400:] if proc.returncode else "",
            "errors": errors,
            "images": len(images),
            "output_schema": schema is not None and a.output_schema,
        }
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def answer(st: State, body: dict[str, Any]) -> tuple[int, dict[str, Any]]:
    a = st.args
    with st.lock:
        st.count += 1
        n = st.count
        failing = st.fail_left > 0
        if failing:
            st.fail_left -= 1
    fp = fingerprint(body)
    rec: dict[str, Any] = {
        "index": n,
        "time": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "t_start": round(time.time(), 3),
        "fingerprint": fp,
        "responder": a.responder,
        "codex_model": a.model if a.responder == "codex" else None,
        "reasoning_effort": a.reasoning_effort if a.responder == "codex" else None,
        "request": describe_request(
            body, load_corpus(a.corpus) if a.corpus else [], st.markers
        ),
    }
    status = 200
    result: dict[str, Any]
    if failing:
        status, result = 500, {"ok": False, "text": "", "note": "--fail-first"}
    elif a.responder == "static":
        text = a.static_response
        if text.startswith("@"):
            src = Path(text[1:])
            text = src.read_text()
            if src.suffix == ".jsonl":
                # One JSON-encoded reply string per line, served in request order (cycling).
                replies = [
                    json.loads(line) for line in text.splitlines() if line.strip()
                ]
                text = replies[(n - 1) % len(replies)]
        time.sleep(a.static_delay)
        result = {
            "ok": True,
            "text": text,
            "usage": {"input_tokens": 0, "output_tokens": a.static_output_tokens}
            if a.static_output_tokens
            else {},
            "latency_s": a.static_delay,
            "finish_reason": a.finish_reason,
        }
    elif a.responder == "replay":
        found = replay(st, fp)
        if found is None:
            status, result = 500, {"ok": False, "text": "", "note": "no replay entry"}
        else:
            result = found
    else:
        with st.lock:
            over = st.codex_calls >= a.max_requests
            if not over:
                st.codex_calls += 1
        if over:
            status, result = (
                429,
                {"ok": False, "text": "", "note": "--max-requests reached"},
            )
        else:
            with st.sem:
                try:
                    result = run_codex(st, body)
                except subprocess.TimeoutExpired:
                    result = {"ok": False, "text": "", "note": "codex timeout"}
            if not result["ok"]:
                status = 502
    rec["t_end"] = round(time.time(), 3)
    rec["status"] = status
    rec["response"] = result
    if a.transcript_dir:
        Path(a.transcript_dir, f"req-{n:04d}.json").write_text(
            json.dumps(rec, indent=1, ensure_ascii=False) + "\n"
        )
    usage_raw = result.get("usage")
    u: dict[str, Any] = usage_raw if isinstance(usage_raw, dict) else {}
    print(
        f"[shim] #{n} {status} {a.responder} in={u.get('input_tokens', '-')} cached={u.get('cached_input_tokens', '-')} "
        f"out={u.get('output_tokens', '-')} {result.get('latency_s', '-')}s chars={len(str(result.get('text', '')))}",
        file=sys.stderr,
        flush=True,
    )
    return status, result


def replay(st: State, fp: str) -> dict[str, Any] | None:
    with st.lock:
        # A fingerprint match is reusable (the same run replayed twice); only the in-order
        # fallback for unmatched requests consumes entries.
        for i, r in enumerate(st.replay):
            if r.get("fingerprint") == fp and r.get("status") == 200:
                st.replay_used.add(i)
                return r["response"]
        for i, r in enumerate(st.replay):
            if i not in st.replay_used and r.get("status") == 200:
                st.replay_used.add(i)
                print(
                    f"[shim] replay: no fingerprint match, using recorded #{r.get('index')}",
                    file=sys.stderr,
                )
                return r["response"]
    return None


class Handler(BaseHTTPRequestHandler):
    server_version = "llm-shim/1"
    st: State

    def log_message(self, format: str, *args: Any) -> None:
        pass

    def send_json(self, status: int, obj: dict[str, Any]) -> None:
        data = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:
        if self.path.rstrip("/").endswith("/models"):
            self.send_json(
                200,
                {
                    "object": "list",
                    "data": [
                        {
                            "id": self.st.args.served_model,
                            "object": "model",
                            "owned_by": "llm-shim",
                        }
                    ],
                },
            )
        else:
            self.send_json(404, {"error": {"message": "not found"}})

    def do_POST(self) -> None:
        if not self.path.rstrip("/").endswith("/chat/completions"):
            self.send_json(404, {"error": {"message": f"unsupported path {self.path}"}})
            return
        body = json.loads(
            self.rfile.read(int(self.headers.get("Content-Length", "0"))) or b"{}"
        )
        status, result = answer(self.st, body)
        if status != 200:
            self.send_json(
                status,
                {
                    "error": {
                        "message": result.get("note")
                        or "; ".join(result.get("errors") or [])
                        or result.get("stderr_tail")
                        or "codex failed",
                        "type": "shim_error",
                    }
                },
            )
            return
        u = result.get("usage") or {}
        usage = {
            "prompt_tokens": u.get("input_tokens", 0),
            "completion_tokens": u.get("output_tokens", 0),
            "total_tokens": u.get("input_tokens", 0) + u.get("output_tokens", 0),
        }
        cid, created, model = (
            f"chatcmpl-shim-{int(time.time() * 1000)}",
            int(time.time()),
            body.get("model", ""),
        )
        if body.get("stream"):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.end_headers()
            base = {
                "id": cid,
                "object": "chat.completion.chunk",
                "created": created,
                "model": model,
            }
            chunks = [
                {
                    **base,
                    "choices": [
                        {
                            "index": 0,
                            "delta": {"role": "assistant", "content": result["text"]},
                            "finish_reason": None,
                        }
                    ],
                },
                {
                    **base,
                    "choices": [
                        {
                            "index": 0,
                            "delta": {},
                            "finish_reason": result.get("finish_reason", "stop"),
                        }
                    ],
                },
            ]
            if (body.get("stream_options") or {}).get("include_usage"):
                chunks.append({**base, "choices": [], "usage": usage})
            for c in chunks:
                self.wfile.write(f"data: {json.dumps(c)}\n\n".encode())
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
            return
        self.send_json(
            200,
            {
                "id": cid,
                "object": "chat.completion",
                "created": created,
                "model": model,
                "choices": [
                    {
                        "index": 0,
                        "message": {"role": "assistant", "content": result["text"]},
                        "finish_reason": result.get("finish_reason", "stop"),
                    }
                ],
                "usage": usage,
            },
        )


def main() -> None:
    p = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    p.add_argument("--host", default="127.0.0.1", help="bind address (keep it local)")
    p.add_argument("--port", type=int, default=8765)
    p.add_argument(
        "--responder", choices=["codex", "replay", "static"], default="codex"
    )
    p.add_argument(
        "--codex",
        default=shutil.which("codex") or str(Path.home() / ".local/bin/codex"),
        help="codex binary",
    )
    p.add_argument(
        "--model",
        default="gpt-6.1-sol",
        help="Codex model passed as -m (default gpt-6.1-sol)",
    )
    p.add_argument(
        "--reasoning-effort",
        default="medium",
        help="model_reasoning_effort for codex (default medium)",
    )
    p.add_argument(
        "--served-model", default="codex-shim", help="model id listed by /v1/models"
    )
    p.add_argument("--timeout", type=int, default=900, help="seconds per codex run")
    p.add_argument("--parallel", type=int, default=2, help="concurrent codex runs")
    p.add_argument(
        "--max-requests",
        type=int,
        default=40,
        help="cap on codex runs per shim process",
    )
    p.add_argument(
        "--no-output-schema",
        dest="output_schema",
        action="store_false",
        help="do not pass json_schema to --output-schema",
    )
    p.add_argument("--transcript-dir", help="write redacted req-NNNN.json records here")
    p.add_argument(
        "--corpus",
        help="directory whose files are located inside requests (recorded as offsets)",
    )
    p.add_argument(
        "--markers",
        help="file with one string per line to locate inside requests (e.g. PDF text lines)",
    )
    p.add_argument("--replay-dir", help="transcript directory for --responder replay")
    p.add_argument(
        "--static-response",
        default='{"nodes": [], "edges": [], "hyperedges": []}',
        help="reply text or @file for --responder static",
    )
    p.add_argument(
        "--static-delay",
        type=float,
        default=0.0,
        help="seconds to wait before a static reply (concurrency probes)",
    )
    p.add_argument(
        "--finish-reason",
        default="stop",
        help="finish_reason of static replies (e.g. length)",
    )
    p.add_argument(
        "--static-output-tokens",
        type=int,
        default=0,
        help="usage.completion_tokens of static replies",
    )
    p.add_argument(
        "--fail-first",
        type=int,
        default=0,
        help="answer the first N requests with HTTP 500",
    )
    args = p.parse_args()
    Handler.st = State(args)
    srv = ThreadingHTTPServer((args.host, args.port), Handler)
    print(
        f"[shim] {args.responder} on http://{args.host}:{args.port}/v1 model={args.model} effort={args.reasoning_effort}",
        file=sys.stderr,
        flush=True,
    )
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
