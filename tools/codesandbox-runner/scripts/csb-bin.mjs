import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * Absolute path of the CodeSandbox CLI script (`csb`) installed with `@codesandbox/sdk`.
 *
 * It is run with `node <this path>`: no shell is involved, so arguments with spaces reach the CLI as
 * one argument each and nothing in them can be interpreted by `cmd.exe` or `sh`, on any platform
 * (the npm `.bin/csb` shim on Windows is a `.cmd` file that can only be started through a shell).
 */
export function resolveCsbBin() {
  let dir = dirname(fileURLToPath(import.meta.resolve("@codesandbox/sdk")));
  for (let depth = 0; depth < 10; depth += 1) {
    const manifest = join(dir, "package.json");
    if (existsSync(manifest)) {
      const pkg = JSON.parse(readFileSync(manifest, "utf8"));
      if (pkg.name === "@codesandbox/sdk") {
        const bin = typeof pkg.bin === "string" ? pkg.bin : pkg.bin?.csb;
        if (!bin) throw new Error("@codesandbox/sdk does not declare a `csb` binary");
        return join(dir, bin);
      }
    }
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  throw new Error("Could not find the @codesandbox/sdk CLI; run `pnpm install`.");
}
