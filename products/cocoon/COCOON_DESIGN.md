# Cocoon design

## Product statement

Cocoon is a local credential firewall for AI agents. A developer connects an existing secret
manager once, defines durable policy, and then runs agents normally. Allowed operations complete
without an approval prompt; denied operations never receive credentialed network access; new
authority requires an explicit approval outside the agent's control.

Cocoon is not a password manager, encrypted dotenv implementation, model router, or general-purpose
forward proxy. Its security value comes from keeping durable credentials outside an untrusted agent
process and mediating every supported use of those credentials.

## Threat model

The protected asset is a durable credential such as a DeepSeek, Muse, or GLM API key. The initial
adversary is an agent that can read and edit its workspace, run shell commands, control request
bodies, and encounter prompt injection. Cocoon aims to prevent the agent from obtaining the durable
credential and to constrain where and how Cocoon uses it.

The first CLI slice proves a narrower boundary: credential replacement inside the broker,
destination/path/model policy, process-environment scrubbing, and plaintext `.env` detection. It
does not yet prevent a supervised process from using its ordinary filesystem or network access to
bypass Cocoon. Product-grade hostile-agent protection additionally requires OS-level filesystem
and egress enforcement. This limitation must remain visible until that layer exists.

Out of scope are a hostile administrator, compromise of the operating system or Cocoon process,
provider-side compromise, and prompt confidentiality from a provider the user intentionally calls.

## Trust boundaries

```text
Existing vault                    Cocoon                         Agent
──────────────          ┌────────────────────────┐     ┌────────────────────┐
durable secret ───────► │ secret-provider adapter│     │ workspace access   │
                        │ deterministic policy   │ ◄── │ disposable token   │
Provider API ◄───────── │ outbound HTTP client   │ ──► │ sanitized response │
                        └────────────────────────┘     └────────────────────┘
                                  trusted                    untrusted
```

Only Cocoon's outbound client holds the upstream Authorization value. The agent receives a random,
ephemeral session token. Cocoon authenticates that token, validates the provider, exact path, and
model, discards the supplied Authorization header, and creates a new upstream header.

## Initial user experience

The first executable interface is explicit while the security boundary is being validated:

```bash
cocoon check --config ~/.config/cocoon/config.toml --workspace .
cocoon daemon --config ~/.config/cocoon/config.toml
cocoon run --profile deepseek -- codex
```

The long-lived daemon resolves vault references at startup. Each `run` call requests a short-lived
session over a mode-`0600` local control socket and automatically configures the child process with
the ephemeral endpoint and token. It revokes the session when the child exits. Projects do not
contain upstream base URLs or durable credentials. A later agent adapter may install this
supervision once so the ordinary `codex` or `claude` command remains unchanged; adapter
installation must never weaken the core boundary.

The initial 1Password adapter resolves each configured credential when the daemon starts. Agent
launches never invoke `op`. Interactive desktop authorization is therefore confined to daemon
startup rather than repeated for every agent task. Unattended daemon startup can use a restricted
1Password service account held outside the agent environment.

## Configuration and policy

Cocoon configuration contains only public provider data and vault references:

```toml
version = 1

[providers.deepseek]
upstream = "https://api.deepseek.com"
secret = "op://AI Agents/DeepSeek/api-key"
allowed_paths = ["/v1/chat/completions"]
allowed_models = ["deepseek-chat", "deepseek-reasoner"]

[profiles.deepseek]
provider = "deepseek"
base_url_env = "OPENAI_BASE_URL"
api_key_env = "OPENAI_API_KEY"
```

Upstream origins always use TLS, redirects are not followed, and the agent cannot override the
scheme, host, port, or credential. Repository-local configuration may suggest requested
capabilities but must never grant them. Durable grants belong in user-controlled configuration or
must carry a user signature.

## Delivery stages

### Stage 1: verified broker core

- 1Password `op://` references only.
- OpenAI-compatible POST requests.
- Exact path and model allowlists.
- A long-lived loopback broker, mode-`0600` control socket, and revocable per-run session tokens.
- Plaintext `.env` detection and parent-environment scrubbing.
- Offline fake-upstream verification.

### Stage 2: hardened local authority

- Harden and sign the local daemon and authenticate configuration changes with the OS.
- Restricted service-account bootstrap stored in the OS credential store.
- Process-tree-bound session issuance over authenticated local IPC.
- Metadata-only, tamper-evident audit receipts.
- Installed Codex and Claude adapters with no per-project configuration.

### Stage 3: complete mediation

- Agent filesystem policy that makes secret sources unreachable.
- Default-deny egress for supervised processes.
- Broker adapters for GitHub, cloud, databases, and other typed capabilities.
- Delegation where subagents may receive only a subset of their parent's authority.

## Relationship to other Monet products

Nimble selects a model and provider. Cocoon decides whether an agent may exercise a credential and
keeps that credential outside the agent. Either works independently. Cocoon may call Nimble through
its public OpenAI-compatible contract, but neither product imports the other or shares runtime
state.
