import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["test/**/*.test.ts"],
    environment: "node",
    // The driver tests run real git and bash processes, which are slow to start on Windows.
    testTimeout: 60_000
  }
});
