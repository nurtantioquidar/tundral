# Cicada CLI Agent Guide

## Purpose

`cicada` is a fast local Rust CLI that checks API-provider balances in one command. It queries
providers concurrently, reads credentials from environment variables, and never stores or prints
secrets. The initial providers are OpenAI, Meta Model API (Muse), Z.ai, and DeepSeek.

## Ownership boundaries

This module owns provider balance endpoints, response parsing, terminal and JSON presentation, and
the semantics of a balance check. It does not import another Monet application, read another
product's credentials, scrape browser sessions, estimate model usage, or become a billing source of
truth. Provider responses are read-only snapshots.

## Provider contract

- DeepSeek uses the documented `GET /user/balance` endpoint.
- Z.ai queries the pay-as-you-go account report and GLM Coding Plan quotas concurrently through its
  bearer-authenticated monitoring surface. Meta uses a bearer-authenticated `/balance` route.
  These routes are not in the providers' public API specifications, so failures must stay explicit;
  never fabricate a balance or quota from a health check or model request.
- OpenAI does not publish a prepaid-credit balance endpoint. Cicada uses the documented organization
  Costs API with an admin key and subtracts month-to-date spend from an operator-supplied monthly
  budget. The output must identify this as a computed budget remainder.
- Money is parsed and accumulated as integer micro-units. Floating-point arithmetic is forbidden.

## Security

- Read keys only from the documented environment variables.
- Never accept keys as CLI arguments, because arguments can enter shell history and process lists.
- Never log request headers, credentials, or raw provider responses.
- Error messages may include a short provider-supplied message but must not dump response bodies.

## Commands

Run from this directory:

```bash
cargo run --
cargo run -- deepseek
cargo test --locked
cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings
```

From the repository root:

```bash
bun run dev:cicada -- --help
bun run --filter @monet/cicada-cli check
```

## Verification

Every change must pass the module `check` command. Unit tests must remain offline and cover each
provider response shape plus integer decimal parsing. Live checks are manual and must never run in
the deterministic test suite.
