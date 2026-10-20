/**
 * Shared contracts for every Symphony client.
 *
 * All API payload types come from `./generated/contracts.ts`, which is generated from the Rust
 * types by `crates/symphony-server/tests/ts_contract.rs` and checked in CI. Do not hand-write
 * API shapes here; add helpers that derive from the generated contracts instead.
 */
export * from "./generated/contracts.js";
export * from "./i18n/index.js";
export * from "./status.js";
