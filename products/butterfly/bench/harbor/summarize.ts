// Summarize Harbor jobs into the metrics the Butterfly program tracks:
// pass rate, cost/task, cost/successful task, tokens/task, cached-token ratio,
// runtime, tool calls/task, retries/task, attempts/task.
//
//   bun summarize.ts [jobs-dir] [--json out.json] [--nimble-db nimble.db]
//
// `--nimble-db` prices Nimble-routed trials from the router's own request records (the
// client cannot price a routed model; Butterfly reports no cost for it).
//
// Reads every <job>/<trial>/result.json Harbor wrote, joins Butterfly's own
// `agent/butterfly.json` report when present (tool calls, retries, attempts,
// verification, price_verified), and groups by (agent, model). Costs from
// unverified price cards are reported separately and never mixed in.

import { Database } from "bun:sqlite";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

interface Trial {
  job: string;
  task: string;
  agent: string;
  model: string;
  reward: number | null;
  errored: boolean;
  runtime_s: number | null;
  input_tokens: number | null;
  cache_tokens: number | null;
  output_tokens: number | null;
  cost_usd: number | null;
  price_verified: boolean | null;
  tool_calls: number | null;
  retries: number | null;
  attempts: number | null;
  verified: boolean | null;
  provider_calls: number | null;
  session_id: string | null;
  provider: string | null;
}

function* trialDirs(root: string): Generator<string> {
  if (!existsSync(root)) return;
  for (const job of readdirSync(root)) {
    const jobDir = join(root, job);
    if (!statSync(jobDir).isDirectory()) continue;
    for (const trial of readdirSync(jobDir)) {
      const dir = join(jobDir, trial);
      if (statSync(dir).isDirectory() && existsSync(join(dir, "result.json"))) yield dir;
    }
  }
}

function num(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function rewardOf(result: any): number | null {
  const rewards = result?.verifier_result?.rewards;
  if (!rewards || typeof rewards !== "object") return null;
  if (typeof rewards.reward === "number") return rewards.reward;
  const values = Object.values(rewards).filter((v) => typeof v === "number") as number[];
  return values.length ? values[0] : null;
}

function seconds(timing: any): number | null {
  if (!timing?.started_at || !timing?.finished_at) return null;
  return (new Date(timing.finished_at).getTime() - new Date(timing.started_at).getTime()) / 1000;
}

function lastJsonObject(text: string): any | undefined {
  const trimmed = text.trim();
  try {
    return JSON.parse(trimmed);
  } catch {
    /* fall through */
  }
  for (const line of trimmed.split("\n").reverse()) {
    try {
      return JSON.parse(line);
    } catch {
      /* keep scanning */
    }
  }
  return undefined;
}

function readTrial(dir: string): Trial {
  const result = JSON.parse(readFileSync(join(dir, "result.json"), "utf8"));
  const agentResult = result.agent_result ?? {};
  const trial: Trial = {
    job: dir.split("/").at(-2) ?? "",
    task: result.task_name ?? dir.split("/").at(-1) ?? "",
    agent: result.agent_info?.name ?? "unknown",
    model: result.agent_info?.model_info
      ? `${result.agent_info.model_info.provider ?? ""}/${result.agent_info.model_info.name}`
      : "unknown",
    reward: rewardOf(result),
    errored: Boolean(result.exception_info),
    runtime_s: seconds(result.agent_execution),
    input_tokens: num(agentResult.n_input_tokens),
    cache_tokens: num(agentResult.n_cache_tokens),
    output_tokens: num(agentResult.n_output_tokens),
    cost_usd: num(agentResult.cost_usd),
    price_verified: null,
    tool_calls: null,
    retries: null,
    attempts: null,
    verified: null,
    provider_calls: null,
    session_id: null,
    provider: null,
  };
  const reportPath = join(dir, "agent", "butterfly.json");
  if (existsSync(reportPath)) {
    const report = lastJsonObject(readFileSync(reportPath, "utf8"));
    if (report) {
      trial.tool_calls = num(report.tool_calls);
      trial.retries = num(report.retries);
      trial.attempts = Array.isArray(report.attempts) ? report.attempts.length : null;
      trial.verified = typeof report.verified === "boolean" ? report.verified : null;
      trial.provider_calls = num(report.provider_calls);
      trial.session_id = typeof report.session_id === "string" ? report.session_id : null;
      trial.provider = typeof report.provider === "string" ? report.provider : null;
      trial.price_verified = typeof report.price_verified === "boolean" ? report.price_verified : null;
      trial.input_tokens ??= num(report.input_tokens);
      trial.cache_tokens ??= num(report.cache_hit_input_tokens);
      trial.output_tokens ??= num(report.output_tokens);
      if (trial.cost_usd == null && typeof report.actual_cost_microusd === "number") {
        trial.cost_usd = report.actual_cost_microusd / 1_000_000;
      }
    }
  }
  return trial;
}

interface Group {
  job: string;
  agent: string;
  model: string;
  trials: number;
  errors: number;
  passed: number;
  cost_usd: number;
  costed: number;
  cost_unverified: boolean;
  input_tokens: number;
  cache_tokens: number;
  output_tokens: number;
  tokened: number;
  runtime_s: number;
  timed: number;
  tool_calls: number;
  tooled: number;
  retries: number;
  attempts: number;
  attempted: number;
}

const args = process.argv.slice(2);
const jobsDir = args.find((a) => !a.startsWith("--")) ?? "jobs";
const jsonOut = args.includes("--json") ? args[args.indexOf("--json") + 1] : undefined;
const nimbleDb = args.includes("--nimble-db") ? args[args.indexOf("--nimble-db") + 1] : undefined;
const nimble = nimbleDb && existsSync(nimbleDb) ? new Database(nimbleDb) : null;

/** Nimble priced every routed request itself; sum them for the trial's session. */
function nimbleCost(sessionId: string): { cost_usd: number; verified: boolean } | null {
  if (!nimble) return null;
  const row = nimble
    .query<{ cost: number | null; unverified: number }, [string]>(
      "SELECT SUM(cost_microusd) AS cost, SUM(CASE WHEN price_verified = 0 THEN 1 ELSE 0 END) AS unverified FROM requests WHERE session_id = ? AND status = 200",
    )
    .get(sessionId);
  if (!row || row.cost == null) return null;
  return { cost_usd: row.cost / 1_000_000, verified: row.unverified === 0 };
}

const trials = [...trialDirs(jobsDir)].map(readTrial);
for (const t of trials) {
  if (t.provider === "nimble" && t.session_id) {
    const priced = nimbleCost(t.session_id);
    if (priced) {
      t.cost_usd = priced.cost_usd;
      t.price_verified = priced.verified;
    }
  }
}
const groups = new Map<string, Group>();
for (const t of trials) {
  const key = `${t.job} ${t.agent} ${t.model}`; // one row per Harbor job: never average two harness versions
  const g =
    groups.get(key) ??
    {
      job: t.job,
      agent: t.agent,
      model: t.model,
      trials: 0,
      errors: 0,
      passed: 0,
      cost_usd: 0,
      costed: 0,
      cost_unverified: false,
      input_tokens: 0,
      cache_tokens: 0,
      output_tokens: 0,
      tokened: 0,
      runtime_s: 0,
      timed: 0,
      tool_calls: 0,
      tooled: 0,
      retries: 0,
      attempts: 0,
      attempted: 0,
    };
  g.trials += 1;
  if (t.errored) g.errors += 1;
  if ((t.reward ?? 0) >= 1) g.passed += 1;
  if (t.cost_usd != null) {
    g.cost_usd += t.cost_usd;
    g.costed += 1;
    if (t.price_verified === false) g.cost_unverified = true;
  }
  if (t.input_tokens != null) {
    g.input_tokens += t.input_tokens;
    g.cache_tokens += t.cache_tokens ?? 0;
    g.output_tokens += t.output_tokens ?? 0;
    g.tokened += 1;
  }
  if (t.runtime_s != null) {
    g.runtime_s += t.runtime_s;
    g.timed += 1;
  }
  if (t.tool_calls != null) {
    g.tool_calls += t.tool_calls;
    g.retries += t.retries ?? 0;
    g.tooled += 1;
  }
  if (t.attempts != null) {
    g.attempts += t.attempts;
    g.attempted += 1;
  }
  groups.set(key, g);
}

const fmt = (n: number | null, digits = 2) => (n == null || !Number.isFinite(n) ? "-" : n.toFixed(digits));
const rows = [...groups.values()].sort((a, b) => a.model.localeCompare(b.model) || a.agent.localeCompare(b.agent) || a.job.localeCompare(b.job));
console.log(
  "| job | agent | model | tasks | pass rate | cost/task | cost/success | tokens/task | cached | runtime | tools/task | retries/task | attempts |",
);
console.log("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
for (const g of rows) {
  const passRate = g.trials ? g.passed / g.trials : null;
  const costPerTask = g.costed ? g.cost_usd / g.costed : null;
  const costPerSuccess = g.passed && g.costed ? g.cost_usd / g.passed : null;
  const tokensPerTask = g.tokened ? (g.input_tokens + g.output_tokens) / g.tokened : null;
  const cached = g.input_tokens ? g.cache_tokens / g.input_tokens : null;
  const runtime = g.timed ? g.runtime_s / g.timed : null;
  const tools = g.tooled ? g.tool_calls / g.tooled : null;
  const retries = g.tooled ? g.retries / g.tooled : null;
  const attempts = g.attempted ? g.attempts / g.attempted : null;
  const unverified = g.cost_unverified ? "*" : "";
  console.log(
    `| ${g.job.replace(/__\d{8}T(\d{6})Z$/, "@$1")} | ${g.agent} | ${g.model} | ${g.trials}${g.errors ? ` (${g.errors} err)` : ""} | ${fmt(passRate == null ? null : passRate * 100, 1)}% | $${fmt(costPerTask, 4)}${unverified} | $${fmt(costPerSuccess, 4)}${unverified} | ${fmt(tokensPerTask, 0)} | ${fmt(cached == null ? null : cached * 100, 1)}% | ${fmt(runtime, 0)}s | ${fmt(tools, 1)} | ${fmt(retries, 2)} | ${fmt(attempts, 2)} |`,
  );
}
if (rows.some((g) => g.cost_unverified)) {
  console.log("\n* cost includes trials priced with an UNVERIFIED price card (placeholder rates); treat as indicative only.");
}
if (jsonOut) {
  writeFileSync(jsonOut, JSON.stringify({ generated_at: new Date().toISOString(), jobs_dir: jobsDir, groups: rows, trials }, null, 2));
  console.error(`wrote ${jsonOut}`);
}
