/**
 * Tests for the release version consistency check (`node --test scripts/`).
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { changelogReleaseDate, changelogSection, checkVersions, collectVersions, readCargoWorkspaceVersion } from "./release-version.mjs";

const here = dirname(fileURLToPath(import.meta.url));

function fixture({ cargo = "2.1.0", root = "2.1.0", app = "2.1.0", tauri = "2.1.0", tool = "2.1.0" } = {}) {
  const dir = mkdtempSync(join(tmpdir(), "symphony-version-"));
  writeFileSync(
    join(dir, "Cargo.toml"),
    `[workspace]\nmembers = []\n\n[workspace.package]\nversion = "${cargo}"\nedition = "2021"\n\n[workspace.dependencies]\nserde = { version = "1.0" }\n`
  );
  writeFileSync(join(dir, "package.json"), JSON.stringify({ name: "root", version: root }));
  mkdirSync(join(dir, "apps", "desktop", "src-tauri"), { recursive: true });
  writeFileSync(join(dir, "apps", "desktop", "package.json"), JSON.stringify({ name: "desktop", version: app }));
  writeFileSync(join(dir, "apps", "desktop", "src-tauri", "tauri.conf.json"), JSON.stringify({ version: tauri }));
  mkdirSync(join(dir, "tools", "api-generator"), { recursive: true });
  writeFileSync(join(dir, "tools", "api-generator", "package.json"), JSON.stringify({ name: "api-generator", version: tool }));
  return dir;
}

test("reads the workspace version, not a dependency version", () => {
  assert.equal(readCargoWorkspaceVersion('[workspace.dependencies]\nx = { version = "9" }\n[workspace.package]\nversion = "1.2.3"\n'), "1.2.3");
  assert.equal(readCargoWorkspaceVersion("[workspace]\n"), null);
});

test("agreeing manifests pass and report the version", () => {
  const dir = fixture();
  try {
    const result = checkVersions(collectVersions(dir));
    assert.deepEqual(result, { ok: true, version: "2.1.0", errors: [] });
    assert.equal(collectVersions(dir).length, 5);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a drifting manifest is named", () => {
  const dir = fixture({ tauri: "2.0.0" });
  try {
    const result = checkVersions(collectVersions(dir));
    assert.equal(result.ok, false);
    assert.match(result.errors.join(" "), /tauri\.conf\.json=2\.0\.0/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a workspace tool's manifest is checked too", () => {
  const dir = fixture({ tool: "2.0.0" });
  try {
    const result = checkVersions(collectVersions(dir));
    assert.equal(result.ok, false);
    assert.match(result.errors.join(" "), /tools\/api-generator\/package\.json=2\.0\.0/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the release tag must match the version", () => {
  const versions = [{ file: "Cargo.toml", version: "2.1.0" }];
  assert.equal(checkVersions(versions, "v2.1.0").ok, true);
  assert.match(checkVersions(versions, "v2.1.1").errors.join(" "), /does not match/);
  assert.match(checkVersions(versions, "2.1.0").errors.join(" "), /must look like/);
  assert.match(checkVersions(versions, "").errors.join(" "), /must look like/);
});

test("pre-release and build metadata are valid versions", () => {
  for (const version of ["2.1.0-beta.1", "2.1.0+build.7", "2.1.0-rc.1+sha.5114f85"]) {
    const versions = [{ file: "Cargo.toml", version }];
    assert.deepEqual(checkVersions(versions, `v${version}`).errors, [], version);
  }
  for (const invalid of ["2.1.0+", "2.1.0+build..7", "2.1.0+build.", "2.1.0-rc..1", "2.1.0-"]) {
    assert.equal(checkVersions([{ file: "a", version: invalid }]).ok, false, invalid);
  }
});

test("malformed versions are rejected", () => {
  const result = checkVersions([{ file: "a", version: "two" }, { file: "b", version: null }]);
  assert.equal(result.ok, false);
  assert.equal(result.errors.filter((e) => /invalid version/.test(e)).length, 2);
});

test("the repository itself is consistent (CLI)", () => {
  const run = spawnSync(process.execPath, [join(here, "release-version.mjs")], { encoding: "utf8" });
  assert.equal(run.status, 0, run.stderr);
  assert.match(run.stdout.trim(), /^\d+\.\d+\.\d+/);
});

test("release notes come from the matching CHANGELOG section only", () => {
  const md = [
    "# Changelog",
    "",
    "## [Unreleased]",
    "- pending",
    "",
    "## [2.1.0] - 2026-10-21",
    "### Fixed",
    "- one",
    "",
    "## [2.0.0] - 2026-09-01",
    "- old",
    "",
    "[2.1.0]: https://example.invalid/compare/v2.0.0...v2.1.0"
  ].join("\n");
  assert.equal(changelogSection(md, "2.1.0"), "### Fixed\n- one");
  assert.equal(changelogSection(md, "2.0.0"), "- old");
  assert.equal(changelogSection(md, "2.0.1"), null);
  // "2.1.0" must not match "2.1.01" or a regex wildcard.
  assert.equal(changelogSection("## [2.1.01]\n- no", "2.1.0"), null);
  assert.equal(changelogSection("## [2.1.0-rc.1]\n- no", "2.1.0"), null);
  assert.equal(changelogSection("## [2x1x0]\n- no", "2.1.0"), null);
});

test("only a dated heading is releasable", () => {
  assert.equal(changelogReleaseDate("## [2.1.0] - 2026-10-21\n- x", "2.1.0"), "2026-10-21");
  assert.equal(changelogReleaseDate("## [2.1.0] - Unreleased\n- x", "2.1.0"), null);
  assert.equal(changelogReleaseDate("## [2.1.0]\n- x", "2.1.0"), null);
  assert.equal(changelogReleaseDate("## [2.1.0] - 2026-99-99\n- x", "2.1.0"), null);
  assert.equal(changelogReleaseDate("## [2.1.0] - 2026-02-30\n- x", "2.1.0"), null);
  assert.equal(changelogReleaseDate("## [2.1.0] - 2028-02-29\n- x", "2.1.0"), "2028-02-29");
});

test("the repository's CHANGELOG documents the current version", () => {
  const run = spawnSync(process.execPath, [join(here, "release-version.mjs")], { encoding: "utf8" });
  const version = run.stdout.trim();
  const changelog = readFileSync(join(here, "..", "CHANGELOG.md"), "utf8");
  assert.ok(changelogSection(changelog, version), `CHANGELOG.md needs a "## [${version}]" section`);
});
