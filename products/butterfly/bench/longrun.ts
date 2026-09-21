// Long-session exercise: fill a 128k-token window (pinned via --window-tokens;
// deepseek-v4's real window is 1M, which would cost too much to fill) with
// genuine repository reading work until auto-compaction fires naturally, then
// probe continuity. This measures what the tiny-window live test cannot:
// summarizer narrative quality over ~100k tokens of real content, and
// cache-hit recovery after a full-scale compaction.
//
// Usage:  DEEPSEEK_API_KEY=... bun longrun.ts [repo-root]
// Writes results/longrun-<epoch>.json with every turn's metrics and the
// continuity probe answers.

import { mkdirSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const BENCH_DIR = import.meta.dir;
const BUTTERFLY_BIN = process.env.BUTTERFLY_BIN ?? join(BENCH_DIR, "../cli/target/debug/butterfly");
const ROOT = process.argv[2] ?? join(BENCH_DIR, "../../..");
// Defaults reproduce the original 128k exercise. The env overrides exist for
// controlled A/B runs: at a pinned 128k the output reserve (65,536) and the
// 40k protected tail leave elision almost no room, so measuring stage 1 needs
// the real window and a turn count fixed across both arms.
const MAX_TURNS = Number(process.env.LONGRUN_MAX_TURNS ?? 30);
const FILES_PER_TURN = Number(process.env.LONGRUN_FILES_PER_TURN ?? 3);
const WINDOW_TOKENS = process.env.LONGRUN_WINDOW_TOKENS ?? "131072";
const MAX_COST_USD = process.env.LONGRUN_MAX_COST_USD ?? "1.00";
// Fixed-turn mode: run exactly MAX_TURNS so both arms read the same files in
// the same order. The adaptive stop would end each arm at whatever turn its
// own elision happened to fire, making recall incomparable.
const NO_EARLY_STOP = process.env.LONGRUN_NO_EARLY_STOP === "1";
const TAG = process.env.LONGRUN_TAG ?? "";

// Candidate reading material: tracked-ish text files between 8KB and 80KB.
function candidateFiles(root: string): string[] {
  const out: string[] = [];
  // .claude holds agent worktrees — whole duplicate copies of the repository.
  // Without it a working-tree run reads the same file several times over and
  // the recall probe is scored against near-duplicate paths.
  const skip = new Set([
    "node_modules",
    ".git",
    ".claude",
    "target",
    "dist",
    "results",
    "fonts",
  ]);
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

async function butterflyTurn(prompt: string, sessionId?: string): Promise<Record<string, any>> {
  const argv = [
    BUTTERFLY_BIN,
    "--read-only",
    "--json",
    "--quiet-reasoning",
    "--max-cost-usd",
    MAX_COST_USD,
    "--window-tokens",
    WINDOW_TOKENS,
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
  if (code !== 0) throw new Error(`butterfly exited ${code}: ${stderr.trim().slice(-500)}`);
  return JSON.parse(stdout.trim().split("\n").at(-1)!);
}

if (!process.env.DEEPSEEK_API_KEY) {
  console.error("DEEPSEEK_API_KEY is required");
  process.exit(1);
}

const files = candidateFiles(ROOT);
console.error(
  `found ${files.length} candidate files under ${ROOT}; window=${WINDOW_TOKENS} ` +
    `turns=${MAX_TURNS}x${FILES_PER_TURN} cap=$${MAX_COST_USD}` +
    `${NO_EARLY_STOP ? " fixed-turns" : ""}${TAG ? ` tag=${TAG}` : ""}`,
);

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
  const report = await butterflyTurn(prompt, sessionId);
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
  if (!NO_EARLY_STOP && compacted && report.compactions === 0 && report.elisions === 0) break;
}

console.error("\nprobing continuity…");
const probeList = await butterflyTurn(
  "Without using any tools: list every file path you have read in this session, one per line.",
  sessionId,
);
const probeEarly = await butterflyTurn(
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
const outPath = join(
  BENCH_DIR,
  "results",
  `longrun-${TAG ? `${TAG}-` : ""}${Date.now()}.json`,
);
await Bun.write(outPath, JSON.stringify(summary, null, 2));

console.log(
  `\nfiles read: ${summary.files_read} · elisions: ${summary.total_elisions} · summarize compactions: ${summary.total_compactions}`,
);
console.log(
  `recall after compaction: ${summary.recall_percent}% · post-compaction cache hit: ${summary.post_compaction_hit_percent}% · total cost: $${summary.session_cost_usd.toFixed(4)}`,
);
console.log(`details: ${outPath}`);
