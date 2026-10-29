import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { isSandboxMissing } from "../src/gateway.ts";

describe("isSandboxMissing", () => {
  it("recognises the SDK's 404 for a sandbox that is gone", () => {
    // `handleResponse` in @codesandbox/sdk: `${operation}: Sandbox not found`.
    expect(isSandboxMissing(new Error("Failed to start VM sb-123: Sandbox not found"))).toBe(true);
    expect(isSandboxMissing(new Error("Failed to start VM, Error: Failed to start VM sb-1: sandbox not found"))).toBe(true);
  });

  it("does not mistake a transient or permission failure for a deleted sandbox", () => {
    // Replacing the sandbox on these would strand the old one's checkpoint and unpushed edits.
    for (const message of [
      "Failed to start VM sb-123: Unauthorized",
      "Failed to start VM sb-123: Bad gateway",
      "Failed to start VM sb-123: Rate limit exceeded",
      "fetch failed",
      "ETIMEDOUT",
    ]) {
      expect(isSandboxMissing(new Error(message)), message).toBe(false);
    }
    expect(isSandboxMissing("boom")).toBe(false);
    expect(isSandboxMissing(undefined)).toBe(false);
  });
});

describe("scripts/build-template.mjs", () => {
  const script = join(dirname(fileURLToPath(import.meta.url)), "..", "scripts", "build-template.mjs");
  const run = (args: string[]) =>
    spawnSync(process.execPath, [script, ...args], {
      encoding: "utf8",
      env: { ...process.env, CSB_API_KEY: "csb_v1_exampleexampleexample12345678" },
    });

  it("refuses an alias that is not a plain template alias, before anything is started", () => {
    // On Windows the CLI is started through a shell, so metacharacters must never get that far.
    for (const alias of ["x & calc", "a;b", "$(id)", "`id`", "a b", "../x", "-rf"]) {
      const result = run(["--alias", alias]);
      expect(result.status, `alias ${JSON.stringify(alias)}`).toBe(2);
      expect(result.stderr).toMatch(/Invalid --alias|CSB_API_KEY/);
    }
  });

  it("refuses an unknown VM tier", () => {
    const result = run(["--vm-tier", "Gigantic"]);
    expect(result.status).toBe(2);
    expect(result.stderr).toMatch(/Unknown VM tier/);
  });
});
