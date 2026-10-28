/**
 * Version of the next automatic release (`.github/workflows/auto-release.yml`).
 *
 *   node scripts/next-version.mjs --current 2.0.0 --tags v1.0.0,v2.0.0 --bump minor   # prints 2.1.0
 *
 * A version the manifests declare that no release tag exists for yet is released as it is: that is
 * the first release, or a bump made on purpose in a pull request. Otherwise the newer of the manifest
 * version and the newest release tag is bumped by `--bump` (patch, minor or major).
 */
import { fileURLToPath } from "node:url";

const STABLE = /^(\d+)\.(\d+)\.(\d+)$/;
const BUMPS = ["patch", "minor", "major"];

/** [major, minor, patch] of a plain `x.y.z` version, or null (pre-releases and build metadata included). */
export function parseStable(version) {
  const match = STABLE.exec(version);
  return match ? match.slice(1).map(Number) : null;
}

/** Negative, zero or positive, like a sort comparator, for two plain `x.y.z` versions. */
export function compareStable(a, b) {
  const x = parseStable(a);
  const y = parseStable(b);
  for (let i = 0; i < 3; i++) {
    if (x[i] !== y[i]) return x[i] - y[i];
  }
  return 0;
}

/**
 * The next release version. `tags` are release tags (`v1.2.3`); tags that are not plain stable
 * versions are ignored.
 */
export function nextReleaseVersion({ current, tags = [], bump = "patch" }) {
  if (!parseStable(current)) throw new Error(`current version "${current}" is not x.y.z`);
  if (!BUMPS.includes(bump)) throw new Error(`bump "${bump}" must be one of ${BUMPS.join(", ")}`);
  const released = tags.map((tag) => tag.replace(/^v/, "")).filter((version) => parseStable(version));
  const highest = released.reduce((max, version) => (max === null || compareStable(version, max) > 0 ? version : max), null);

  if (!released.includes(current) && (highest === null || compareStable(current, highest) > 0)) return current;

  const base = highest !== null && compareStable(highest, current) > 0 ? highest : current;
  const [major, minor, patch] = parseStable(base);
  if (bump === "major") return `${major + 1}.0.0`;
  if (bump === "minor") return `${major}.${minor + 1}.0`;
  return `${major}.${minor}.${patch + 1}`;
}

function main() {
  const args = process.argv.slice(2);
  const option = (name, fallback) => {
    const index = args.indexOf(`--${name}`);
    return index >= 0 ? args[index + 1] ?? "" : fallback;
  };
  const tags = option("tags", "").split(",").map((tag) => tag.trim()).filter(Boolean);
  try {
    console.log(nextReleaseVersion({ current: option("current", ""), tags, bump: option("bump", "patch") }));
  } catch (error) {
    console.error(`::error::${error.message}`);
    process.exit(1);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
