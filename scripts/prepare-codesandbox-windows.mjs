#!/usr/bin/env node
/**
 * Prepare the self-contained CodeSandbox bridge for Tauri's Windows NSIS bundle.
 * The Node executable comes from the trusted Node installation used to build Symphony
 * (CI uses actions/setup-node@22). We do not download or execute binaries at install time.
 *
 * Usage: node scripts/prepare-codesandbox-windows.mjs [--verify]
 * The runner must have been compiled by pnpm --filter @symphony/codesandbox-runner build.
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, existsSync } from "node:fs";
import { copyFile, mkdir, stat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
export const resourceDirectory = join(root, "apps/desktop/src-tauri/resources/codesandbox-runner");
export const sourceBridge = join(root, "tools/codesandbox-runner/dist/bridge.mjs");
export const stagedBridge = join(resourceDirectory, "bridge.mjs");
export const stagedNode = join(resourceDirectory, "node.exe");

export function supportedNodeVersion(value) {
  const match = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(value.trim());
  if (!match) return false;
  const major = Number(match[1]);
  const minor = Number(match[2]);
  return major > 22 || (major === 22 && minor >= 13);
}

function run(executable, args, description) {
  const result = spawnSync(executable, args, { encoding: "utf8", timeout: 30_000, windowsHide: true });
  if (result.error || result.status !== 0) {
    throw new Error(`${description} failed: ${result.error?.message ?? result.stderr?.trim() ?? result.status}`);
  }
  return result.stdout.trim();
}

async function checksum(file) {
  const hash = createHash("sha256");
  for await (const bytes of createReadStream(file)) hash.update(bytes);
  return hash.digest("hex");
}

export async function verifyBundle() {
  for (const file of [stagedBridge, stagedNode]) {
    if (!existsSync(file) || (await stat(file)).size === 0) throw new Error(`CodeSandbox bundle resource missing/empty: ${file}`);
  }
  const version = run(stagedNode, ["--version"], "bundled Node version probe");
  if (!supportedNodeVersion(version)) throw new Error(`CodeSandbox requires Node.js >=22.13.0; bundled Node is ${version}`);
  run(stagedNode, ["--check", stagedBridge], "bundled CodeSandbox bridge syntax check");
  console.log(`CodeSandbox Windows resources verified: ${version}; node.exe SHA256 ${await checksum(stagedNode)}; bridge.mjs SHA256 ${await checksum(stagedBridge)}`);
}

export async function prepareBundle() {
  if (process.platform !== "win32" || process.arch !== "x64") {
    throw new Error("CodeSandbox Windows NSIS bundles require a Windows x64 Node.js build host");
  }
  if (!supportedNodeVersion(process.version)) {
    throw new Error(`Build Node.js >=22.13.0 required, found ${process.version}`);
  }
  if (!existsSync(sourceBridge) || (await stat(sourceBridge)).size === 0) {
    throw new Error("Build tools/codesandbox-runner/dist/bridge.mjs first (pnpm --filter @symphony/codesandbox-runner build)");
  }
  run(process.execPath, ["--check", sourceBridge], "compiled CodeSandbox bridge syntax check");
  await mkdir(resourceDirectory, { recursive: true });
  await copyFile(sourceBridge, stagedBridge);
  // Copy the exact Windows x64 Node runtime that performed the build; no end-user PATH dependency.
  await copyFile(process.execPath, stagedNode);
  await verifyBundle();
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.slice(2).includes("--verify")) await verifyBundle();
    else await prepareBundle();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
