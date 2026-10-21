/**
 * KaTeX 0.18 renamed layout classes (`base`, `strut`, `sizing` → `katex-*`) that the stylesheet bundled
 * in @openai/apps-sdk-ui 0.2 still targets. While the workspace forces KaTeX >= 0.18 (the security
 * override in pnpm-workspace.yaml), every app must load KaTeX's own stylesheet after the SDK's, or
 * rendered math loses its layout (`node --test scripts/`).
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const apps = ["desktop", "mobile"];

function installedKatexVersion(app) {
  const pkg = join(root, "apps", app, "node_modules", "katex", "package.json");
  return JSON.parse(readFileSync(pkg, "utf8")).version;
}

for (const app of apps) {
  test(`${app} loads KaTeX's own stylesheet after the Apps SDK stylesheet`, () => {
    const [major, minor] = installedKatexVersion(app).split(".").map(Number);
    if (major === 0 && minor < 18) return; // The SDK's bundled stylesheet still matches.

    const css = readFileSync(join(root, "apps", app, "src", "styles", "globals.css"), "utf8");
    const sdk = css.indexOf('@import "@openai/apps-sdk-ui/css";');
    const katex = css.indexOf('@import "katex/dist/katex.min.css" layer(base);');
    assert.notEqual(sdk, -1, "the Apps SDK stylesheet import is missing");
    assert.notEqual(katex, -1, "KaTeX >= 0.18 needs its own stylesheet in layer(base)");
    assert.ok(katex > sdk, "KaTeX's stylesheet must come after the Apps SDK stylesheet");
  });
}
