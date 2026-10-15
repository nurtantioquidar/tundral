# Semantic pass bench

How to run Graphify's semantic (model) pass without a paid API key, record what it sends in a
clean-room form, and replay the recorded replies to Graphify or, later, to dijkstral. The behaviour
these runs revealed is specified in `../../cli/docs/graphify-semantic.md`; the goldens live in
`../../cli/tests/fixtures/semantic/`.

## Pieces

| File | Purpose |
|---|---|
| `../llm_shim.py` | Localhost OpenAI-compatible server (`/v1/models`, `/v1/chat/completions`, plain and SSE). Responders: `codex` (forwards each request to `codex exec`), `replay` (answers from a transcript directory), `static` (fixed or scripted replies, for free probes). Stdlib only; `--help` lists every flag |
| `make_binaries.py` | Regenerates the fixture's PDF, PNG (via macOS `sips`), DOCX, XLSX and an optional WAV with fixed bytes |
| `mixed.markers.txt` | Strings from the binary fixtures (PDF text lines, office conversion headers) the shim locates inside requests |

## Graphify with the semantic extras

The benchmark install (`../setup.sh`) has no `openai`, `pdf`, `office` or `watch` extra. Use a
separate venv so the timing baseline stays untouched:

```bash
uv venv --python 3.13 ~/.cache/dijkstral-bench/tools/graphify-0.9.86-semantic
uv pip install --python ~/.cache/dijkstral-bench/tools/graphify-0.9.86-semantic/bin/python \
  "graphifyy[leiden,mcp,openai,pdf,office]==0.9.86" watchdog
```

Always run Graphify with a scrubbed environment so it cannot find a real provider key (it picks a
backend from whichever key is set):

```bash
G=~/.cache/dijkstral-bench/tools/graphify-0.9.86-semantic/bin/graphify
env -i HOME=/tmp/graphify-home PATH=/usr/bin:/bin LANG=en_US.UTF-8 GRAPHIFY_QUERY_LOG_DISABLE=1 \
  OPENAI_BASE_URL=http://127.0.0.1:8765/v1 OPENAI_API_KEY=dummy OPENAI_MODEL=codex-shim \
  "$G" extract /tmp/scratch/mixed --backend openai
```

Work on a scratch copy of a fixture (`cp -R ../../cli/tests/fixtures/semantic/mixed /tmp/scratch/`);
Graphify writes `graphify-out/` (graph, cache, converted office files) into the corpus.

## Through Codex (subscription, no API key)

```bash
cd products/dijkstral
uv run --no-project bench/llm_shim.py --port 8765 --responder codex \
  --model gpt-6.1-sol --reasoning-effort medium --max-requests 4 \
  --transcript-dir /tmp/run1 --corpus /tmp/scratch/mixed --markers bench/semantic/mixed.markers.txt
```

- Each request becomes one `codex exec --ephemeral --skip-git-repo-check --ignore-user-config
  --ignore-rules -s read-only -C <empty temp dir> -m <model> -c model_reasoning_effort=<effort>
  --json -o <last message file> -` run; images in the request are attached with `-i`. The model
  and effort are always passed explicitly, so a run never depends on `~/.codex/config.toml`.
- Codex sessions are subject to the ChatGPT plan's usage limits: keep corpora tiny and set
  `--max-requests` (later requests get HTTP 429). A one-chunk extract of `mixed/` costs one
  request (about 17k input tokens, 3k output, 60–70 s); `label` costs one more.
- Reported token counts are Codex's: they include roughly 12k tokens of Codex's own instructions,
  so Graphify's `tokens:` line and `input_tokens` are higher than a direct API call would be.

## Transcripts (clean room)

`--transcript-dir` writes one `req-NNNN.json` per request: the responder, Codex model and
reasoning effort, timings, the request's structure and the full reply with its usage. Request text
is never stored. Each text part is recorded as `<prompt text: N chars>` with its line count,
hashes, the JSON keys and quoted identifiers it names, and a `layout`: in text order, where corpus
files (`--corpus`), their paths, the sha256 of a file and `--markers` strings occur, with the
framing between them reduced to character counts (`{"gap": 46}`). Response-format schemas are kept
with their `description` strings redacted. Check a transcript before committing it: no field other
than `response.text` may contain prose that came from Graphify.

## Replay

```bash
uv run --no-project bench/llm_shim.py --port 8766 --responder replay \
  --replay-dir cli/tests/fixtures/semantic/mixed.transcript --transcript-dir /tmp/replay
```

A request is answered with the recorded reply whose fingerprint (sha256 of its `messages` and
`response_format`) matches; unmatched requests take unused recorded replies in order. Graphify on
a fresh copy of `mixed/` then reproduces `mixed.extract.graphify.json` (default),
`mixed.extract_raw.graphify.json` (`--no-cluster`), `mixed.deep.graphify.json` (`--mode deep`) and,
after an extract, the `label` goldens byte for byte, with no Codex call.

dijkstral's prompts are its own, so its requests will not match Graphify's fingerprints. To test
dijkstral's merge logic against Graphify's, feed it the recorded `response.text` of each
transcript (or the crafted `merge_probe*.reply.json`) as the model reply for the same corpus and
compare with the corresponding goldens; `--responder static --static-response @reply.json` serves
any one reply to any client.

## Free probes (static responder)

```bash
uv run --no-project bench/llm_shim.py --port 8767 --responder static \
  --static-response @/tmp/reply.json --static-delay 1.5 --transcript-dir /tmp/probe
```

- `--static-response @file.jsonl` serves one JSON-encoded string per line in request order.
- `--finish-reason length --static-output-tokens 16384` provokes Graphify's truncation bisection.
- `--fail-first N` answers the first N requests with HTTP 500 (client retries).
- `--static-delay` with the `t_start`/`t_end` fields shows concurrency (`--max-concurrency`).
