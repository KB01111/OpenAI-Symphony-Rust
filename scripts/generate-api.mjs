#!/usr/bin/env node
/**
 * Regenerates every API contract artifact from the Rust source of truth:
 *   1. openapi/symphony-api.json                       (utoipa, crates/symphony-server)
 *   2. packages/shared-types/src/generated/stateMachine.ts (TaskStatus::allowed_transitions)
 *   3. packages/shared-types/src/generated/openapi/     (Hey API: types, SDK, fetch client)
 *
 * Usage: `pnpm generate:api` (works on Windows, macOS and Linux).
 * CI runs the same command and fails when the committed files differ. Cargo runs with `--locked`
 * so generation always uses the committed Cargo.lock (and the exactly pinned utoipa versions).
 */
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function run(command, args, env = {}) {
  console.log(`> ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    env: { ...process.env, ...env },
    shell: process.platform === "win32"
  });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run("cargo", ["test", "--locked", "-p", "symphony-server", "--test", "openapi_contract", "--test", "state_machine_contract"], {
  UPDATE_OPENAPI: "1",
  UPDATE_STATE_MACHINE: "1"
});
run("pnpm", ["--filter", "@symphony/shared-types", "generate"]);
