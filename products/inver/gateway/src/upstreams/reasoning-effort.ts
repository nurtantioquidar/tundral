/** Reasoning effort is a per-request customer choice, not part of a model's public identity. */
export const REASONING_EFFORTS = ["minimal", "low", "medium", "high"] as const;
export type ReasoningEffort = (typeof REASONING_EFFORTS)[number];

/**
 * Extracts the customer-requested reasoning effort from a generation request body.
 *
 * Chat Completions carries it as top-level `reasoning_effort`; the Responses API nests it as
 * `reasoning.effort`. Absent means "let the upstream default apply". Invalid values are rejected
 * here — before reservation — rather than surfacing as an opaque upstream error.
 */
export function requestedReasoningEffort(
  path: "/v1/chat/completions" | "/v1/responses",
  body: Record<string, unknown>,
): ReasoningEffort | undefined {
  let raw: unknown;
  if (path === "/v1/responses") {
    const reasoning = body.reasoning;
    if (reasoning === undefined || reasoning === null) return undefined;
    if (typeof reasoning !== "object" || Array.isArray(reasoning)) {
      throw new Error("reasoning must be an object");
    }
    raw = (reasoning as Record<string, unknown>).effort;
  } else {
    raw = body.reasoning_effort;
  }
  if (raw === undefined || raw === null) return undefined;
  if (typeof raw !== "string" || !(REASONING_EFFORTS as readonly string[]).includes(raw)) {
    throw new Error(`reasoning effort must be one of: ${REASONING_EFFORTS.join(", ")}`);
  }
  return raw as ReasoningEffort;
}
