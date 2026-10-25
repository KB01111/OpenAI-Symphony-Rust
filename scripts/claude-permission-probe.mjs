#!/usr/bin/env node
/**
 * Opt-in probe against a REAL `claude` binary (no model call, no API key, no paid usage): checks the
 * host permission protocol Symphony's Claude Code driver relies on (`--permission-prompt-tool stdio`).
 *
 * A local stand-in for the Messages API answers the first request with a Bash tool use
 * (`echo probe > probe.txt`). The probe runs claude twice in a throw-away HOME and workspace, answers
 * the `can_use_tool` control request once with `allow` and once with `deny`, and checks that the file
 * exists only after `allow`.
 *
 *   node scripts/claude-permission-probe.mjs [path-to-claude]      (default: `claude` on PATH)
 *
 * Verified with claude 2.1.310 (docs/PRODUCTION_GAP_AUDIT.md, AG-04).
 * Exit code 0 = protocol as expected, 1 = it was not, 2 = probe error.
 */
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";

const claude = process.argv[2] ?? "claude";
const root = mkdtempSync(join(tmpdir(), "symphony-claude-probe-"));

function sse(res, events) {
  res.writeHead(200, { "content-type": "text/event-stream" });
  for (const [type, data] of events) res.write(`event: ${type}\ndata: ${JSON.stringify({ type, ...data })}\n\n`);
  res.end();
}

const usage = { input_tokens: 1, output_tokens: 1, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 };
const server = createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    let parsed = {};
    try { parsed = JSON.parse(body); } catch {}
    if (!req.url.startsWith("/v1/messages") || !parsed.stream) {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ id: "m", type: "message", role: "assistant", model: "probe", content: [{ type: "text", text: "ok" }], stop_reason: "end_turn", usage }));
      return;
    }
    const message = { id: "msg", type: "message", role: "assistant", model: parsed.model, content: [], stop_reason: null, stop_sequence: null, usage };
    const answered = JSON.stringify(parsed.messages ?? []).includes("tool_result");
    const block = answered
      ? [{ type: "text", text: "" }, { type: "text_delta", text: "Done." }, "end_turn"]
      : [{ type: "tool_use", id: "toolu_probe", name: "Bash", input: {} }, { type: "input_json_delta", partial_json: JSON.stringify({ command: "echo probe > probe.txt", description: "Write a probe file" }) }, "tool_use"];
    sse(res, [
      ["message_start", { message }],
      ["content_block_start", { index: 0, content_block: block[0] }],
      ["content_block_delta", { index: 0, delta: block[1] }],
      ["content_block_stop", { index: 0 }],
      ["message_delta", { delta: { stop_reason: block[2], stop_sequence: null }, usage: { output_tokens: 1 } }],
      ["message_stop", {}]
    ]);
  });
});

function run(port, behavior) {
  const dir = join(root, behavior);
  const home = join(dir, "home");
  const ws = join(dir, "ws");
  mkdirSync(home, { recursive: true });
  mkdirSync(ws, { recursive: true });
  return new Promise((resolve, reject) => {
    const child = spawn(claude, ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--permission-prompt-tool", "stdio", "--model", "claude-sonnet-5-5"], {
      cwd: ws,
      // No credential of the caller's: the stand-in server is the only "API".
      env: { PATH: process.env.PATH, HOME: home, CLAUDE_CONFIG_DIR: join(home, ".claude"), ANTHROPIC_API_KEY: "sk-ant-probe-not-a-key", ANTHROPIC_BASE_URL: `http://127.0.0.1:${port}`, DISABLE_TELEMETRY: "1", DISABLE_AUTOUPDATER: "1", CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1" },
      stdio: ["pipe", "pipe", "ignore"],
      // On Windows `claude` is usually an npm `.cmd` shim, which only a shell can start.
      shell: process.platform === "win32"
    });
    let asked = false;
    const send = (m) => child.stdin.write(JSON.stringify(m) + "\n");
    send({ type: "user", message: { role: "user", content: "write the probe file" } });
    createInterface({ input: child.stdout }).on("line", (line) => {
      let m;
      try { m = JSON.parse(line); } catch { return; }
      if (m.type === "control_request" && m.request?.subtype === "can_use_tool") {
        asked = true;
        const response = behavior === "allow" ? { behavior: "allow", updatedInput: m.request.input } : { behavior: "deny", message: "denied by probe" };
        send({ type: "control_response", response: { subtype: "success", request_id: m.request_id, response } });
      } else if (m.type === "result") child.stdin.end();
    });
    const timer = setTimeout(() => { child.kill(); reject(new Error(`${behavior}: claude did not finish`)); }, 60_000);
    child.on("error", reject);
    child.on("exit", () => { clearTimeout(timer); resolve({ asked, wrote: existsSync(join(ws, "probe.txt")) }); });
  });
}

let code = 0;
try {
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const port = server.address().port;
  const allow = await run(port, "allow");
  const deny = await run(port, "deny");
  console.log(JSON.stringify({ allow, deny }));
  code = allow.asked && allow.wrote && deny.asked && !deny.wrote ? 0 : 1;
} catch (e) {
  console.error(e.message);
  code = 2;
} finally {
  server.close();
  rmSync(root, { recursive: true, force: true });
}
process.exit(code);
