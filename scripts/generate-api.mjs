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
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

/**
 * `@hey-api/openapi-ts` drives the TypeScript JavaScript compiler API, which TypeScript 7 (the
 * native compiler) no longer ships: under 7 the generator crashes with an unhelpful TypeError.
 * Fail before generating anything, with the reason, when the generator resolves another major.
 */
export function generatorTypeScriptProblem(version) {
  const major = Number.parseInt(String(version).split(".")[0], 10);
  if (major === 6) return null;
  return (
    `tools/api-generator resolves TypeScript ${version}, but @hey-api/openapi-ts needs the ` +
    "TypeScript 6 compiler API. Keep `typescript` on ^6 in tools/api-generator/package.json " +
    "(see docs/PRODUCTION_READINESS.md §5) and close Dependabot pull requests that raise it."
  );
}

function checkGeneratorTypeScript() {
  const requireFromGenerator = createRequire(join(root, "tools", "api-generator", "package.json"));
  const { version } = requireFromGenerator("typescript/package.json");
  const problem = generatorTypeScriptProblem(version);
  if (problem) {
    console.error(problem);
    process.exit(1);
  }
}

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

function main() {
  checkGeneratorTypeScript();
  run("cargo", ["test", "--locked", "-p", "symphony-server", "--test", "openapi_contract", "--test", "state_machine_contract"], {
    UPDATE_OPENAPI: "1",
    UPDATE_STATE_MACHINE: "1"
  });
  run("pnpm", ["--filter", "@symphony/api-generator", "generate"]);
}

// Run only when executed directly; the script tests import the helpers.
if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
