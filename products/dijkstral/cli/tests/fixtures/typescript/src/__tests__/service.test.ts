import { describe, expect, it } from "vitest";
import { UserService, run } from "../service";
import { User } from "../models";

describe("UserService", () => {
  it("registers", async () => {
    const service = await run("/tmp/alice");
    expect(service).toBeInstanceOf(UserService);
  });

  it("parses", () => {
    expect(User.parse(" bob ").name).toBe("bob");
  });
});

function makeUser(): User {
  return new User("fixture");
}
