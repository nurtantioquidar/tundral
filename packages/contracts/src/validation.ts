export interface GatewayErrorEnvelope {
  error: {
    message: string;
    type?: string;
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseGatewayError(value: unknown): GatewayErrorEnvelope | null {
  if (!isRecord(value) || !isRecord(value.error) || typeof value.error.message !== "string") {
    return null;
  }
  const type = typeof value.error.type === "string" ? value.error.type : undefined;
  return {
    error: {
      message: value.error.message,
      ...(type ? { type } : {}),
    },
  };
}
