#!/usr/bin/env node
/**
 * Builds the sandbox template (template/) with the CodeSandbox SDK's CLI and prints how to use it.
 *
 *   CSB_API_KEY=... pnpm --filter @symphony/codesandbox-runner build:template
 *
 * Options (all optional):  --alias <name>   default "symphony-agents"
 *                          --vm-tier <Pico|Nano|Micro|Small|Medium|Large|XLarge>   default Small
 *                          --vm-build-tier <same choices>                          default Medium
 */
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { resolveCsbBin } from "./csb-bin.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const TIERS = ["Pico", "Nano", "Micro", "Small", "Medium", "Large", "XLarge"];

function option(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

if (!process.env.CSB_API_KEY) {
  console.error("CSB_API_KEY is not set. Create an API token at https://codesandbox.io/t/api and export it.");
  process.exit(2);
}

const alias = option("alias", "symphony-agents");
// The CLI is run without a shell (see below), so nothing here could be interpreted as shell syntax;
// the alias is still restricted to what a template alias can contain.
if (!/^[A-Za-z0-9][A-Za-z0-9._@-]{0,63}$/.test(alias)) {
  console.error(`Invalid --alias "${alias}": use letters, digits and . _ @ - (at most 64, starting with a letter or digit).`);
  process.exit(2);
}
const vmTier = option("vm-tier", "Small");
const vmBuildTier = option("vm-build-tier", "Medium");
for (const tier of [vmTier, vmBuildTier]) {
  if (!TIERS.includes(tier)) {
    console.error(`Unknown VM tier "${tier}". Choose one of: ${TIERS.join(", ")}`);
    process.exit(2);
  }
}

const args = [
  "build",
  join(root, "template"),
  "--name",
  "Symphony cloud agents",
  "--alias",
  alias,
  "--vm-tier",
  vmTier,
  "--vm-build-tier",
  vmBuildTier,
  "--ci"
];
console.log(`> csb ${args.join(" ")}`);
// `node <csb script>`: no shell, so arguments with spaces stay one argument each and nothing in them is
// interpreted, on Windows (where the npm shim is a .cmd file) as much as elsewhere.
const result = spawnSync(process.execPath, [resolveCsbBin(), ...args], { stdio: "inherit", env: process.env });
if (result.status !== 0) process.exit(result.status ?? 1);

console.log(`
Template built. Use the template id printed above in WORKFLOW.md:

  codesandbox:
    template_id: <template id>

(The alias "${alias}" points at it; use the id if the alias is not accepted for your account.)`);
