import { describe, expect, test } from "bun:test";
import { parseGatewayError } from "../src/validation";

describe("gateway error contract", () => {
  test("accepts the public error envelope", () => {
    expect(parseGatewayError({
      error: { message: "Insufficient balance", type: "billing_error" },
    })).toEqual({
      error: { message: "Insufficient balance", type: "billing_error" },
    });
  });

  test("rejects malformed responses", () => {
    expect(parseGatewayError({ error: "nope" })).toBeNull();
    expect(parseGatewayError(null)).toBeNull();
  });
});
