#!/usr/bin/env node
/**
 * Bundles the bridge into dist/bridge.mjs (one file: Symphony runs it with `node`, without a
 * node_modules next to it).
 */
import { build } from "esbuild";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

await build({
  entryPoints: [join(root, "src", "main.ts")],
  outfile: join(root, "dist", "bridge.mjs"),
  bundle: true,
  platform: "node",
  format: "esm",
  target: "node22",
  // The SDK's CommonJS dependencies call require(); give the ESM bundle one.
  banner: { js: 'import { createRequire as __createRequire } from "node:module"; const require = __createRequire(import.meta.url);' },
  legalComments: "none",
  logLevel: "info"
});
