import { describe, expect, test } from "bun:test";
import { renderMetrics } from "../src/http/metrics";

describe("operational metrics", () => {
  test("renders low-cardinality Prometheus gauges without customer data", () => {
    const output = renderMetrics({
      databaseReady: true,
      upstreamConfigured: false,
      generationEnabled: true,
      inflightRequests: 2,
      configurationSource: "database",
    });
    expect(output).toContain("inver_gateway_database_ready 1");
    expect(output).toContain("inver_gateway_upstream_configured 0");
    expect(output).toContain("inver_gateway_inflight_requests 2");
    expect(output).toContain('source="database"');
    expect(output).not.toContain("customer");
  });
});
