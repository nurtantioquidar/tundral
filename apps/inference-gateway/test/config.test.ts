import { describe, expect, test } from "bun:test";
import { loadGatewayConfig } from "../src/config";

describe("gateway configuration", () => {
  test("loads validated defaults", () => {
    const config = loadGatewayConfig({});
    expect(config).toMatchObject({
      host: "127.0.0.1",
      port: 8080,
      upstreamMode: "openai",
      upstreamConfigSource: "environment",
      maxGlobalConcurrentRequests: 4,
    });
  });

  test("rejects invalid modes and token limits", () => {
    expect(() => loadGatewayConfig({ UPSTREAM_MODE: "invalid" })).toThrow();
    expect(() => loadGatewayConfig({
      DEFAULT_MAX_OUTPUT_TOKENS: "100",
      MAX_OUTPUT_TOKENS_PER_REQUEST: "50",
    })).toThrow();
  });

  test("requires distinct internal credentials when strict scoping is enabled", () => {
    expect(() => loadGatewayConfig({
      REQUIRE_SCOPED_INTERNAL_KEYS: "true",
      ADMIN_API_KEY: "same",
      WEB_BFF_API_KEY: "same",
    })).toThrow();
    expect(loadGatewayConfig({
      REQUIRE_SCOPED_INTERNAL_KEYS: "true",
      ADMIN_API_KEY: "admin-secret",
      WEB_BFF_API_KEY: "web-secret",
    })).toMatchObject({
      adminApiKey: "admin-secret",
      webBffApiKey: "web-secret",
      requireScopedInternalKeys: true,
    });
  });
});
