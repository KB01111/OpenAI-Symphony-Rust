import { readFileSync } from "node:fs";
import { defineConfig } from "vitest/config";

export default defineConfig({
  // `.sh` files are imported as their text, as the bundler (scripts/build.mjs) does.
  plugins: [
    {
      name: "sh-as-text",
      enforce: "pre",
      load(id) {
        if (id.endsWith(".sh")) return `export default ${JSON.stringify(readFileSync(id, "utf8"))};`;
      },
    },
  ],
  test: {
    include: ["test/**/*.test.ts"],
    environment: "node",
    // The driver tests run real git and bash processes, which are slow to start on Windows.
    testTimeout: 60_000
  }
});
