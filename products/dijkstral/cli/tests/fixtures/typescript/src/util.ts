const { join } = require("path");
const legacy = require("./legacy");

export function slugify(text: string): string {
  return join(text, helper(text)).replace(/\W+/g, "-");
}

function helper(value: string): string {
  return value.trim();
}

export const traced = function <F extends (...args: unknown[]) => unknown>(fn: F): F {
  const wrapper = (...args: unknown[]) => fn(...args);
  return wrapper as F;
};

module.exports.legacySlug = legacy.slug;
