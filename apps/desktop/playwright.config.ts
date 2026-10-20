/**
 * End-to-end tests run the real desktop UI (Vite) against a real Symphony runtime (`symphony-cli`)
 * with the deterministic simulated agent — no paid models. The runtime uses a throwaway data
 * directory and a fixture Git repository created here.
 *
 * Native Tauri behaviour (tray, window state, notifications, Explorer) is NOT covered by these
 * browser tests; it requires a Windows Tauri build.
 */
import { defineConfig, devices } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const exe = process.platform === "win32" ? "symphony-cli.exe" : "symphony-cli";
const cli = join(repoRoot, "target", "debug", exe);
const runtimePort = Number(process.env.SYMPHONY_E2E_RUNTIME_PORT ?? 47910);
const uiPort = Number(process.env.SYMPHONY_E2E_UI_PORT ?? 1430);

// Created once in the main process; workers inherit the environment.
if (!process.env.SYMPHONY_E2E_DIR) {
  const dir = mkdtempSync(join(tmpdir(), "symphony-e2e-"));
  const repo = join(dir, "fixture-repo");
  mkdirSync(repo);
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: repo });
  execFileSync("git", ["config", "user.email", "e2e@symphony.local"], { cwd: repo });
  execFileSync("git", ["config", "user.name", "Symphony E2E"], { cwd: repo });
  writeFileSync(join(repo, "README.md"), "# E2E fixture\n");
  execFileSync("git", ["add", "."], { cwd: repo });
  execFileSync("git", ["commit", "-q", "-m", "init"], { cwd: repo });
  process.env.SYMPHONY_E2E_DIR = dir;
  process.env.SYMPHONY_E2E_REPO = repo;
}
const dataDir = join(process.env.SYMPHONY_E2E_DIR!, "data");
if (!existsSync(cli)) {
  execFileSync("cargo", ["build", "-q", "-p", "symphony-cli"], { cwd: repoRoot, stdio: "inherit" });
}

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  reporter: [["list"]],
  outputDir: join(process.env.SYMPHONY_E2E_DIR!, "results"),
  use: {
    baseURL: `http://127.0.0.1:${uiPort}`,
    viewport: { width: 1440, height: 900 },
    trace: "retain-on-failure",
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE }
      : existsSync("/opt/pw-browsers/chromium")
        ? { executablePath: "/opt/pw-browsers/chromium" }
        : {}
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } } }],
  webServer: [
    {
      command: `"${cli}" --data-dir "${dataDir}" --port ${runtimePort}`,
      cwd: process.env.SYMPHONY_E2E_DIR,
      url: `http://127.0.0.1:${runtimePort}/api/v1/health`,
      env: { SYMPHONY_SIMULATED_STEP_MS: "250", RUST_LOG: "warn" },
      reuseExistingServer: false,
      timeout: 120_000
    },
    {
      command: `pnpm exec vite --port ${uiPort} --strictPort`,
      url: `http://127.0.0.1:${uiPort}`,
      env: { SYMPHONY_DATA_DIR: dataDir },
      reuseExistingServer: false,
      timeout: 120_000
    }
  ]
});
