/**
 * `abort.sh` against real process trees: the driver, the agent it starts, and the commands the agent
 * runs must all stop, not just the driver. Needs `pgrep` and a Unix `bash` (Linux, macOS), so it is
 * skipped on Windows, where the scripts only ever run inside the Linux sandbox.
 */
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

const abortScript = resolve(dirname(fileURLToPath(import.meta.url)), "..", "template", "drivers", "abort.sh");
const unix = process.platform !== "win32" && spawnSync("pgrep", ["-f", "no-such-process-name-for-symphony"]).error === undefined;

let dir: string;
beforeAll(() => {
  if (unix) dir = mkdtempSync(join(tmpdir(), "symphony-abort-"));
});
afterAll(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
});

const sleep = (ms: number) => new Promise((resolveSleep) => setTimeout(resolveSleep, ms));

function pids(pattern: string): string[] {
  return spawnSync("pgrep", ["-f", pattern], { encoding: "utf8" }).stdout.split("\n").filter(Boolean);
}

describe.skipIf(!unix)("abort.sh", () => {
  it("stops the driver, the agent it started and the commands the agent ran", async () => {
    // Unique sleep durations identify this test's processes among everything else on the machine.
    const id = String(30000 + Math.floor(Math.random() * 9000));
    const driver = join(dir, "symphony-run-agent-fake.sh");
    // The file name carries the driver's marker; below it: an agent, its command, and that command's command.
    writeFileSync(
      driver,
      [
        "#!/bin/bash",
        `sleep ${id}1 &`,
        `bash -c 'sleep ${id}2 & bash -c "sleep ${id}3 & wait" & wait' &`,
        "wait",
        "",
      ].join("\n"),
      { mode: 0o755 },
    );
    const child = spawn("bash", [driver], { detached: true, stdio: "ignore" });
    child.unref();

    const sleepers = () => pids(`sleep ${id}[123]`);
    for (let i = 0; i < 50 && sleepers().length < 3; i += 1) await sleep(100);
    expect(sleepers().length).toBeGreaterThanOrEqual(3);
    expect(pids("symphony-run-agent-fake")).not.toEqual([]);

    const result = spawnSync("bash", [abortScript], { encoding: "utf8", timeout: 30_000 });

    expect(result.status, result.stderr).toBe(0);
    // Nothing below the driver survived: signalling the driver alone would leave all of these running.
    expect(sleepers()).toEqual([]);
    expect(pids("symphony-run-agent-fake")).toEqual([]);
  });

  it("succeeds without doing anything when no run is active", () => {
    const result = spawnSync("bash", [abortScript], { encoding: "utf8", timeout: 30_000 });
    expect(result.status, result.stderr).toBe(0);
  });
});
