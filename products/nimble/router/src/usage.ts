// Usage normalization across OpenAI-compatible dialects. DeepSeek reports
// `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`; OpenAI-style servers and GLM report
// `prompt_tokens_details.cached_tokens`. Everything collapses to { input, cached, output }.

import type { Usage } from "./pricing";

export function normalizeUsage(raw: unknown): Usage | null {
  if (!raw || typeof raw !== "object") return null;
  const u = raw as Record<string, unknown>;
  const num = (v: unknown) => (typeof v === "number" && Number.isFinite(v) ? v : null);
  const input = num(u.prompt_tokens) ?? num(u.input_tokens);
  if (input === null) return null;
  const details = (u.prompt_tokens_details ?? u.input_tokens_details) as Record<string, unknown> | undefined;
  const cached = num(u.prompt_cache_hit_tokens) ?? (details ? num(details.cached_tokens) : null) ?? 0;
  const output = num(u.completion_tokens) ?? num(u.output_tokens) ?? 0;
  return { input_tokens: input, cached_tokens: Math.min(cached, input), output_tokens: output };
}

/** Rough token estimate for byte-sized planning when the client gave none: ~4 bytes/token. */
export function estimateTokens(text: string): number {
  return Math.ceil(Buffer.byteLength(text, "utf8") / 4);
}

/** Stable-prefix hash: the system message plus tool schemas, serialized deterministically. */
export function stablePrefixHash(messages: unknown[], tools: unknown): string {
  const first = messages.length > 0 ? messages[0] : null;
  const payload = JSON.stringify([first, tools ?? null]);
  const hasher = new Bun.CryptoHasher("sha256");
  hasher.update(payload);
  return `sha256:${hasher.digest("hex")}`;
}

export function splitTokenEstimate(messages: unknown[], tools: unknown): { stable: number; dynamic: number } {
  const first = messages.length > 0 ? messages[0] : null;
  const stable = estimateTokens(JSON.stringify([first, tools ?? null]));
  const dynamic = estimateTokens(JSON.stringify(messages.slice(1)));
  return { stable, dynamic };
}
