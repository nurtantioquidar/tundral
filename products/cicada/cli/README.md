# Cicada

One fast command for the balances behind your model APIs. Cicada checks providers concurrently, so
checking all configured accounts takes about as long as checking the slowest one.

## Use

From the repository root:

```bash
bun run dev:cicada
cargo install --path products/cicada/cli
```

With an installed release binary:

```bash
cicada                 # all four providers
cicada all             # the same, explicitly
cicada deepseek        # one provider
cicada openai meta     # any subset
cicada --json          # machine-readable output
```

Provider aliases include `muse` for Meta and `glm` / `z-ai` for Z.ai.

## Credentials

Cicada reads credentials only from the environment. It never stores keys and intentionally does not
accept them on the command line.

```bash
export DEEPSEEK_API_KEY="..."
export ZAI_API_KEY="..."
export META_MODEL_API_KEY="..."  # MODEL_API_KEY and META_API_KEY also work
export OPENAI_ADMIN_KEY="..."
export OPENAI_MONTHLY_BUDGET_USD="100"
```

An ordinary `OPENAI_API_KEY` cannot read organization costs. OpenAI does not expose prepaid credit
remaining through its public API, so Cicada computes `monthly budget - month-to-date organization
costs`; it labels that number as computed. DeepSeek exposes a documented exact balance. For Z.ai,
Cicada checks the pay-as-you-go account wallet and GLM Coding Plan quota windows concurrently. Those Z.ai
monitoring routes and Meta's balance route are not in their public API specifications, so Cicada
reports authentication, endpoint, and response-shape failures instead of guessing.

Self-hosted or region-specific endpoints can be selected without changing code:

```bash
export DEEPSEEK_BASE_URL="https://api.deepseek.com"
export ZAI_BASE_URL="https://api.z.ai"
export META_BASE_URL="https://api.meta.ai/v1"
export OPENAI_BASE_URL="https://api.openai.com/v1"
```

`ZAI_BASE_URL` may also be the normal or Coding Plan API base; Cicada strips the known API suffix to
find the monitoring origin. Set `ZAI_MONITOR_BASE_URL` to override that origin, or set
`ZAI_BALANCE_URL` and `ZAI_QUOTA_URL` to complete URLs. `ZAI_CREDITS_URL` remains a compatibility
alias for `ZAI_BALANCE_URL`. For the other providers, use `DEEPSEEK_BALANCE_URL` or
`META_BALANCE_URL` to override the complete balance URL.

## Exit status

Cicada exits with status `0` only when every selected provider returns a balance or quota. Missing
configuration, unsupported provider behavior, authentication failures, and network failures are
shown per provider and produce status `1`.
