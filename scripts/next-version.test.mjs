/**
 * Tests for the automatic release version (`node --test scripts/`).
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { compareStable, nextReleaseVersion, parseStable } from "./next-version.mjs";

test("the first release is the version the manifests declare", () => {
  assert.equal(nextReleaseVersion({ current: "2.0.0", tags: [] }), "2.0.0");
});

test("after a release the next merge bumps the patch", () => {
  assert.equal(nextReleaseVersion({ current: "2.0.0", tags: ["v2.0.0"] }), "2.0.1");
  assert.equal(nextReleaseVersion({ current: "2.0.9", tags: ["v2.0.0", "v2.0.9"] }), "2.0.10");
});

test("minor and major bumps reset the lower parts", () => {
  assert.equal(nextReleaseVersion({ current: "2.0.4", tags: ["v2.0.4"], bump: "minor" }), "2.1.0");
  assert.equal(nextReleaseVersion({ current: "2.3.4", tags: ["v2.3.4"], bump: "major" }), "3.0.0");
});

test("a version bumped on purpose in a pull request is released as it is", () => {
  assert.equal(nextReleaseVersion({ current: "2.1.0", tags: ["v2.0.4"] }), "2.1.0");
  assert.equal(nextReleaseVersion({ current: "2.1.0", tags: ["v2.0.4"], bump: "major" }), "2.1.0");
});

test("a manifest version behind the newest tag is bumped from the tag", () => {
  assert.equal(nextReleaseVersion({ current: "2.0.1", tags: ["v2.0.1", "v2.0.5"] }), "2.0.6");
});

test("tags that are not plain stable versions are ignored", () => {
  assert.equal(nextReleaseVersion({ current: "2.0.0", tags: ["v2.0.0-rc.1", "nightly", "v9.9.9-beta"] }), "2.0.0");
  assert.equal(nextReleaseVersion({ current: "2.0.1", tags: ["v2.0.1", "v2.0.1-rc.1"] }), "2.0.2");
});

test("rejects malformed versions and unknown bump kinds", () => {
  assert.throws(() => nextReleaseVersion({ current: "2.0" }), /not x\.y\.z/);
  assert.throws(() => nextReleaseVersion({ current: "2.0.0-rc.1" }), /not x\.y\.z/);
  assert.throws(() => nextReleaseVersion({ current: "2.0.0", bump: "build" }), /must be one of/);
});

test("parseStable and compareStable order versions numerically", () => {
  assert.deepEqual(parseStable("10.2.3"), [10, 2, 3]);
  assert.equal(parseStable("1.2"), null);
  assert.ok(compareStable("2.10.0", "2.9.9") > 0);
  assert.equal(compareStable("2.0.0", "2.0.0"), 0);
});
