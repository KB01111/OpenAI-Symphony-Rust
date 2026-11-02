import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { resourceDirectory, sourceBridge, stagedBridge, stagedNode, supportedNodeVersion } from "./prepare-codesandbox-windows.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const config = JSON.parse(readFileSync(join(root, "apps/desktop/src-tauri/tauri.windows.conf.json"), "utf8"));
const tauri = JSON.parse(readFileSync(join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"));
const desktop = JSON.parse(readFileSync(join(root, "apps/desktop/package.json"), "utf8"));

test("Windows installer maps the compiled bridge and its own Node executable into the discoverable resources layout", () => {
  const target = "resources/codesandbox-runner/";
  assert.deepEqual(config.bundle.resources, {
    [target + "bridge.mjs"]: target + "bridge.mjs",
    [target + "node.exe"]: target + "node.exe"
  });
  assert.equal(stagedBridge, join(resourceDirectory, "bridge.mjs"));
  assert.equal(stagedNode, join(resourceDirectory, "node.exe"));
  assert.equal(sourceBridge, join(root, "tools/codesandbox-runner/dist/bridge.mjs"));
  assert.ok(tauri.build.beforeBuildCommand.includes("prepare:codesandbox"));
  assert.ok(desktop.scripts["prepare:codesandbox"].includes("@symphony/codesandbox-runner build"));
  assert.ok(desktop.scripts["prepare:codesandbox"].includes("prepare-codesandbox-windows.mjs"));
});

test("packaged runtime enforces Node >=22.13 (not just executable presence)", () => {
  for (const value of ["v18.20.0", "v20.19.0", "v22.12.0", "not-a-version", ""]) {
    assert.equal(supportedNodeVersion(value), false, value);
  }
  for (const value of ["v22.13.0", "v22.20.0", "v23.0.0", "v24.0.0"]) {
    assert.equal(supportedNodeVersion(value), true, value);
  }
});
