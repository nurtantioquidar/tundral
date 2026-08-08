# Monet inference API

## Base URL and authentication

The public API is OpenAI-compatible and served below:

```text
https://<gateway-domain>/v1
```

Send a customer API key as a bearer token:

```http
Authorization: Bearer sk-cg-...
```

API keys are workload credentials, not browser-login passwords. A key is shown only when created.
Use a different key for every device or workload so it can be capped and revoked independently.

## Models and pricing

`GET /v1/models` returns models currently routable by the gateway. `GET /v1/pricing` returns the
active rate-card version and USD prices per one million tokens.

The initial public model IDs are:

- `gpt-5.6-luna`;
- `gpt-5.5`.

Reasoning effort is chosen per request — `reasoning_effort` on Chat Completions or
`reasoning.effort` on Responses, one of `minimal`, `low`, `medium`, or `high`. Omitting it uses the
upstream default, and every effort bills at the same rate.

Do not assume an unavailable model, context window, or provider identity that is not returned by
the API.

## Generation

### Chat Completions

```bash
curl https://<gateway-domain>/v1/chat/completions \
  -H "Authorization: Bearer $MONET_API_KEY" \
  -H "Content-Type: application/json" \
  -H "Idempotency-Key: stable-request-id" \
  -d '{
    "model": "gpt-5.6-luna",
    "messages": [{"role": "user", "content": "Explain this repository"}],
    "max_completion_tokens": 2048,
    "reasoning_effort": "medium"
  }'
```

### Responses

```bash
curl https://<gateway-domain>/v1/responses \
  -H "Authorization: Bearer $MONET_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5.6-luna",
    "input": "Review this function",
    "max_output_tokens": 1024,
    "reasoning": {"effort": "low"}
  }'
```

`stream: true` emits server-sent events. The gateway continues upstream execution and settlement if
the client disconnects. Customer-defined function tools are selected by the model and returned to
the caller; Monet never executes customer tools.

Codex-backed routes support one selected customer tool per HTTP turn. Do not send
`parallel_tool_calls: true`.

## Usage and balance

`GET /v1/usage` returns the customer’s available balance and recent settled usage. Usage includes:

- public model and immutable rate-card version;
- API-key attribution;
- input, cached-input, cache-write, and output tokens;
- settled cost and balance snapshots.

Balances and charges are computed by the gateway in integer micro-USD. Displayed decimal USD values
have six fractional digits.

## Idempotency

Generation accepts an optional `Idempotency-Key` header containing 1–200 URL-safe characters.

- The key is scoped to the customer API key and exact request bytes.
- A completed duplicate replays the original response without charging again.
- A concurrent duplicate returns `409`.
- Reusing a key with different request bytes returns `409`.
- A failed attempt may be retried with the same key.

Use a stable key for every logical request when a client may retry after a timeout.

## Errors

Errors use an OpenAI-style envelope:

```json
{
  "error": {
    "message": "Insufficient balance",
    "type": "billing_error"
  }
}
```

Important statuses:

| Status | Meaning |
|---:|---|
| `400` | Invalid request, unsupported model, or unsupported tool behavior |
| `401` | Missing, invalid, expired, revoked, or frozen customer key |
| `402` | Insufficient balance or a spending guardrail was reached |
| `409` | Idempotency conflict |
| `429` | Per-key rate/concurrency limit or gateway capacity reached |
| `500–503` | Gateway disabled, no route available, or upstream failure |

Respect `Retry-After` on `429`. Do not automatically retry `400`, `401`, or `402`.

## Compatibility boundary

Monet preserves standard upstream response fields and adds `gateway_usage` to successful generation
responses. Treat additional fields as additive. Pin behavior to the public model ID and returned
rate-card version, not to an assumed underlying provider.
