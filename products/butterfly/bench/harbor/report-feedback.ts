// Teach Nimble what the harness could not know: post each trial's verifier
// verdict as `POST /v1/feedback` for the Nimble session that served it.
//
//   NIMBLE_API_KEY=… bun report-feedback.ts jobs/<job-dir> [more job dirs…]
//   NIMBLE_BASE_URL defaults to http://127.0.0.1:8092/v1
//
// Butterfly already reported a null outcome (no oracle inside the container);
// Nimble upgrades that record once with this verdict and updates its per-model,
// per-task-type history. Re-running is safe: a settled record is a duplicate.
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const base = (process.env.NIMBLE_BASE_URL ?? "http://127.0.0.1:8092/v1").replace(/\/$/, "");
const key = process.env.NIMBLE_API_KEY ?? "";
const jobs = process.argv.slice(2);
if (jobs.length === 0) {
  console.error("usage: bun report-feedback.ts <job-dir>…");
  process.exit(2);
}
const taskTypes: Record<string, string> = JSON.parse(readFileSync(new URL("./task-types.json", import.meta.url), "utf8"));

function lastJson(text: string): any {
  try {
    return JSON.parse(text.trim());
  } catch {
    for (const line of text.trim().split("\n").reverse()) {
      try {
        return JSON.parse(line);
      } catch {
        /* keep scanning */
      }
    }
  }
  return null;
}

let posted = 0;
const outcomes: Record<string, number> = {};
for (const job of jobs) {
  for (const trial of readdirSync(job)) {
    const dir = join(job, trial);
    if (!statSync(dir).isDirectory() || !existsSync(join(dir, "result.json"))) continue;
    const result = JSON.parse(readFileSync(join(dir, "result.json"), "utf8"));
    if (result.exception_info) continue; // an errored trial is not a verdict on the model
    const reportPath = join(dir, "agent", "butterfly.json");
    if (!existsSync(reportPath)) continue;
    const report = lastJson(readFileSync(reportPath, "utf8"));
    if (!report || report.provider !== "nimble" || typeof report.session_id !== "string") continue;
    const reward = result?.verifier_result?.rewards?.reward;
    if (typeof reward !== "number") continue;
    const task = String(result.task_name ?? trial).split("/").at(-1)!.split("__")[0];
    const taskType = Object.entries(taskTypes).find(([k]) => k !== "_comment" && task.endsWith(k))?.[1] ?? null;
    const body = {
      version: "nimble/v1",
      session_id: report.session_id,
      request_id: report.routing?.request_id ?? null,
      task_type: taskType,
      task_success: reward >= 1,
      verification_result: reward >= 1 ? "passed" : "failed",
      attempt_count: Array.isArray(report.attempts) ? report.attempts.length : null,
    };
    let outcome: string;
    let json: any = {};
    try {
      const response = await fetch(`${base}/feedback`, {
        method: "POST",
        headers: { "content-type": "application/json", ...(key ? { authorization: `Bearer ${key}` } : {}) },
        body: JSON.stringify(body),
      });
      json = (await response.json().catch(() => ({}))) as any;
      outcome = json.outcome ?? `http_${response.status}`;
    } catch (error) {
      outcome = `unreachable (${(error as Error).message.split("\n")[0]})`;
    }
    outcomes[outcome] = (outcomes[outcome] ?? 0) + 1;
    posted += 1;
    console.log(`${task.padEnd(26)} ${taskType ?? "-"} reward=${reward} session=${report.session_id.slice(0, 8)} model=${json.joined_model ?? "?"} → ${outcome}`);
  }
}
console.log(`posted ${posted} verdicts:`, outcomes);
