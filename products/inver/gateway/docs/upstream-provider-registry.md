# Upstream providers and model routing

## Ownership and frontend boundary

The inference gateway owns provider configuration, encrypted upstream credentials, model
capabilities, and routing. The operator flow is:

```text
Owner browser -> Inver web backend -> private gateway API -> gateway SQLite -> runtime snapshot
```

The browser never calls `/internal/*` and never receives the gateway admin token or an upstream
credential. The web backend authorizes each owner operation and invokes the narrow internal API.

Set `UPSTREAM_CONFIG_SOURCE=database` to make the registry authoritative. An empty or unroutable
registry intentionally makes generation unavailable; the gateway never falls back to environment
credentials. `UPSTREAM_MODE` selects the adapter only for `environment` mode. Database routes may
mix OpenAI-compatible HTTP providers and a local Codex subscription.

## Data model

Configuration is separated into four resources:

1. A **provider** identifies one vendor/transport and its base URL.
2. A **credential** is one write-only API key belonging to an HTTP provider. A provider may have
   many keys; each key has its own weight and concurrency limit.
3. A **model capability** maps a canonical model to the provider's model ID and supported API
   paths.
4. A **model route** is the published, revisioned priority list for one Inver public model.

For example, public `gpt-5.6-luna` has canonical model `gpt-5.6-luna`. A route may use any
enabled capability whose canonical model is
exactly `gpt-5.6-luna`. It cannot use a DeepSeek capability, even as a last resort.

Adding a new public model remains a separate product/rate-card change. Declaring an arbitrary
provider capability does not expose that model or invent a price.

## Priority behavior

Targets are evaluated from the lowest priority number to the highest:

- targets with the same priority form a weighted round-robin tier;
- a later tier is considered only when every eligible target in the earlier tier is saturated,
  cooling down, or fails retryably;
- request-validation failures do not fall through to another provider;
- a failed target enters a short process-local cooldown;
- once a streaming target emits text or a tool call, the gateway never changes providers mid-stream.

Provider, route-target, and credential concurrency limits all apply. Credential selection inside an
HTTP provider is also weighted and skips saturated/cooling keys.

Each request uses an immutable route snapshot. A publish or provider change swaps the snapshot for
new requests; in-flight work finishes on the old snapshot. Settled usage stores the route revision
and route-target ID internally for audit, but customer usage APIs do not expose pool topology.

## Credential encryption

Upstream API keys must be recoverable for outbound authentication, so they use AES-256-GCM rather
than hash-only storage. Configure a base64-encoded 32-byte deployment key:

```bash
openssl rand -base64 32
```

Store it as `UPSTREAM_CREDENTIAL_ENCRYPTION_KEY` in the deployment secret store and back it up
separately from SQLite. Losing it makes stored provider keys undecryptable. Read APIs return
`has_api_key` and a credential version, never plaintext or ciphertext. Keys are accepted only when
creating or rotating a credential.

Database-managed HTTP URLs require HTTPS and reject literal private-network destinations by
default. Local development may explicitly set both `ALLOW_INSECURE_UPSTREAM_HTTP=true` and
`ALLOW_PRIVATE_UPSTREAM_NETWORKS=true`. Production must also restrict egress; URL validation alone
does not prevent every DNS-rebinding case.

## Internal API

Every endpoint requires the gateway admin bearer token and must be blocked by the public reverse
proxy. Mutating APIs accept the last observed `revision`; a stale revision returns HTTP 409.

### 1. Providers

```http
GET /internal/upstreams
POST /internal/upstreams

{
  "name": "Vendor A",
  "transport": "openai_http",
  "base_url": "https://api.vendor-a.example/v1",
  "enabled": false,
  "max_concurrency": 8
}
```

For a locally authenticated Codex subscription:

```json
{
  "name": "Codex subscription",
  "transport": "codex_cli",
  "enabled": false,
  "max_concurrency": 2
}
```

`codex_cli` has no base URL or API-key records. Provider transport is immutable; replace the
provider to change its transport.

```http
GET /internal/upstreams/PROVIDER_ID
PATCH /internal/upstreams/PROVIDER_ID
DELETE /internal/upstreams/PROVIDER_ID?revision=2
```

Deleting a provider cascades to its credentials, capabilities, and route targets. Prefer disabling
when rollback may be needed.

### 2. API keys

```http
GET /internal/upstreams/PROVIDER_ID/credentials
POST /internal/upstreams/PROVIDER_ID/credentials

{
  "name": "primary",
  "api_key": "write-only-secret",
  "enabled": true,
  "weight": 2,
  "max_concurrency": 4,
  "models": ["gpt-5.6-luna", "gpt-5.5"]
}
```

`models` is an optional canonical-model allowlist for that key; `null` allows every capability on
the provider.

```http
PATCH /internal/upstream-credentials/CREDENTIAL_ID
POST /internal/upstream-credentials/CREDENTIAL_ID/rotate-key
DELETE /internal/upstream-credentials/CREDENTIAL_ID?revision=3
```

Rotation accepts `{"api_key":"replacement","revision":3}` and increments both the configuration
revision and credential version.

### 3. Supported models

```http
GET /internal/upstreams/PROVIDER_ID/models
POST /internal/upstreams/PROVIDER_ID/models

{
  "canonical_model_id": "gpt-5.6-luna",
  "upstream_model_id": "vendor-luna-2026-07",
  "api_paths": ["chat_completions", "responses"],
  "enabled": true
}
```

The canonical model is the semantic identity used for safe routing. The upstream model is the
vendor-specific request value. Canonical identity is immutable.

```http
GET /internal/upstreams/PROVIDER_ID/available-models
```

Read-only discovery for the operator console: calls the provider's own `/v1/models` with one
stored, decrypted credential and returns the reported model IDs. Requires an `openai_http`
provider with at least one enabled key; upstream failures map to `502` and never leak the key.

```http
PATCH /internal/upstream-models/CAPABILITY_ID
DELETE /internal/upstream-models/CAPABILITY_ID?revision=1
POST /internal/upstream-models/CAPABILITY_ID/test

{
  "public_model_id": "gpt-5.6-luna",
  "path": "/v1/chat/completions"
}
```

Testing works while a provider/capability is disabled. It performs a small real generation and may
consume provider credit. Success requires valid upstream usage because customer traffic cannot be
settled without it.

### 4. Publish model priorities

```http
GET /internal/model-routes
GET /internal/model-routes/gpt-5.6-luna
PUT /internal/model-routes/gpt-5.6-luna

{
  "enabled": true,
  "revision": 0,
  "targets": [
    {
      "capability_id": "CAP_VENDOR_A",
      "priority": 1,
      "weight": 2,
      "max_concurrency": 8,
      "enabled": true
    },
    {
      "capability_id": "CAP_CODEX",
      "priority": 2,
      "weight": 1,
      "max_concurrency": 2,
      "enabled": true
    }
  ]
}
```

`PUT` publishes the full target set atomically; omitted targets are removed. The first publish uses
revision `0`. The gateway validates the complete proposed snapshot before committing. Only
rate-card models can be published, and every target must have the public model's canonical model.

`GET /v1/models` and `GET /v1/pricing` expose only rate-card models that currently have at least
one eligible published target in database mode.

## Migration and rollback

Existing one-row-per-key provider records are migrated automatically:

- each embedded key becomes a `primary` credential;
- the old model allowlist becomes canonical capabilities;
- when no routes exist, matching rate-card routes are initialized as one equal-priority tier.

For moving from environment configuration:

1. Generate and securely back up `UPSTREAM_CREDENTIAL_ENCRYPTION_KEY`.
2. Keep `UPSTREAM_CONFIG_SOURCE=environment` while staging providers, capabilities, keys, and
   routes through the operator UI.
3. Test every capability.
4. Publish the intended routes and enable their providers.
5. Set `UPSTREAM_CONFIG_SOURCE=database` and restart the single gateway process.
6. Verify `/ready`, `/v1/models`, one billed request, its stored usage, the exact balance change,
   and reconciliation.
7. Remove obsolete environment keys after the rollback window.

Rollback by restoring `UPSTREAM_CONFIG_SOURCE=environment` and restarting. Do not delete the
registry or encryption key during the rollback window.
