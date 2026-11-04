import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { isSandboxMissing } from "../src/gateway.ts";
import { resolveCsbBin } from "../scripts/csb-bin.mjs";

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

  it("runs the CodeSandbox CLI through node, with no shell between the arguments and the CLI", () => {
    const bin = resolveCsbBin();
    expect(existsSync(bin)).toBe(true);
    // `build --help` needs no network. An argument with spaces (like the template name the script passes)
    // is one argument to the CLI, which is what a cmd.exe command line would have split.
    const result = spawnSync(process.execPath, [bin, "build", "--help"], { encoding: "utf8" });
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toMatch(/Build an efficient memory snapshot/);
    expect(result.stdout).toMatch(/--alias/);
  });

  it("hands the template name to the CLI as one argument, and passes its exit code on", () => {
    const dir = mkdtempSync(join(tmpdir(), "symphony-csb-"));
    try {
      const out = join(dir, "argv.json");
      const fake = join(dir, "fake-csb.mjs");
      // Records the arguments it was started with, then exits with the code the test asks for.
      writeFileSync(
        fake,
        [
          'import { writeFileSync } from "node:fs";',
          "writeFileSync(process.env.FAKE_ARGV_OUT, JSON.stringify(process.argv.slice(2)));",
          "process.exit(Number(process.env.FAKE_EXIT ?? 0));",
          "",
        ].join("\n"),
      );
      const env = { ...process.env, CSB_API_KEY: "csb_v1_exampleexampleexample12345678", SYMPHONY_CSB_BIN: fake, FAKE_ARGV_OUT: out };
      const ok = spawnSync(process.execPath, [script, "--alias", "my-agents", "--vm-tier", "Micro"], { encoding: "utf8", env });
      expect(ok.status, ok.stderr).toBe(0);
      const argv: string[] = JSON.parse(readFileSync(out, "utf8"));
      // A shell would have split these on the spaces.
      expect(argv[0]).toBe("build");
      expect(argv[1]).toMatch(/template$/);
      expect(argv.slice(2)).toEqual([
        "--name",
        "Symphony cloud agents",
        "--alias",
        "my-agents",
        "--vm-tier",
        "Micro",
        "--vm-build-tier",
        "Medium",
        "--ci",
      ]);

      const failed = spawnSync(process.execPath, [script], { encoding: "utf8", env: { ...env, FAKE_EXIT: "7" } });
      expect(failed.status).toBe(7);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it("refuses an unknown VM tier", () => {
    const result = run(["--vm-tier", "Gigantic"]);
    expect(result.status).toBe(2);
    expect(result.stderr).toMatch(/Unknown VM tier/);
  });
});
