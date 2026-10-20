#!/usr/bin/env node
/**
 * API contract versioning policy, enforced by `.github/workflows/api-contract.yml`.
 *
 * Rules (the Rust route table is the source of truth; `API_VERSION` lives in
 * `crates/symphony-server/src/lib.rs`):
 *   - `API_VERSION` never decreases.
 *   - A breaking OpenAPI change (any oasdiff `breaking` finding at level ERR) is accepted only
 *     when the pull request carries the `api-breaking-change` label AND raises `API_VERSION`
 *     above the target branch's value. Missing either one fails.
 *   - A non-breaking change never requires a version bump (a bump is allowed, with a notice).
 *
 * CLI (used by CI):
 *   node scripts/api-contract-policy.mjs \
 *     --changes oasdiff-breaking.json        # `oasdiff breaking --format json` output
 *     --base-source base/crates/symphony-server/src/lib.rs \
 *     --head-source head/crates/symphony-server/src/lib.rs \
 *     --labelled true|false
 * Exit code 0 = allowed, 1 = rejected, 2 = invalid input.
 */
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

/** oasdiff severity of a breaking change (`level` 3 = ERR, 2 = WARN, 1 = INFO). */
export const OASDIFF_ERR = 3;

/** Extract `API_VERSION` from the server crate's source; throws when it cannot be found. */
export function readApiVersion(rustSource) {
  const match = /pub\s+const\s+API_VERSION\s*:\s*u32\s*=\s*(\d+)\s*;/.exec(rustSource);
  if (!match) throw new Error("pub const API_VERSION: u32 = <n>; not found");
  return Number(match[1]);
}

/** Breaking findings (level ERR) from `oasdiff breaking --format json` output. */
export function breakingFindings(changes) {
  if (!Array.isArray(changes)) throw new Error("oasdiff output must be a JSON array");
  return changes.filter((c) => Number(c?.level) >= OASDIFF_ERR);
}

/**
 * Decide whether a contract change is acceptable.
 * @param {{ breaking: number, labelled: boolean, baseVersion: number | null, headVersion: number }} input
 *   `breaking` is the number of breaking findings; `baseVersion` is `null` when the target branch
 *   has no API yet.
 * @returns {{ ok: boolean, errors: string[], notices: string[] }}
 */
export function evaluateContractChange({ breaking, labelled, baseVersion, headVersion }) {
  const errors = [];
  const notices = [];
  if (!Number.isInteger(headVersion) || headVersion < 1) {
    errors.push(`API_VERSION must be a positive integer (found ${headVersion}).`);
    return { ok: false, errors, notices };
  }
  if (baseVersion === null) {
    notices.push("The target branch has no API contract yet; nothing to compare.");
    return { ok: true, errors, notices };
  }
  const bumped = headVersion > baseVersion;
  if (headVersion < baseVersion) {
    errors.push(`API_VERSION decreased from ${baseVersion} to ${headVersion}; it must never go down.`);
  }
  if (breaking > 0) {
    if (!labelled) {
      errors.push(
        `${breaking} breaking API change(s) detected. Avoid the break, or label the pull request ` +
          "'api-breaking-change' and increase API_VERSION in crates/symphony-server/src/lib.rs."
      );
    }
    if (!bumped) {
      errors.push(
        `${breaking} breaking API change(s) require API_VERSION to increase above ${baseVersion} ` +
          `(it is ${headVersion}).`
      );
    }
    if (labelled && bumped) {
      notices.push(`Intentional breaking change accepted: API_VERSION ${baseVersion} -> ${headVersion}.`);
    }
  } else {
    if (bumped) notices.push(`API_VERSION raised ${baseVersion} -> ${headVersion} without a breaking change (allowed).`);
    if (labelled) notices.push("The 'api-breaking-change' label is set but no breaking change was detected.");
  }
  return { ok: errors.length === 0, errors, notices };
}

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i];
    if (!key?.startsWith("--") || argv[i + 1] === undefined) throw new Error(`invalid argument ${key}`);
    args[key.slice(2)] = argv[i + 1];
  }
  for (const required of ["changes", "head-source", "labelled"]) {
    if (!(required in args)) throw new Error(`missing --${required}`);
  }
  return args;
}

function main() {
  let args;
  let input;
  try {
    args = parseArgs(process.argv.slice(2));
    const changes = JSON.parse(readFileSync(args.changes, "utf8"));
    const baseSource = args["base-source"];
    input = {
      breaking: breakingFindings(changes).length,
      labelled: args.labelled === "true",
      baseVersion: baseSource && existsSync(baseSource) ? readApiVersion(readFileSync(baseSource, "utf8")) : null,
      headVersion: readApiVersion(readFileSync(args["head-source"], "utf8"))
    };
  } catch (error) {
    console.error(`::error::api-contract-policy: ${error.message}`);
    process.exit(2);
  }
  const result = evaluateContractChange(input);
  console.log(
    `breaking changes: ${input.breaking}, label: ${input.labelled}, API_VERSION base: ${input.baseVersion ?? "none"}, head: ${input.headVersion}`
  );
  for (const notice of result.notices) console.log(`::notice::${notice}`);
  for (const error of result.errors) console.log(`::error::${error}`);
  process.exit(result.ok ? 0 : 1);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
