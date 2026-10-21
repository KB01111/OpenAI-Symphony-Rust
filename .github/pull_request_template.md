## Summary

- Describe the changes introduced in this Pull Request.

## Verification Evidence

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo test --workspace --locked`
- [ ] `pnpm generate:api` (no diff)
- [ ] `pnpm typecheck`
- [ ] `pnpm test`
- [ ] `pnpm build`
- [ ] `pnpm --filter @symphony/desktop e2e`
- [ ] `CHANGELOG.md` updated for user-visible changes
- [ ] API change: `api-breaking-change` label and `API_VERSION` bump if breaking

## Merge Readiness (1–5)

- **Score:** 
- **Notes / Remaining Platform Limitations:** (say which layer verified native Windows behaviour: Rust tests, native smoke test, or manual)
