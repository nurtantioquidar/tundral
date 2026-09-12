# Cocoon CLI Agent Guide

## Purpose

`cocoon` is a local credential firewall for AI agents. It lets an agent use an API without placing
the upstream bearer credential in the agent's files, environment, arguments, prompt, or output.
Cocoon obtains credentials from an existing vault, applies deterministic destination and request
policy, and attaches the credential only in its own outbound HTTP client.

The first vertical slice brokers OpenAI-compatible LLM requests, uses 1Password secret references,
and supervises a child process with disposable Cocoon credentials. The durable product boundary is
brokering and enforcement, not secret storage or model routing.

## Ownership boundaries

This module owns local agent supervision, secret-provider adapters, request authorization,
credential attachment, environment scrubbing, and metadata-only audit events. It does not own a
vault, synchronize secrets, select models, meter customer credits, or import another Monet product.
It may speak OpenAI-compatible HTTP to Nimble through the same optional boundary as any other
client, but it must not import Nimble code.

## Security invariants

- Never accept a real credential as a CLI argument or in Cocoon configuration.
- Configuration contains secret references only. The initial provider accepts only `op://`
  references and obtains their values with `op read`.
- Never place an upstream credential in the supervised child's files, environment, arguments,
  stdout, stderr, or an audit record.
- Replace, rather than forward, the child's `Authorization` header.
- Fix every upstream origin in trusted configuration. A request may choose neither scheme, host,
  port, nor provider URL.
- Policies are deterministic. Model and path checks happen before any upstream request.
- Never store request or response bodies. Diagnostics contain metadata only.
- Offline tests use fake loopback upstreams and synthetic credentials. They never call 1Password
  or a paid provider.
- Do not claim that the initial supervised mode is a complete hostile-process sandbox. Until the
  OS enforcement layer exists, direct vault access and unrestricted egress remain outside the
  proven boundary and must be documented explicitly.

## Commands

From this directory:

```bash
cargo run -- check --config cocoon.example.toml
cargo run -- daemon --config cocoon.example.toml
cargo run -- run --profile deepseek -- your-agent
cargo test --locked
cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings
```

From the repository root:

```bash
bun run dev:cocoon -- --help
bun run --filter @monet/cocoon-cli check
```

## Verification

Every change must pass the module `check` command. Tests must cover credential replacement,
authentication failure, destination/path/model denial, environment scrubbing, and plaintext
`.env` detection without printing secret values.
