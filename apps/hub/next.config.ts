import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  poweredByHeader: false,
  /* The hub is a static index (see AGENTS.md), so it builds to plain files in `out/` and is served
   * from Cloudflare Pages rather than a Node process. */
  output: "export",
};

export default nextConfig;
