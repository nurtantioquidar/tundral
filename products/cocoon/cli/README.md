# Cocoon CLI

Cocoon is an early local credential-firewall prototype for AI agents. It obtains an upstream API
key from a 1Password secret reference, keeps that key out of the supervised process, and attaches it
only after a request passes provider, path, and model policy.

The current executable proves the broker boundary. It is not yet a complete hostile-process
sandbox; read [the design and threat model](../COCOON_DESIGN.md) before relying on it.

## What the first slice protects

- Configuration cannot contain a literal upstream credential.
- The child receives a random session token, never the upstream key.
- The child cannot select or redirect the upstream origin.
- Cocoon replaces the child's Authorization header instead of forwarding it.
- Only configured request paths and models are accepted.
- Provider redirects are not followed.
- Parent environment variables with common secret names are removed from the child.
- Cocoon refuses to start when it finds plaintext secret-like values in workspace `.env*` files.

The `.env` scanner reports only file, line, and variable name. It never prints the value.

## Try it

Create a private user configuration from `cocoon.example.toml` and point `secret` at a real
1Password reference. The reference is not the secret and may safely appear in configuration.

```bash
cargo run -- check --config cocoon.example.toml --workspace .
```

Start Cocoon once. This is the only process that resolves 1Password references:

```bash
cargo run -- daemon --config /path/to/cocoon.toml
```

Then run any number of supervised agent sessions from another terminal:

```bash
cargo run -- run --profile deepseek --workspace /path/to/project -- your-agent
```

Cocoon sets the profile's base-URL and API-key environment variables for the child. The API-key
value is an ephemeral Cocoon session token. It is valid only against the daemon's loopback listener,
is scoped to the profile's provider, and is revoked when the child exits. The daemon stores only a
SHA-256 digest of each live session token.

If the 1Password CLI uses desktop integration, it may ask for authorization when the daemon starts.
Agent launches do not call `op`. A restricted service account can also make daemon startup
non-interactive; Cocoon removes its token from the supervised process environment.

## Verification

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The test suite is offline and uses synthetic credentials plus loopback fake providers.
