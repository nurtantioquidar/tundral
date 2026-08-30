/**
 * Parse a duration like "2h", "30m", "45s", or combined forms like "1h30m"
 * and "1h2m3s" into total seconds. Throws on anything else.
 */
export function parseDuration(text: string): number {
  const match = /^(\d+)([hms])$/.exec(text);
  if (!match) {
    throw new Error(`invalid duration: ${text}`);
  }
  const value = Number(match[1]);
  switch (match[2]) {
    case "h":
      return value * 3600;
    case "m":
      return value * 60;
    default:
      return value;
  }
}
