/**
 * Release version consistency.
 *
 * One product version is declared in several manifests (Cargo workspace, every package.json and the
 * Tauri bundle config). The installer, `symphony-cli --version`, the About screen and the release tag
 * must all agree, so CI and the release workflow fail when they drift:
 *
 *   node scripts/release-version.mjs                 # all manifests agree
 *   node scripts/release-version.mjs --tag v2.1.0    # ...and match the release tag
 *   node scripts/release-version.mjs --tag v2.1.0 --notes out.md
 *                                                    # ...and write its dated CHANGELOG.md section
 */
import { readFileSync, readdirSync, existsSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

// MAJOR.MINOR.PATCH with optional pre-release and build metadata (semver.org).
const IDENTIFIERS = "[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*"; // non-empty, dot-separated
const SEMVER = new RegExp(`^\\d+\\.\\d+\\.\\d+(?:-${IDENTIFIERS})?(?:\\+${IDENTIFIERS})?$`);

/** Version of `[workspace.package]` in the root Cargo.toml, or null. */
export function readCargoWorkspaceVersion(cargoToml) {
  const section = cargoToml.match(/^\[workspace\.package\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m);
  if (!section) return null;
  const version = section[1].match(/^version\s*=\s*"([^"]+)"/m);
  return version ? version[1] : null;
}

/** Every manifest that carries the product version: [{ file, version }]. */
export function collectVersions(root) {
  const found = [];
  const add = (file, version) => found.push({ file: relative(root, file).replaceAll("\\", "/"), version });

  const cargo = join(root, "Cargo.toml");
  add(cargo, readCargoWorkspaceVersion(readFileSync(cargo, "utf8")));

  const packageFiles = [join(root, "package.json")];
  for (const group of ["apps", "packages", "tools"]) {
    const dir = join(root, group);
    if (!existsSync(dir)) continue;
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const file = join(dir, entry.name, "package.json");
      if (entry.isDirectory() && existsSync(file)) packageFiles.push(file);
    }
  }
  for (const file of packageFiles) {
    add(file, JSON.parse(readFileSync(file, "utf8")).version ?? null);
  }

  const tauri = join(root, "apps", "desktop", "src-tauri", "tauri.conf.json");
  if (existsSync(tauri)) add(tauri, JSON.parse(readFileSync(tauri, "utf8")).version ?? null);
  return found;
}

/** Errors for disagreeing or malformed versions, and for a tag that is not `v<version>`. */
export function checkVersions(versions, tag) {
  const errors = [];
  for (const { file, version } of versions) {
    if (!version || !SEMVER.test(version)) errors.push(`${file}: missing or invalid version "${version ?? ""}"`);
  }
  const distinct = [...new Set(versions.map((v) => v.version))];
  if (distinct.length > 1) {
    errors.push(
      `manifests disagree on the version: ${versions.map((v) => `${v.file}=${v.version}`).join(", ")}`
    );
  }
  const version = distinct.length === 1 ? distinct[0] : null;
  if (tag !== undefined && tag !== null) {
    if (!/^v\d/.test(tag)) errors.push(`release tag "${tag}" must look like v<major>.<minor>.<patch>`);
    else if (version && tag.slice(1) !== version) errors.push(`release tag ${tag} does not match version ${version}`);
  }
  return { ok: errors.length === 0, version, errors };
}

/** Body of the `## [<version>]` section of a Keep-a-Changelog file, or null. */
export function changelogSection(changelog, version) {
  const lines = changelog.split(/\r?\n/);
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const heading = new RegExp(`^##\\s+\\[?${escaped}\\]?(?:\\s|$)`);
  const start = lines.findIndex((line) => heading.test(line));
  if (start < 0) return null;
  const rest = lines.slice(start + 1);
  const end = rest.findIndex((line) => /^##\s/.test(line) || /^\[[^\]]+\]:\s/.test(line));
  const body = (end < 0 ? rest : rest.slice(0, end)).join("\n").trim();
  return body.length > 0 ? body : null;
}

/** Release date (`YYYY-MM-DD`) on the `## [<version>]` heading, or null (e.g. "Unreleased"). */
export function changelogReleaseDate(changelog, version) {
  const line = changelog.split(/\r?\n/).find((l) => l.startsWith(`## [${version}]`));
  const date = line?.match(/^## \[[^\]]+\]\s+-\s+(\d{4}-\d{2}-\d{2})\s*$/)?.[1];
  // Only real calendar dates (2026-02-30 and 2026-99-99 are rejected).
  const parsed = date ? new Date(`${date}T00:00:00Z`) : null;
  return parsed && !Number.isNaN(parsed.getTime()) && parsed.toISOString().startsWith(date) ? date : null;
}

function main() {
  const args = process.argv.slice(2);
  const tagIndex = args.indexOf("--tag");
  const tag = tagIndex >= 0 ? args[tagIndex + 1] ?? "" : undefined;
  const notesIndex = args.indexOf("--notes");
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  const result = checkVersions(collectVersions(root), tag);
  if (notesIndex >= 0 && result.ok) {
    const changelog = join(root, "CHANGELOG.md");
    const text = existsSync(changelog) ? readFileSync(changelog, "utf8") : "";
    const notes = changelogSection(text, result.version);
    if (!notes) result.errors.push(`CHANGELOG.md has no section for ${result.version}`);
    else if (!changelogReleaseDate(text, result.version)) {
      result.errors.push(`CHANGELOG.md: date the "## [${result.version}]" heading (YYYY-MM-DD) before releasing it`);
    } else writeFileSync(args[notesIndex + 1], `${notes}\n`);
  }
  if (result.errors.length > 0) {
    for (const error of result.errors) console.error(`::error::${error}`);
    process.exit(1);
  }
  console.log(result.version);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
