/**
 * Tests for the API versioning policy (`node --test scripts/`). They prove that CI rejects every
 * invalid breaking-change scenario and does not demand needless version bumps.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { breakingFindings, evaluateContractChange, readApiVersion } from "./api-contract-policy.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const script = join(here, "api-contract-policy.mjs");

const BREAKING = [
  { id: "api-path-removed-without-deprecation", level: 3, operation: "GET", path: "/api/v1/worktrees" },
  { id: "response-optional-property-removed", level: 2, operation: "GET", path: "/api/v1/tasks" }
];

test("breaking change without label and without bump is rejected", () => {
  const r = evaluateContractChange({ breaking: 1, labelled: false, baseVersion: 2, headVersion: 2 });
  assert.equal(r.ok, false);
  assert.equal(r.errors.length, 2);
});

test("breaking change with the label but without a bump is rejected", () => {
  const r = evaluateContractChange({ breaking: 1, labelled: true, baseVersion: 2, headVersion: 2 });
  assert.equal(r.ok, false);
  assert.match(r.errors.join(" "), /increase above 2/);
});

test("breaking change with a bump but without the label is rejected", () => {
  const r = evaluateContractChange({ breaking: 1, labelled: false, baseVersion: 2, headVersion: 3 });
  assert.equal(r.ok, false);
  assert.match(r.errors.join(" "), /api-breaking-change/);
});

test("breaking change with the label and a bump is accepted", () => {
  const r = evaluateContractChange({ breaking: 2, labelled: true, baseVersion: 2, headVersion: 3 });
  assert.equal(r.ok, true);
  assert.deepEqual(r.errors, []);
});

test("non-breaking change needs neither label nor bump", () => {
  const r = evaluateContractChange({ breaking: 0, labelled: false, baseVersion: 2, headVersion: 2 });
  assert.equal(r.ok, true);
  assert.deepEqual(r.notices, []);
});

test("an optional bump or a stray label on a non-breaking change is allowed with a notice", () => {
  const r = evaluateContractChange({ breaking: 0, labelled: true, baseVersion: 2, headVersion: 3 });
  assert.equal(r.ok, true);
  assert.equal(r.notices.length, 2);
});

test("API_VERSION may never decrease, breaking or not", () => {
  assert.equal(evaluateContractChange({ breaking: 0, labelled: false, baseVersion: 3, headVersion: 2 }).ok, false);
  assert.equal(evaluateContractChange({ breaking: 1, labelled: true, baseVersion: 3, headVersion: 2 }).ok, false);
});

test("a target branch without an API contract is accepted", () => {
  assert.equal(evaluateContractChange({ breaking: 5, labelled: false, baseVersion: null, headVersion: 1 }).ok, true);
});

test("only ERR-level oasdiff findings count as breaking", () => {
  assert.equal(breakingFindings(BREAKING).length, 1);
  assert.equal(breakingFindings([]).length, 0);
  assert.throws(() => breakingFindings({}));
});

test("API_VERSION is read from the server crate", () => {
  assert.equal(readApiVersion("/// doc\npub const API_VERSION: u32 = 7;\n"), 7);
  assert.throws(() => readApiVersion("const OTHER: u32 = 1;"));
  // Commented-out declarations never count.
  assert.equal(readApiVersion("// pub const API_VERSION: u32 = 9;\npub const API_VERSION: u32 = 2;"), 2);
  assert.throws(() => readApiVersion("/* pub const API_VERSION: u32 = 9; */"));
  assert.throws(() => readApiVersion("// pub const API_VERSION: u32 = 9;"));
  assert.throws(() => readApiVersion("pub const API_VERSION: u32 = 2;\npub const API_VERSION: u32 = 3;"));
  const current = readFileSync(join(here, "../crates/symphony-server/src/lib.rs"), "utf8");
  assert.ok(readApiVersion(current) >= 2);
});

function runCli({ changes, base, head, labelled }) {
  const dir = mkdtempSync(join(tmpdir(), "api-policy-"));
  writeFileSync(join(dir, "changes.json"), JSON.stringify(changes));
  writeFileSync(join(dir, "head.rs"), `pub const API_VERSION: u32 = ${head};\n`);
  const args = [script, "--changes", join(dir, "changes.json"), "--head-source", join(dir, "head.rs"), "--labelled", String(labelled)];
  if (base !== null) {
    writeFileSync(join(dir, "base.rs"), `pub const API_VERSION: u32 = ${base};\n`);
    args.push("--base-source", join(dir, "base.rs"));
  }
  try {
    return spawnSync(process.execPath, args, { encoding: "utf8" });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("CLI exit codes follow the policy", () => {
  assert.equal(runCli({ changes: BREAKING, base: 2, head: 2, labelled: false }).status, 1);
  assert.equal(runCli({ changes: BREAKING, base: 2, head: 2, labelled: true }).status, 1);
  assert.equal(runCli({ changes: BREAKING, base: 2, head: 3, labelled: false }).status, 1);
  assert.equal(runCli({ changes: BREAKING, base: 2, head: 3, labelled: true }).status, 0);
  assert.equal(runCli({ changes: [], base: 2, head: 2, labelled: false }).status, 0);
  const rejected = runCli({ changes: BREAKING, base: 2, head: 2, labelled: false });
  assert.match(rejected.stdout, /::error::/);
});

test("CLI rejects malformed input with exit code 2", () => {
  const dir = mkdtempSync(join(tmpdir(), "api-policy-"));
  writeFileSync(join(dir, "changes.json"), "not json");
  writeFileSync(join(dir, "head.rs"), "pub const API_VERSION: u32 = 2;\n");
  const r = spawnSync(process.execPath, [script, "--changes", join(dir, "changes.json"), "--head-source", join(dir, "head.rs"), "--labelled", "false"], {
    encoding: "utf8"
  });
  rmSync(dir, { recursive: true, force: true });
  assert.equal(r.status, 2);
});
