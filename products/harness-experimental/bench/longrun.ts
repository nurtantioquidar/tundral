// Long-session exercise: fill hx's REAL 128k window with genuine repository
// reading work until auto-compaction fires naturally, then probe continuity.
// This measures what the tiny-window live test cannot: summarizer narrative
// quality over ~100k tokens of real content, and cache-hit recovery after a
// full-scale compaction.
//
// Usage:  DEEPSEEK_API_KEY=... bun longrun.ts [repo-root]
// Writes results/longrun-<epoch>.json with every turn's metrics and the
// continuity probe answers.

import { mkdirSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const BENCH_DIR = import.meta.dir;
const HX_BIN = process.env.HX_BIN ?? join(BENCH_DIR, "../cli/target/debug/hx");
const ROOT = process.argv[2] ?? join(BENCH_DIR, "../../..");
const MAX_TURNS = 30;
const FILES_PER_TURN = 3;

// Candidate reading material: tracked-ish text files between 8KB and 80KB.
function candidateFiles(root: string): string[] {
  const out: string[] = [];
  const skip = new Set(["node_modules", ".git", "target", "dist", "results", "fonts"]);
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        if (!skip.has(entry.name)) walk(join(dir, entry.name));
        continue;
      }
      if (!/\.(ts|tsx|rs|md)$/.test(entry.name) || entry.name.endsWith(".lock")) continue;
      const path = join(dir, entry.name);
      const size = statSync(path).size;
      if (size >= 8_000 && size <= 80_000) out.push(relative(root, path));
    }
  };
  walk(root);
  return out.sort((a, b) => statSync(join(root, b)).size - statSync(join(root, a)).size);
}

async function hxTurn(prompt: string, sessionId?: string): Promise<Record<string, any>> {
  const argv = [
    HX_BIN,
    "--read-only",
    "--json",
    "--quiet-reasoning",
    "--max-cost-usd",
    "0.50",
    "--root",
    ROOT,
    "-p",
    prompt,
  ];
  if (sessionId) argv.push("--resume", sessionId);
  const proc = Bun.spawn(argv, { env: process.env, stdout: "pipe", stderr: "pipe" });
  const [stdout, stderr, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ]);
  if (code !== 0) throw new Error(`hx exited ${code}: ${stderr.trim().slice(-500)}`);
  return JSON.parse(stdout.trim().split("\n").at(-1)!);
}

if (!process.env.DEEPSEEK_API_KEY) {
  console.error("DEEPSEEK_API_KEY is required");
  process.exit(1);
}

const files = candidateFiles(ROOT);
console.error(`found ${files.length} candidate files under ${ROOT}`);

const turns: Record<string, any>[] = [];
const readFiles: string[] = [];
let sessionId: string | undefined;
let compacted = false;

for (let turn = 0; turn < MAX_TURNS; turn++) {
  const batch = files.slice(turn * FILES_PER_TURN, (turn + 1) * FILES_PER_TURN);
  if (batch.length === 0) break;
  const prompt =
    `Use the read_file tool to read each of these files completely (use offset/limit if truncated), ` +
    `then give a two-sentence summary of each:\n${batch.join("\n")}`;
  const report = await hxTurn(prompt, sessionId);
  sessionId = report.session_id;
  readFiles.push(...batch);
  turns.push(report);
  console.error(
    `turn ${turn + 1}: util=${report.context_utilization_percent}% hit=${report.cache_hit_percent}% ` +
      `in=${report.input_tokens} elisions=${report.elisions} compactions=${report.compactions} ` +
      `session=$${(report.session_cost_microusd / 1_000_000).toFixed(4)}`,
  );
  if (report.budget_exhausted) {
    console.error("cost cap reached before compaction");
    break;
  }
  if (report.compactions > 0 || report.elisions > 0) compacted = true;
  // Stop once we have compacted AND run at least one post-compaction turn.
  if (compacted && report.compactions === 0 && report.elisions === 0) break;
}

console.error("\nprobing continuity…");
const probeList = await hxTurn(
  "Without using any tools: list every file path you have read in this session, one per line.",
  sessionId,
);
const probeEarly = await hxTurn(
  "Without using any tools: what did the FIRST few files you read in this session cover? Two sentences.",
  sessionId,
);

const answer = (probeList.response as string) ?? "";
const recalled = readFiles.filter((path) => answer.includes(path.split("/").at(-1)!));
const summary = {
  session_id: sessionId,
  files_read: readFiles.length,
  turns: turns.length,
  total_elisions: turns.reduce((n, t) => n + t.elisions, 0),
  total_compactions: turns.reduce((n, t) => n + t.compactions, 0),
  recall_percent: Math.round((recalled.length / Math.max(readFiles.length, 1)) * 100),
  final_utilization_percent: probeEarly.context_utilization_percent,
  session_cost_usd: (probeEarly.session_cost_microusd as number) / 1_000_000,
  post_compaction_hit_percent: probeList.cache_hit_percent,
  probe_list_answer: answer,
  probe_early_answer: probeEarly.response,
  turn_metrics: turns,
};

mkdirSync(join(BENCH_DIR, "results"), { recursive: true });
const outPath = join(BENCH_DIR, "results", `longrun-${Date.now()}.json`);
await Bun.write(outPath, JSON.stringify(summary, null, 2));

console.log(
  `\nfiles read: ${summary.files_read} · elisions: ${summary.total_elisions} · summarize compactions: ${summary.total_compactions}`,
);
console.log(
  `recall after compaction: ${summary.recall_percent}% · post-compaction cache hit: ${summary.post_compaction_hit_percent}% · total cost: $${summary.session_cost_usd.toFixed(4)}`,
);
console.log(`details: ${outPath}`);
