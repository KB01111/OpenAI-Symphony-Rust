import { defineConfig } from "@hey-api/openapi-ts";

/**
 * Generates the typed HTTP contract from the Rust OpenAPI document (`openapi/symphony-api.json`,
 * produced by `crates/symphony-server/tests/openapi_contract.rs`) into `@symphony/shared-types`.
 * Never hand-edit the output.
 */
export default defineConfig({
  input: "../../openapi/symphony-api.json",
  output: {
    path: "../../packages/shared-types/src/generated/openapi",
    // Keep `.js` import specifiers like the rest of the ESM workspace.
    importFileExtension: ".js"
  },
  plugins: ["@hey-api/typescript", { name: "@hey-api/sdk" }, { name: "@hey-api/client-fetch" }]
});
