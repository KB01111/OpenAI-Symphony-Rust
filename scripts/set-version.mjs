/**
 * Writes one product version into every manifest that carries it, so `release-version.mjs` agrees
 * with the release tag: the Cargo workspace version, every package.json, the Tauri bundle config and
 * the workspace crates' entries in Cargo.lock.
 *
 *   node scripts/set-version.mjs 2.1.0
 *
 * Only the version lines change, so the diff of a release commit is the version and nothing else.
 */
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { collectVersions, readCargoWorkspaceVersion } from "./release-version.mjs";

const SEMVER_STABLE = /^\d+\.\d+\.\d+$/;

/** Cargo.toml with the `version` of `[workspace.package]` replaced. */
export function setCargoWorkspaceVersion(cargoToml, version) {
  return cargoToml.replace(/^(\[workspace\.package\][\s\S]*?^version\s*=\s*)"[^"]*"/m, `$1"${version}"`);
}

/**
 * package.json / tauri.conf.json text with the top-level `version` replaced. The top-level key is the
 * only one indented by exactly two spaces in these files, so nested keys named `version` are left alone.
 */
export function setJsonVersion(jsonText, version) {
  return jsonText.replace(/^(  "version": )"[^"]*"/m, `$1"${version}"`);
}

/**
 * Cargo.lock with the workspace crates moved from `from` to `to`. Workspace (path) crates are the
 * `[[package]]` entries without a `source`; registry and git packages always have one and keep their
 * versions.
 */
export function setCargoLockVersions(lockText, from, to) {
  return lockText
    .split(/(?=^\[\[package\]\]$)/m)
    .map((block) => {
      if (!block.startsWith("[[package]]") || /^source = /m.test(block)) return block;
      return block.replace(/^version = "([^"]*)"$/m, (line, version) => (version === from ? `version = "${to}"` : line));
    })
    .join("");
}

/** Writes `version` into every manifest under `root`, then returns the files it changed. */
export function setProductVersion(root, version) {
  if (!SEMVER_STABLE.test(version)) throw new Error(`version "${version}" must be x.y.z`);
  const before = collectVersions(root);
  const from = before.find((entry) => entry.file === "Cargo.toml")?.version;
  if (!from) throw new Error("Cargo.toml has no [workspace.package] version");

  const changed = [];
  const update = (relativeFile, transform) => {
    const file = join(root, relativeFile);
    const text = readFileSync(file, "utf8");
    const next = transform(text);
    if (next !== text) {
      writeFileSync(file, next);
      changed.push(relativeFile);
    }
  };

  update("Cargo.toml", (text) => setCargoWorkspaceVersion(text, version));
  update("Cargo.lock", (text) => setCargoLockVersions(text, from, version));
  for (const { file } of before) {
    if (file === "Cargo.toml" || file === "Cargo.lock") continue;
    update(file, (text) => setJsonVersion(text, version));
  }
  return changed;
}

function main() {
  const version = process.argv[2];
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  if (!version) {
    console.error("usage: node scripts/set-version.mjs <x.y.z>");
    process.exit(2);
  }
  try {
    const changed = setProductVersion(root, version);
    const now = collectVersions(root);
    const stale = now.filter((entry) => entry.version !== version).map((entry) => entry.file);
    if (stale.length > 0 || readCargoWorkspaceVersion(readFileSync(join(root, "Cargo.toml"), "utf8")) !== version) {
      console.error(`::error::these manifests did not take version ${version}: ${stale.join(", ")}`);
      process.exit(1);
    }
    console.log(`version ${version}: changed ${changed.join(", ")}`);
  } catch (error) {
    console.error(`::error::${error.message}`);
    process.exit(1);
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
