# Contributing to Symphony Desktop

Thank you for contributing to **Symphony Desktop 1.0**. This guide covers the development workflow, coding conventions, and verification checks required before submitting a Pull Request.

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
cargo clippy --workspace --locked -- -D warnings
cargo test --workspace --locked
```

### 2. Frontend TypeScript Typechecking, Tests, and Production Builds

```powershell
pnpm typecheck
pnpm test
pnpm build
```

---

## Architectural Conventions

- **No Authoritative State in UI**: Keep all task scheduling, retry backoff, and state transitions inside `crates/symphony-orchestrator` and `crates/symphony-core`.
- **Use Authentic Design System Components**:
  - Use `@fluentui/react-components` for application shell, navigation, dialogs, and settings forms.
  - Use `@openai/apps-sdk-ui` (`Alert`, `Badge`, `Button`, `CodeBlock`, `Markdown`, `Textarea`, `Tooltip`, `Popover`, `EmptyMessage`, `SegmentedControl`, `ShimmerText`) for AI-facing surfaces.
- **Security**:
  - Never pass raw tracker tokens (`GITHUB_TOKEN`, `LINEAR_API_KEY`) into coding agent child processes.
  - Always validate workspace paths via `symphony_workspaces::validate_workspace_path`.
- **Localization**:
  - Add any new UI strings to both `en` and `sv` dictionaries in `packages/shared-types/src/i18n.ts`.
