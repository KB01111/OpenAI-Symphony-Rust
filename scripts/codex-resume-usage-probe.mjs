#!/usr/bin/env node
/**
 * Opt-in probe against a REAL `codex app-server` binary (no model call, no authentication, no paid
 * usage): answers "what does thread/resume report as token usage?".
 *
 * It starts a thread in a throw-away CODEX_HOME, lets Codex write the rollout file (the first turn
 * fails with 401 because nothing is authenticated), appends a `token_count` event to that rollout the
 * way Codex records usage, then resumes the thread in a fresh app-server process and prints every
 * `thread/tokenUsage/updated` notification it receives.
 *
 * Verified with codex-cli 0.163.0 (see docs/PRODUCTION_GAP_AUDIT.md, finding P1-01): the resumed
 * thread replays its absolute total (`tokenUsage.total`) with the id of its previous turn, before any
 * new turn runs. Symphony's accounting (`symphony_core::usage::ThreadUsageLedger`) depends on that.
 *
 *   node scripts/codex-resume-usage-probe.mjs [path-to-codex]      (default: `codex` on PATH)
 *
 * Exit code 0 = the replay was observed with the expected totals, 1 = it was not, 2 = probe error.
 */
import { spawn } from "node:child_process";
import { appendFileSync, mkdtempSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const codex = process.argv[2] ?? "codex";
const home = mkdtempSync(join(tmpdir(), "symphony-codex-probe-"));
const TOTAL = { input_tokens: 1000, cached_input_tokens: 0, output_tokens: 500, reasoning_output_tokens: 0, total_tokens: 1500 };

/** The caller's environment without any credential: the probe must never authenticate, or its
 * "unauthenticated" turn would become a real, billable model call. */
function probeEnv() {
  const env = { ...process.env, CODEX_HOME: home };
  for (const key of Object.keys(env)) {
    if (/^(OPENAI|CODEX|ANTHROPIC|AZURE_OPENAI)_/i.test(key) && key !== "CODEX_HOME") delete env[key];
    if (/(_API_KEY|_TOKEN|_SECRET)$/i.test(key)) delete env[key];
  }
  return env;
}

function appServer() {
  const child = spawn(codex, ["app-server"], { env: probeEnv(), stdio: ["pipe", "pipe", "ignore"] });
  const messages = [];
  let buffer = "";
  child.stdout.on("data", (chunk) => {
    buffer += chunk;
    let nl;
    while ((nl = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, nl);
      buffer = buffer.slice(nl + 1);
      try {
        messages.push(JSON.parse(line));
      } catch {
        /* not JSON-RPC */
      }
    }
  });
  const send = (message) => child.stdin.write(JSON.stringify(message) + "\n");
  const request = async (id, method, params, waitMs = 3000) => {
    send({ method, id, params });
    const end = Date.now() + waitMs;
    while (Date.now() < end) {
      const hit = messages.find((m) => m.id === id);
      if (hit) return hit;
      await new Promise((r) => setTimeout(r, 50));
    }
    throw new Error(`no response to ${method}`);
  };
  return { child, messages, send, request };
}

function findRollout(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      const found = findRollout(path);
      if (found) return found;
    } else if (name.startsWith("rollout-") && name.endsWith(".jsonl")) return path;
  }
  return null;
}

const init = { clientInfo: { name: "symphony-probe", version: "0" }, capabilities: { experimentalApi: true } };
try {
  // 1. Create a rollout: thread/start + a turn that fails (unauthenticated), then stop the server.
  const first = appServer();
  await first.request(0, "initialize", init);
  first.send({ method: "initialized", params: {} });
  const started = await first.request(1, "thread/start", { cwd: tmpdir() });
  const threadId = started.result.thread.id;
  const turn = await first.request(2, "turn/start", { threadId, input: [{ type: "text", text: "hi" }] });
  const turnId = turn.result.turn.id;
  await new Promise((r) => setTimeout(r, 1500));
  first.child.kill();
  const rollout = findRollout(join(home, "sessions"));
  if (!rollout) throw new Error("Codex wrote no rollout file; cannot continue");

  // 2. Record usage the way Codex does.
  const stamp = new Date().toISOString();
  appendFileSync(rollout, JSON.stringify({ timestamp: stamp, ordinal: 1000, type: "event_msg", payload: { type: "token_count", info: { total_token_usage: TOTAL, last_token_usage: TOTAL, model_context_window: 200000 }, rate_limits: null } }) + "\n");
  appendFileSync(rollout, JSON.stringify({ timestamp: stamp, ordinal: 1001, type: "event_msg", payload: { type: "turn_complete", turn_id: turnId, last_agent_message: "done", completed_at: Math.floor(Date.now() / 1000), duration_ms: 1000 } }) + "\n");

  // 3. Resume in a fresh process and look at what is reported before any new turn.
  const second = appServer();
  await second.request(0, "initialize", init);
  second.send({ method: "initialized", params: {} });
  await second.request(1, "thread/resume", { threadId }, 8000);
  await new Promise((r) => setTimeout(r, 1500));
  second.child.kill();
  const reports = second.messages.filter((m) => m.method === "thread/tokenUsage/updated");
  console.log(`codex: ${codex}`);
  console.log(`resumed thread ${threadId} (previous turn ${turnId}) reported ${reports.length} tokenUsage notification(s):`);
  for (const r of reports) console.log(JSON.stringify(r.params));
  const replay = reports.find((r) => r.params.threadId === threadId && r.params.turnId === turnId && r.params.tokenUsage?.total?.totalTokens === TOTAL.total_tokens);
  if (replay) {
    console.log("OK: resume replays the thread's absolute total, tagged with the previous turn id.");
    process.exitCode = 0;
  } else {
    console.log("NOT OBSERVED: Symphony then treats an unknown baseline as 'estimated' (see ThreadUsageLedger).");
    process.exitCode = 1;
  }
} catch (error) {
  console.error(`probe failed: ${error.message}`);
  process.exitCode = 2;
} finally {
  rmSync(home, { recursive: true, force: true });
}
