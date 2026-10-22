/**
 * Tests for the updater manifest generator (`node --test scripts/`).
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { PLATFORM, buildManifest, checkSignature } from "./updater-manifest.mjs";

const here = dirname(fileURLToPath(import.meta.url));
// What `tauri signer sign` writes: base64 of a minisign signature file with the exact field sizes
// (the bytes are not a real signature, only the structure is checked here).
const minisign = ({ id = "ED", signature = 72, global = 64, comments = ["untrusted comment: signature from tauri secret key", "trusted comment: timestamp:1"] } = {}) =>
  Buffer.from(
    [
      comments[0],
      Buffer.concat([Buffer.from(id), Buffer.alloc(signature, 7)]).toString("base64"),
      comments[1],
      Buffer.alloc(global, 9).toString("base64"),
      ""
    ].join("\n")
  ).toString("base64");
const SIGNATURE = minisign();
const input = { tag: "v2.1.0", repo: "KB01111/OpenAI-Symphony-Rust", installer: "Symphony-Desktop_2.1.0_x64-setup.exe", signature: `${SIGNATURE}\n`, notes: "  Faster startup\n" };

test("builds the manifest the updater expects", () => {
  const manifest = buildManifest({ ...input, pubDate: new Date("2026-10-22T10:00:00.123Z") });
  assert.deepEqual(manifest, {
    version: "2.1.0",
    notes: "Faster startup",
    pub_date: "2026-10-22T10:00:00Z",
    platforms: {
      [PLATFORM]: {
        signature: SIGNATURE,
        url: "https://github.com/KB01111/OpenAI-Symphony-Rust/releases/download/v2.1.0/Symphony-Desktop_2.1.0_x64-setup.exe"
      }
    }
  });
});

test("keeps pre-release tags and refuses anything that is not v<semver>", () => {
  assert.equal(buildManifest({ ...input, tag: "v2.1.0-rc.1" }).version, "2.1.0-rc.1");
  for (const tag of ["2.1.0", "vnext", "v2.1", "v2.1.0/../x"]) {
    assert.throws(() => buildManifest({ ...input, tag }), /tag/);
  }
});

test("refuses a repository or installer name that could change the URL", () => {
  assert.throws(() => buildManifest({ ...input, repo: "a/b/c" }), /owner\/name/);
  assert.throws(() => buildManifest({ ...input, repo: "evil.example/x?y=" }), /owner\/name/);
  assert.throws(() => buildManifest({ ...input, installer: "../setup.exe" }), /unexpected characters/);
  assert.throws(() => buildManifest({ ...input, installer: "a b.exe" }), /unexpected characters/);
});

test("refuses a missing or foreign signature", () => {
  assert.throws(() => buildManifest({ ...input, signature: "" }), /minisign/);
  assert.throws(() => buildManifest({ ...input, signature: "not base64 of a signature" }), /minisign/);
  assert.doesNotThrow(() => checkSignature(SIGNATURE));
  assert.doesNotThrow(() => checkSignature(minisign({ id: "Ed" })));
  // Right header, wrong shape: a truncated or edited file must not reach the release.
  const bad = [
    minisign({ signature: 10 }),
    minisign({ global: 10 }),
    minisign({ id: "XX" }),
    minisign({ comments: ["comment: nope", "trusted comment: x"] }),
    minisign({ comments: ["untrusted comment: x", "comment: nope"] }),
    Buffer.from("untrusted comment: only a header\n").toString("base64"),
    `${SIGNATURE}!`
  ];
  for (const signature of bad) assert.throws(() => checkSignature(signature), /minisign/);
});

test("the command line writes the manifest and fails with a message on bad input", () => {
  const dir = mkdtempSync(join(tmpdir(), "symphony-manifest-"));
  writeFileSync(join(dir, "setup.exe.sig"), SIGNATURE);
  writeFileSync(join(dir, "notes.md"), "Notes\n");
  const out = join(dir, "latest.json");
  const args = ["--tag", "v2.1.0", "--repo", input.repo, "--installer", input.installer, "--signature", join(dir, "setup.exe.sig"), "--notes", join(dir, "notes.md"), "--out", out];
  const ok = spawnSync(process.execPath, [join(here, "updater-manifest.mjs"), ...args], { encoding: "utf8" });
  assert.equal(ok.status, 0, ok.stderr);
  const written = JSON.parse(readFileSync(out, "utf8"));
  assert.equal(written.version, "2.1.0");
  assert.equal(written.notes, "Notes");

  writeFileSync(join(dir, "setup.exe.sig"), "garbage");
  const bad = spawnSync(process.execPath, [join(here, "updater-manifest.mjs"), ...args], { encoding: "utf8" });
  assert.equal(bad.status, 1);
  assert.match(bad.stderr, /updater-manifest: .*minisign/);
});
