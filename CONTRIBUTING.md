# Contributing to Symphony Desktop

Thank you for contributing to **Symphony Desktop 2.0**. This guide covers the development workflow, coding conventions, and verification checks required before submitting a Pull Request.

---

## Development Environment Setup (Windows 11 PowerShell)

### Toolchain prerequisites

| Tool | Version | Notes |
| --- | --- | --- |
| Node.js | `>= 22.13.0` | Hard requirement of pnpm 11 (`engines.node` in `package.json`). |
| pnpm | `11.15.1` | Pinned by `packageManager` in the root `package.json`; this is the single source of truth used by CI. |
| Rust | stable | Installed by `dtolnay/rust-toolchain@stable` in CI; `rustfmt` and `clippy` components are required. |

If you use Corepack, run `corepack enable` once and the pinned pnpm version is applied automatically. Prefer the pinned version instead of installing another pnpm major locally: `pnpm-lock.yaml` and the `allowBuilds` build-script policy in `pnpm-workspace.yaml` are both maintained with pnpm 11 (pnpm 11 removed `onlyBuiltDependencies` in favour of `allowBuilds`).

1. **Clone the repository**:
   ```powershell
   git clone https://github.com/KB01111/OpenAI-Symphony-Rust.git
   cd OpenAI-Symphony-Rust
   ```
2. **Install Node.js workspace dependencies** (use `--frozen-lockfile` to reproduce CI exactly):
   ```powershell
   pnpm install --frozen-lockfile
   ```
3. **Verify the Rust workspace compiles**:
   ```powershell
   cargo check --workspace
   ```

---

## Verification Checklist

Every pull request must pass the full verification suite cleanly before review:

### 1. Rust Formatting, Linting, and Tests

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

### 2. API Contract, Frontend Typechecking, Tests, and Production Builds

```powershell
pnpm install --frozen-lockfile
pnpm generate:api          # must leave no diff; run it twice to confirm determinism
pnpm typecheck
pnpm test                  # workspace Vitest suites + scripts/*.test.mjs (API versioning policy)
pnpm build
pnpm --filter @symphony/desktop e2e
```

`pnpm test` includes the API client integration suites, which start a real `symphony-cli`; rebuild it
(`cargo build -p symphony-cli`) after changing Rust code so they do not run against a stale binary.

---

## API Contract Lifecycle

The Axum route table in `crates/symphony-server` is the only source of truth for the HTTP API:

1. Every handler carries a `#[utoipa::path]` annotation; `utoipa-axum` builds `openapi/symphony-api.json`
   from the same routes that serve requests. Hey API generates the TypeScript types, SDK and fetch client in
   `packages/shared-types/src/generated/` from it. Never edit generated files by hand.
2. After changing a route or an API type, run `pnpm generate:api` and commit the result. Generation uses
   the committed `Cargo.lock` (`cargo test --locked`) and exactly pinned generator versions, and writes LF
   line endings on every OS (`.gitattributes`), so it is deterministic on Windows, Linux and macOS.
3. CI (`.github/workflows/api-contract.yml`) fails when the committed artifacts are stale or a second
   regeneration changes them, and applies the versioning policy (`scripts/api-contract-policy.mjs`):
   - a **breaking** change (any `oasdiff breaking` finding at level ERR) requires **both** the
     `api-breaking-change` label **and** `API_VERSION` (in `crates/symphony-server/src/lib.rs`) raised above
     the target branch's value — either one alone fails;
   - `API_VERSION` never decreases;
   - a non-breaking change needs neither (bumping is allowed and reported as a notice).
   The policy job runs the base branch's copy of the script, so a pull request cannot relax its own check.
4. Every operation is classified for the remote companion listener (`x-symphony-remote` in the document,
   `REMOTE_OPERATIONS` in `crates/symphony-server/src/remote.rs`). New endpoints are local-only unless you add
   them to that list deliberately; `tests/remote_access.rs` fails on an unclassified operation.

---

## Native Windows vs. Browser Testing

| Layer | What runs it | What it proves |
| --- | --- | --- |
| Rust unit/integration tests | `cargo test --workspace` on `windows-latest` (ci.yml) and Linux | Vault (DPAPI on Windows), owner-only DACLs in world-writable directories, HKCU autostart round-trip (`crates/symphony-platform/tests/windows_native.rs`), CLI runtime lifecycle (`crates/symphony-cli/tests/runtime_lifecycle.rs`), remote access (`crates/symphony-server/tests/remote_access.rs`) |
| Native smoke test | `scripts/windows-native-smoke.ps1` in desktop-native.yml and release.yml, against the release `symphony-desktop.exe` and `symphony-cli.exe` | Runtime start, DACLs, DPAPI persistence across restart, owner-only token-free desktop log, single instance / single scheduler, crash recovery without orphans, attach mode |
| Browser E2E | Playwright (`pnpm --filter @symphony/desktop e2e`) against the Vite dev server and a real `symphony-cli` | UI flows and contracts; **not** native behaviour |
| Manual | [`docs/WINDOWS_NATIVE_SMOKE_TEST.md`](./docs/WINDOWS_NATIVE_SMOKE_TEST.md) | Tray, folder picker, notifications, window-state restore, autostart at sign-in, unsigned installer |

A successful Tauri build alone proves compilation, not runtime behaviour; say which layer verified a claim.

---

## Dependencies and Supply Chain

- `cargo deny check` (policy in `deny.toml`) and `pnpm audit --audit-level moderate` run in
  `.github/workflows/supply-chain.yml`. A new license needs a review before it is added to `deny.toml`;
  an advisory may only be ignored there with the reason it does not apply and when to remove the entry.
- Prefer upgrading a parent package. When a parent pins a vulnerable version exactly, add a
  version-scoped override to `pnpm-workspace.yaml` with a comment naming the advisory and the parent.
- New workflow steps use actions pinned to a full commit SHA (with the tag as a comment) and the least
  token permissions they need.

## Releasing

1. Bump the version in every manifest (root `Cargo.toml` `[workspace.package]`, every `package.json`,
   `apps/desktop/src-tauri/tauri.conf.json`), run `cargo check --workspace` to refresh `Cargo.lock`, and
   replace `Unreleased` in the `CHANGELOG.md` heading with the release date (`## [X.Y.Z] - YYYY-MM-DD`).
2. `node scripts/release-version.mjs --tag vX.Y.Z --notes notes.md` must pass; merge the change to `main`.
3. Tag the merge commit `vX.Y.Z` and push the tag. `.github/workflows/release.yml` re-checks the version,
   runs the Rust and frontend tests, builds the NSIS installer and CLI, signs them when the
   `WINDOWS_CERTIFICATE` / `WINDOWS_CERTIFICATE_PASSWORD` secrets are set, runs the native smoke test,
   writes CycloneDX SBOMs, `BUILD-INFO.txt` (source commit) and `SHA256SUMS.txt` and creates a **draft**
   release with the changelog section as notes (see [`docs/RELEASING.md`](./docs/RELEASING.md)).
4. Run the manual checklist in [`docs/WINDOWS_NATIVE_SMOKE_TEST.md`](./docs/WINDOWS_NATIVE_SMOKE_TEST.md)
   with the drafted installer, then publish the draft. Never publish assets named `UNSIGNED`.

---

## Architectural Conventions

- **No Authoritative State in UI**: Keep all task scheduling, retry backoff, and state transitions inside `crates/symphony-orchestrator` and `crates/symphony-core`.
- **Use Authentic Design System Components**:
  - Use the components exported by `@symphony/ui` (shadcn/ui on Radix: `Button`, `Field`, `Input`, `Select`, `Dialog`, `Sheet`, `Tabs`, `Alert`, `Badge`, `ToggleGroup`, …) for every desktop surface, including AI-facing ones (`AiApprovalBanner`, `AiEventStreamCard`, `Markdown`, `CodeBlock`).
  - Size layouts from their container (`@container` + `@md:`), never from the viewport, so forms and tables fit side sheets and resizable panes.
- **Database changes** are forward-only migrations: add `crates/symphony-storage/migrations/NNNN_name.sql`,
  list it in `MIGRATIONS` and raise `SCHEMA_VERSION`; never edit an applied migration. Add a test that opens a
  database in the previous schema (see `crates/symphony-storage/tests/migrations.rs`) and shows nothing is lost.
- **Security**:
  - Agent tools (`github_api`, `linear_graphql`) are an allowlist (`crates/symphony-trackers/src/tool_policy.rs`);
    do not widen it without tests. There is deliberately no merge operation.
  - Never pass raw tracker tokens (`GITHUB_TOKEN`, `LINEAR_API_KEY`) into coding agent child processes.
  - Always validate workspace paths via `symphony_workspaces::validate_workspace_path`.
  - Store secrets only through `SecretStore` and write secret-bearing files only through
    `symphony_platform::secure_fs` (owner-only creation, verification, atomic replace).
  - Never expose a new endpoint to the remote companion without adding it to `REMOTE_OPERATIONS` on purpose.
- **Localization**:
  - Add every new UI string to both the `en` and `sv` dictionaries: feature strings live in the feature's
    `messages.ts` (e.g. `apps/desktop/src/features/settings/messages.ts`, `apps/mobile/src/messages.ts`),
    shared strings in `packages/shared-types/src/i18n/index.ts`.
