import path, { basename as base } from "path";
import * as fs from "node:fs";
import def from "./default.cjs";
export * from "./store.js";
export { helper as formatHelper } from "./store.js";

// WHY: labels are shown in a fixed-width column.
export function formatLabel(text, width = 12) {
  function pad(s) {
    return s.padEnd(width);
  }
  return pad(base(text));
}

export default function formatAll(items) {
  return items.map(formatLabel).join(",");
}

export const helper = (value) => value + fs.existsSync(path.sep) + def();
