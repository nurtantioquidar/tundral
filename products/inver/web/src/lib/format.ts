const usdFormatter = new Intl.NumberFormat("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const tokenFormatter = new Intl.NumberFormat("en-US");

export function usd(micros: number | bigint): string {
  const value = Number(micros) / 1_000_000;
  if (!Number.isFinite(value)) return "—";
  if (value !== 0 && Math.abs(value) < 0.01) return `$${value.toFixed(6)}`;
  return `$${usdFormatter.format(value)}`;
}

export function usdFromString(value: string | null | undefined): string {
  if (!value) return "—";
  const parsed = Number(value);
  return Number.isFinite(parsed) ? usd(Math.round(parsed * 1_000_000)) : "—";
}

/* Formats a rate-card decimal string (e.g. "1.000000", "0.000125") for display. Pure string work —
 * the value is never parsed into a float, so a small rate can't be rounded away. Keeps at least two
 * decimals and trims the redundant trailing zeros beyond them. */
export function rateUsd(value: string | null | undefined): string {
  if (!value) return "—";
  const match = /^(-?)(\d+)(?:\.(\d*))?$/.exec(value.trim());
  if (!match) return "—";
  const [, sign, whole, fraction = ""] = match;
  const trimmed = fraction.replace(/0+$/, "");
  return `${sign}$${whole}.${trimmed.length < 2 ? trimmed.padEnd(2, "0") : trimmed}`;
}

export function tokens(count: number): string {
  return tokenFormatter.format(count);
}

export function compactTokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(count >= 10_000_000 ? 0 : 1)}M`;
  if (count >= 1_000) return `${(count / 1_000).toFixed(count >= 10_000 ? 0 : 1)}k`;
  return tokenFormatter.format(count);
}

export function timestamp(ms: number): string {
  return new Date(ms).toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: "UTC",
  });
}

export function day(ms: number): string {
  return new Date(ms).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" });
}
