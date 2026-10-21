/**
 * Remote companion access through the generated client against a real runtime (`symphony-cli`):
 * token issue/rotation/revocation, the local/remote authorization boundary, and the mobile
 * reconnect path (`setConnection` with the new token).
 */
import { execFile, spawn, type ChildProcess } from "node:child_process";
import { promisify } from "node:util";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { SymphonyApiError, SymphonyClient } from "../../src/index.js";
import { readRuntimeDiscovery } from "../../src/node/index.js";

const repoRoot = resolve(__dirname, "../../../..");
const exe = process.platform === "win32" ? "symphony-cli.exe" : "symphony-cli";
const cliPath = process.env.SYMPHONY_CLI_BIN ?? join(repoRoot, "target", "debug", exe);

let tmp: string;
let runtime: ChildProcess;
let local: SymphonyClient;
let localToken = "";

async function waitFor<T>(fn: () => T | undefined | null | false | Promise<T | undefined | null | false>, label: string, timeoutMs = 30_000): Promise<T> {
  const start = Date.now();
  for (;;) {
    const value = await fn();
    if (value) return value;
    if (Date.now() - start > timeoutMs) throw new Error(`Timed out waiting for ${label}`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

function freePort(): Promise<number> {
  return new Promise((resolvePort, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : 0;
      server.close(() => resolvePort(port));
    });
  });
}

async function failure(promise: Promise<unknown>): Promise<SymphonyApiError> {
  return (await promise.then(
    () => {
      throw new Error("expected the request to fail");
    },
    (e: unknown) => e
  )) as SymphonyApiError;
}

beforeAll(async () => {
  if (!existsSync(cliPath)) {
    await promisify(execFile)("cargo", ["build", "-p", "symphony-cli"], { cwd: repoRoot, maxBuffer: 64 * 1024 * 1024 });
  }
  tmp = mkdtempSync(join(tmpdir(), "symphony-remote-it-"));
  const dataDir = join(tmp, "data");
  runtime = spawn(cliPath, ["--data-dir", dataDir, "--port", "0"], {
    cwd: tmp,
    env: { ...process.env, RUST_LOG: "warn" },
    stdio: ["ignore", "ignore", "inherit"]
  });
  const record = await waitFor(() => readRuntimeDiscovery(dataDir), "runtime.json");
  localToken = record.token;
  local = new SymphonyClient({ baseUrl: record.base_url, token: record.token, endpoint: { source: "configured", mode: "attached" } });
}, 240_000);

afterAll(async () => {
  if (runtime && runtime.exitCode === null && runtime.signalCode === null) {
    const exited = new Promise<void>((r) => runtime.once("exit", () => r()));
    runtime.kill();
    await Promise.race([exited, new Promise((r) => setTimeout(r, 10_000))]);
  }
  if (tmp) {
    try {
      rmSync(tmp, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    } catch {
      /* the OS cleans the temp dir */
    }
  }
});

describe("remote companion access (real runtime, generated client)", () => {
  let remoteBase = "";
  let firstToken = "";

  it("is disabled by default and reports the effective state", async () => {
    const status = await local.getRemoteAccess();
    expect(status.enabled).toBe(false);
    expect(status.state).toBe("disabled");
    expect(status.token_configured).toBe(false);
  });

  it("starts after enabling it and issuing a token, without a restart", async () => {
    const settings = await local.getSettings();
    const port = await freePort();
    await local.saveSettings({ ...settings, remote_api_enabled: true, remote_bind_address: "127.0.0.1", remote_port: port });
    expect((await local.getRemoteAccess()).state).toBe("token_missing");

    const issued = await local.rotateRemoteAccessToken();
    expect(issued.token).toMatch(/^[0-9a-f]{64}$/);
    expect(issued.status.state).toBe("running");
    expect(issued.status.listening_on).toBe(`127.0.0.1:${port}`);
    remoteBase = `http://127.0.0.1:${port}`;
    firstToken = issued.token;
    // The token is never readable afterwards.
    expect(JSON.stringify(await local.getRemoteAccess())).not.toContain(issued.token);
    expect(JSON.stringify(await local.listCredentials())).not.toContain(issued.token);
  });

  it("serves companion operations and forbids administration", async () => {
    const phone = new SymphonyClient({ baseUrl: remoteBase, token: firstToken });
    const state = await phone.getState();
    expect(state.scheduler_paused).toBe(false);
    expect(Array.isArray(await phone.listTasks())).toBe(true);
    expect(Array.isArray(await phone.listApprovals(true))).toBe(true);

    for (const call of [
      () => phone.listCredentials(),
      () => phone.saveCredential("GITHUB_TOKEN", "ghp_from_phone"),
      () => phone.getWorkflow(),
      () => phone.saveWorkflow("---\n---\n"),
      () => phone.getSettings(),
      () => phone.exportDiagnostics(),
      () => phone.rotateRemoteAccessToken()
    ]) {
      const err = await failure(call());
      expect(err.status).toBe(403);
      expect(err.code).toBe("forbidden_scope");
    }
    // Credentials are not interchangeable between the listeners.
    const localOnRemote = new SymphonyClient({ baseUrl: remoteBase, token: localToken });
    expect((await failure(localOnRemote.getState())).kind).toBe("unauthorized");
    const remoteOnLocal = new SymphonyClient({ baseUrl: local.connection.baseUrl, token: firstToken });
    expect((await failure(remoteOnLocal.getState())).kind).toBe("unauthorized");
  });

  it("rotation invalidates the old token and the companion reconnects with the new one", async () => {
    const phone = new SymphonyClient({ baseUrl: remoteBase, token: firstToken });
    const rotated = await local.rotateRemoteAccessToken();
    expect(rotated.token).not.toBe(firstToken);
    expect((await failure(phone.getState())).kind).toBe("unauthorized");
    phone.setConnection(remoteBase, rotated.token);
    expect((await phone.getState()).scheduler_paused).toBe(false);
  });

  it("revocation stops the listener", async () => {
    const status = await local.revokeRemoteAccessToken();
    expect(status.state).toBe("token_missing");
    expect(status.token_configured).toBe(false);
    const phone = new SymphonyClient({ baseUrl: remoteBase, token: firstToken, timeoutMs: 3_000 });
    expect((await failure(phone.health())).kind).toBe("network");
  });

  it("rejects unsafe remote settings", async () => {
    const settings = await local.getSettings();
    const exposed = await failure(local.saveSettings({ ...settings, remote_bind_address: "0.0.0.0" }));
    expect(exposed.status).toBe(400);
    const wildcard = await failure(local.saveSettings({ ...settings, remote_allowed_origins: ["*"] }));
    expect(wildcard.status).toBe(400);
  });
});
