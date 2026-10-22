/**
 * Builds `latest.json`, the manifest the in-app updater (tauri-plugin-updater) reads from a GitHub
 * release: the version, the release notes, and for each platform the installer URL and the
 * minisign signature of that installer (`<installer>.sig`, created by `tauri signer sign`).
 *
 *   node scripts/updater-manifest.mjs --tag v2.1.0 --repo owner/name \
 *     --installer Symphony-Desktop_2.1.0_x64-setup.exe --signature setup.exe.sig \
 *     --notes notes.md --out latest.json
 *
 * The manifest is not itself signed: the app verifies the installer's signature against the public
 * key compiled into it, so a wrong or tampered manifest can only fail an update, not install one.
 */
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
const REPO = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const ASSET = /^[A-Za-z0-9._-]+$/;
/** The updater picks `<os>-<arch>` for the running target; the only shipped build is Windows x64. */
export const PLATFORM = "windows-x86_64";

/** Decodes strict base64 (canonical padding, no stray characters) or returns null. */
function strictBase64(text) {
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(text)) return null;
  const bytes = Buffer.from(text, "base64");
  return bytes.toString("base64") === text ? bytes : null;
}

/**
 * Fails unless `signature` has the structure the updater parses: base64 of a minisign signature
 * file, i.e. an untrusted comment, a 74-byte signature line (`Ed`/`ED` + 8-byte key id + 64-byte
 * Ed25519 signature), a trusted comment and a 64-byte global signature. It cannot check the
 * signature itself (that needs the public key); it catches truncated, hand-edited or foreign files
 * before they are published.
 */
export function checkSignature(signature) {
  const fail = (why) => {
    throw new Error(`the signature is not a minisign signature created by \`tauri build\` / \`tauri signer sign\` (${why})`);
  };
  const outer = strictBase64(signature);
  if (!outer) fail("not base64");
  const lines = outer.toString("utf8").split("\n");
  if (lines.at(-1) === "") lines.pop();
  if (lines.length !== 4) fail(`expected 4 lines, found ${lines.length}`);
  if (!lines[0].startsWith("untrusted comment:")) fail("missing untrusted comment");
  const body = strictBase64(lines[1]);
  if (!body || body.length !== 74 || body[0] !== 0x45 || (body[1] !== 0x64 && body[1] !== 0x44)) fail("bad signature line");
  if (!lines[2].startsWith("trusted comment:")) fail("missing trusted comment");
  const global = strictBase64(lines[3]);
  if (!global || global.length !== 64) fail("bad global signature");
}

export function buildManifest({ tag, repo, installer, signature, notes = "", pubDate = new Date() }) {
  if (!/^v/.test(tag)) throw new Error(`tag "${tag}" must start with "v"`);
  const version = tag.slice(1);
  if (!SEMVER.test(version)) throw new Error(`tag "${tag}" is not v<semver>`);
  if (!REPO.test(repo)) throw new Error(`repository "${repo}" must be owner/name`);
  if (!ASSET.test(installer)) throw new Error(`installer name "${installer}" has unexpected characters`);
  const sig = signature.trim();
  checkSignature(sig);
  return {
    version,
    notes: notes.trim(),
    pub_date: pubDate.toISOString().replace(/\.\d{3}Z$/, "Z"),
    platforms: {
      [PLATFORM]: {
        signature: sig,
        url: `https://github.com/${repo}/releases/download/${tag}/${installer}`
      }
    }
  };
}

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i += 2) {
    if (!argv[i].startsWith("--") || argv[i + 1] === undefined) throw new Error(`unexpected argument ${argv[i]}`);
    args[argv[i].slice(2)] = argv[i + 1];
  }
  return args;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const a = parseArgs(process.argv.slice(2));
    for (const key of ["tag", "repo", "installer", "signature", "out"]) {
      if (!a[key]) throw new Error(`missing --${key}`);
    }
    const manifest = buildManifest({
      tag: a.tag,
      repo: a.repo,
      installer: a.installer,
      signature: readFileSync(a.signature, "utf8"),
      notes: a.notes ? readFileSync(a.notes, "utf8") : ""
    });
    writeFileSync(a.out, `${JSON.stringify(manifest, null, 2)}\n`);
    console.log(`${a.out}: ${manifest.version} -> ${manifest.platforms[PLATFORM].url}`);
  } catch (error) {
    console.error(`updater-manifest: ${error.message}`);
    process.exit(1);
  }
}
