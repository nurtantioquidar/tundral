"""A local stand-in for the OpenAI Responses API, so the harness can drive real `codex exec`
end to end without calling a model (`run.py run|judge --fake-provider`). Stdlib only.

Each request is answered from the conversation so far, so concurrent sessions need no state:

- no tool output yet: one `exec_command` call (`ls`);
- one tool output and an MCP namespace (`mcp__<server>`) offered: that server's `graph_stats`;
- otherwise the final message: for a request with a JSON schema (`--output-schema`), a fixed
  judge verdict; else `FAKE ANSWER`.

Codex is pointed at it with `-c model_provider=...` flags (see `codex_flags`); its usage numbers
are fixed so token accounting can be checked.
"""

from __future__ import annotations

import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

FAKE_MODEL = "fake-model"
FAKE_ANSWER = "FAKE ANSWER"
FAKE_VERDICT = {"correct": False, "score": 0.0, "reason": "fake provider"}
USAGE = {
    "input_tokens": 100,
    "input_tokens_details": {"cached_tokens": 40},
    "output_tokens": 5,
    "output_tokens_details": {"reasoning_tokens": 1},
    "total_tokens": 105,
}


def _reply(body: dict) -> dict:
    inputs = body.get("input") or []
    outputs = sum(1 for i in inputs if str(i.get("type", "")).endswith("_output"))
    namespaces = [
        t.get("name") for t in body.get("tools") or [] if t.get("type") == "namespace"
    ]
    mcp = [n for n in namespaces if n and n.startswith("mcp__")]
    tool_names = {t.get("name") for t in body.get("tools") or []}
    if outputs == 0 and "exec_command" in tool_names:
        return {
            "type": "function_call",
            "id": "fc_1",
            "call_id": "call_1",
            "name": "exec_command",
            "arguments": json.dumps({"cmd": "ls"}),
            "status": "completed",
        }
    if outputs == 1 and mcp:
        return {
            "type": "function_call",
            "id": "fc_2",
            "call_id": "call_2",
            "namespace": mcp[0],
            "name": "graph_stats",
            "arguments": "{}",
            "status": "completed",
        }
    fmt = ((body.get("text") or {}).get("format") or {}).get("type")
    text = json.dumps(FAKE_VERDICT) if fmt == "json_schema" else FAKE_ANSWER
    return {
        "type": "message",
        "id": "msg_1",
        "role": "assistant",
        "status": "completed",
        "content": [{"type": "output_text", "text": text, "annotations": []}],
    }


class _Handler(BaseHTTPRequestHandler):
    def log_message(self, format: str, *args) -> None:
        pass

    def do_GET(self) -> None:  # /v1/models: Codex falls back to built-in metadata
        self.send_response(404)
        self.end_headers()

    def do_POST(self) -> None:
        n = int(self.headers.get("content-length", 0))
        body = json.loads(self.rfile.read(n) or b"{}")
        item = _reply(body)
        rid = "resp_fake"
        events = [
            {"type": "response.created", "response": {"id": rid}},
            {"type": "response.output_item.done", "item": item},
            {"type": "response.completed", "response": {"id": rid, "usage": USAGE}},
        ]
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        for e in events:
            self.wfile.write(f"event: {e['type']}\ndata: {json.dumps(e)}\n\n".encode())
        self.wfile.flush()


class FakeProvider:
    def __init__(self) -> None:
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def codex_flags(self) -> list[str]:
        return [
            "-c", 'model_provider="benchfake"',
            "-c", 'model_providers.benchfake.name="benchfake"',
            "-c", f'model_providers.benchfake.base_url="http://127.0.0.1:{self.port}/v1"',
            "-c", 'model_providers.benchfake.wire_api="responses"',
            "-c", "model_providers.benchfake.supports_websockets=false",
            "-c", "model_providers.benchfake.request_max_retries=0",
            "-c", "model_providers.benchfake.stream_max_retries=0",
        ]  # fmt: skip

    def stop(self) -> None:
        self.server.shutdown()
