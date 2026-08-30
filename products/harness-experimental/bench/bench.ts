// Head-to-head runner: identical fixture tasks through hx (DeepSeek) and the
// Codex-built harness (OpenAI/Anthropic/subscription). Different models serve
// each side, so completion and cost are NOT model-quality comparisons; the
// comparable shapes are cache-hit behavior, token volumes, and wall-clock.
//
// Usage:
//   DEEPSEEK_API_KEY=... bun bench.ts [--only hx|harness] [--task <name>]
//   HARNESS_BIN=/path/to/harness bun bench.ts   # enable the harness side
//
// Results: printed as a table and written to results/bench-<epoch>.json.

import { cpSync, mkdirSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BENCH_DIR = import.meta.dir;
const HX_BIN = process.env.HX_BIN ?? join(BENCH_DIR, "../cli/target/debug/hx");
const HARNESS_BIN = process.env.HARNESS_BIN ?? "";

const PROMPT =
  "This repository has failing tests. Run `bun test` to see them, then fix the " +
  "source code (never modify the tests) so every test passes. Re-run `bun test` " +
  "to confirm before you finish.";

interface RunResult {
  system: string;
  task: string;
  verified: boolean;
  wall_ms: number;
  input_tokens?: number;
  cached_input_tokens?: number;
  cache_hit_percent?: number;
  output_tokens?: number;
  cost_microusd?: number | null;
  normalized_cost_microusd?: number | null;
  model?: string;
  error?: string;
}

function checkout(task: string): string {
  const dir = mkdtempSync(join(tmpdir(), `bench-${task}-`));
  cpSync(join(BENCH_DIR, "fixtures", task), dir, { recursive: true });
  const git = (...args: string[]) =>
    Bun.spawnSync(["git", ...args], { cwd: dir, stdout: "ignore", stderr: "ignore" });
  git("init", "-q");
  git("add", "-A");
  git("-c", "user.email=bench@local", "-c", "user.name=bench", "-c", "commit.gpgsign=false", "commit", "-qm", "fixture");
  return dir;
}

async function verify(dir: string): Promise<boolean> {
  const proc = Bun.spawn(["bun", "test"], { cwd: dir, stdout: "ignore", stderr: "ignore" });
  return (await proc.exited) === 0;
}

async function runCli(argv: string[], cwd: string, env: Record<string, string | undefined>): Promise<{ stdout: string; stderr: string; code: number; wall_ms: number }> {
  const started = performance.now();
  const proc = Bun.spawn(argv, {
    cwd,
    env: { ...process.env, ...env },
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ]);
  return { stdout, stderr, code, wall_ms: Math.round(performance.now() - started) };
}

function lastJsonLine(stdout: string): Record<string, unknown> | undefined {
  const trimmed = stdout.trim();
  // Whole-output JSON (possibly pretty-printed).
  try {
    return JSON.parse(trimmed);
  } catch {
    /* fall through */
  }
  // Pretty-printed JSON after other output: parse from the last "\n{".
  const brace = trimmed.lastIndexOf("\n{");
  if (brace >= 0) {
    try {
      return JSON.parse(trimmed.slice(brace + 1));
    } catch {
      /* fall through */
    }
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

async function runHx(task: string): Promise<RunResult> {
  const dir = checkout(task);
  try {
    const { stdout, stderr, code, wall_ms } = await runCli(
      [HX_BIN, "--auto", "--json", "--no-save", "--quiet-reasoning", "--max-cost-usd", "0.10", "--root", dir, "-p", PROMPT],
      dir,
      {},
    );
    const report = lastJsonLine(stdout);
    if (code !== 0 || !report) {
      return { system: "hx", task, verified: false, wall_ms, error: stderr.trim().slice(-400) || `exit ${code}` };
    }
    return {
      system: "hx",
      task,
      verified: await verify(dir),
      wall_ms,
      input_tokens: report.input_tokens as number,
      cached_input_tokens: report.cache_hit_input_tokens as number,
      cache_hit_percent: report.cache_hit_percent as number,
      output_tokens: report.output_tokens as number,
      cost_microusd: report.actual_cost_microusd as number,
      normalized_cost_microusd: report.normalized_cache_miss_cost_microusd as number,
      model: report.model as string,
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

async function runHarness(task: string): Promise<RunResult> {
  const dir = checkout(task);
  try {
    const argv = [HARNESS_BIN, "--json", "--no-tui", "--cwd", dir, "-p", PROMPT];
    if (process.env.HARNESS_PROVIDER) argv.push("--provider", process.env.HARNESS_PROVIDER);
    if (process.env.HARNESS_MODEL) argv.push("--model", process.env.HARNESS_MODEL);
    const { stdout, stderr, code, wall_ms } = await runCli(argv, dir, {});
    const report = lastJsonLine(stdout);
    if (code !== 0 || !report) {
      return {
        system: "harness",
        task,
        verified: await verify(dir),
        wall_ms,
        error: `no JSON parsed (exit ${code}); stdout tail: ${stdout.trim().slice(-200)}; stderr tail: ${stderr.trim().slice(-200)}`,
      };
    }
    return {
      system: "harness",
      task,
      verified: await verify(dir),
      wall_ms,
      input_tokens: report.input_tokens as number,
      cached_input_tokens: report.cached_input_tokens as number,
      cache_hit_percent: report.cache_hit_percent as number,
      output_tokens: report.output_tokens as number,
      cost_microusd: (report.estimated_cost_microusd as number | null) ?? null,
      model: report.model as string,
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

async function runCodex(task: string): Promise<RunResult> {
  const dir = checkout(task);
  try {
    const argv = [
      "codex",
      "exec",
      "--json",
      "--ephemeral",
      "--skip-git-repo-check",
      "-s",
      "workspace-write",
      "-C",
      dir,
    ];
    if (process.env.CODEX_MODEL) argv.push("-m", process.env.CODEX_MODEL);
    argv.push(PROMPT);
    const { stdout, stderr, code, wall_ms } = await runCli(argv, dir, {});
    // JSONL events: accumulate any usage objects and capture the model name.
    let input = 0;
    let cached = 0;
    let output = 0;
    let model: string | undefined;
    let sawUsage = false;
    for (const line of stdout.split("\n")) {
      let event: any;
      try {
        event = JSON.parse(line);
      } catch {
        continue;
      }
      const usage = event?.usage ?? event?.msg?.info?.total_token_usage;
      if (usage && typeof usage.input_tokens === "number") {
        sawUsage = true;
        input += usage.input_tokens;
        cached += usage.cached_input_tokens ?? usage.cached_tokens ?? 0;
        output += usage.output_tokens ?? 0;
      }
      model ??= event?.model ?? event?.msg?.model ?? event?.thread?.model;
    }
    if (code !== 0) {
      return {
        system: "codex",
        task,
        verified: await verify(dir),
        wall_ms,
        error: `exit ${code}; stderr tail: ${stderr.trim().slice(-200)}`,
      };
    }
    return {
      system: "codex",
      task,
      verified: await verify(dir),
      wall_ms,
      input_tokens: sawUsage ? input : undefined,
      cached_input_tokens: sawUsage ? cached : undefined,
      cache_hit_percent: sawUsage && input > 0 ? Math.round((cached / input) * 1000) / 10 : undefined,
      output_tokens: sawUsage ? output : undefined,
      cost_microusd: null, // subscription: no per-call price
      model,
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

const args = process.argv.slice(2);
const only = args.includes("--only") ? args[args.indexOf("--only") + 1] : undefined;
const taskFilter = args.includes("--task") ? args[args.indexOf("--task") + 1] : undefined;
const tasks = readdirSync(join(BENCH_DIR, "fixtures")).filter(
  (task) => !taskFilter || task === taskFilter,
);

const results: RunResult[] = [];
for (const task of tasks) {
  if (only === undefined || only === "hx") {
    if (!process.env.DEEPSEEK_API_KEY) {
      console.error("DEEPSEEK_API_KEY missing: skipping hx side");
    } else {
      console.error(`running hx / ${task} …`);
      results.push(await runHx(task));
    }
  }
  if (only === undefined || only === "harness") {
    if (!HARNESS_BIN) {
      console.error("HARNESS_BIN missing: skipping harness side");
    } else {
      console.error(`running harness / ${task} …`);
      results.push(await runHarness(task));
    }
  }
  if (only === undefined || only === "codex") {
    console.error(`running codex / ${task} …`);
    results.push(await runCodex(task));
  }
}

const usd = (micro?: number | null) =>
  micro == null ? "n/a" : `$${(micro / 1_000_000).toFixed(6)}`;
console.log("\n| system | task | verified | wall | in tokens | cached | hit% | out | cost |");
console.log("|---|---|---|---|---|---|---|---|---|");
for (const r of results) {
  console.log(
    `| ${r.system} | ${r.task} | ${r.error ? "ERROR" : r.verified ? "✅" : "❌"} | ${(r.wall_ms / 1000).toFixed(1)}s | ${r.input_tokens ?? "-"} | ${r.cached_input_tokens ?? "-"} | ${r.cache_hit_percent ?? "-"} | ${r.output_tokens ?? "-"} | ${usd(r.cost_microusd)} |`,
  );
  if (r.error) console.log(`  error: ${r.error}`);
}

mkdirSync(join(BENCH_DIR, "results"), { recursive: true });
const outPath = join(BENCH_DIR, "results", `bench-${Date.now()}.json`);
await Bun.write(outPath, JSON.stringify(results, null, 2));
console.error(`\nresults written to ${outPath}`);
