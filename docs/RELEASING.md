# Releasing Symphony Desktop

Everything between a tagged commit and a published release, what is automated, and what only the repository
owner can do. The release **gate list and current status** are in `docs/PRODUCTION_RELEASE_REPORT.md`.

## Pipeline (`.github/workflows/release.yml`)

Trigger: push a `v<version>` tag (or *Run workflow* with an existing tag). The workflow:

1. checks that the tag, every manifest version and a **dated** `CHANGELOG.md` section agree
   (`scripts/release-version.mjs`);
2. runs the Rust tests (including the Windows-only native tests) and the frontend tests;
3. optionally prepares **Authenticode** signing (below);
4. builds the desktop app and the **NSIS** installer (`tauri build --bundles nsis`) and the CLI;
5. runs the native Windows smoke test against the release binaries (`scripts/windows-native-smoke.ps1`);
6. stages the assets, CycloneDX SBOMs (`sbom-npm`, `sbom-symphony-desktop`, `sbom-symphony-cli`),
   `BUILD-INFO.txt` (version, tag, **source commit**, run URL, signed flag) and `SHA256SUMS.txt` over every
   file;
7. creates or updates a **draft** GitHub release. Publishing is always a manual click.

Unsigned builds carry `UNSIGNED` in their file names and a warning at the top of the release notes. They are
for verification only and must not be published.

## What a maintainer must do before the first public release

1. **Authenticode certificate** (`REL-03`): add the repository secrets `WINDOWS_CERTIFICATE` (base64 of the
   `.pfx`) and `WINDOWS_CERTIFICATE_PASSWORD`. Never commit a key or certificate.
2. **Date the changelog** (`## [2.0.0] - YYYY-MM-DD`), merge, tag `v2.0.0`, let the workflow finish.
3. **Run the manual Windows 11 checklist** (`docs/WINDOWS_NATIVE_SMOKE_TEST.md`) with the *drafted*
   installer, including install → upgrade → uninstall and "data survives".
4. Confirm `SHA256SUMS.txt`, the SBOMs and `BUILD-INFO.txt` match the commit, then publish the draft.
5. Branch protection on `main` for the checks in `ci.yml`, `desktop-native.yml`, `api-contract.yml`,
   `ui-e2e.yml`, `supply-chain.yml`.

## Auto-update (not implemented: needs owner decisions)

Tauri 2 updates through `tauri-plugin-updater`. **Update signing is a different mechanism from Authenticode**:
the updater verifies a minisign-style signature of each update artifact against a public key compiled into
the app; Authenticode is what Windows/SmartScreen checks on the installer. Both are needed.

What is required and cannot be created from this repository:

- An update **key pair** (`pnpm tauri signer generate`), created outside the repository. The public key goes
  into `tauri.conf.json` (`plugins.updater.pubkey`), the private key and its password into the secrets
  `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Losing the private key means installed
  apps can never be updated again; leaking it lets anyone ship an update.
- A **hosted HTTPS manifest** (`latest.json`: version, notes, per-platform URL and signature), e.g. GitHub
  release assets behind a stable URL, and a decision on stable vs. prerelease channels (two manifests).

Integration plan once those exist (each step needs a Windows build to verify, which is why none of it is
merged unverified):

1. `bundle.createUpdaterArtifacts: true` (only when the signing secret is present, through a `--config`
   override like the Authenticode one) so the NSIS build emits the installer's `.sig`.
2. Add `tauri-plugin-updater` and `tauri-plugin-process` (restart), grant only `updater:default` in
   `capabilities/default.json`, `endpoints` over HTTPS only.
3. A Settings → Application "Check for updates" action (opt-in; the app makes no network call on its own),
   showing version, notes and download progress.
4. **Running agents**: refuse to install while tasks are `Running` unless the user confirms; the installer
   restarts the app and the Job Object ends agent trees, so unfinished attempts are re-queued by crash
   recovery. Data directory and vault are untouched by an update.
5. Release workflow: upload `latest.json` and the `.sig` files, include them in `SHA256SUMS.txt`.
6. Test with a throw-away key pair against a local HTTPS server before the first real key is created.

## Verifying a download

`Get-FileHash <file> -Algorithm SHA256` against `SHA256SUMS.txt`; `Get-AuthenticodeSignature <exe>` must say
`Valid` for a published release; `BUILD-INFO.txt` names the commit the assets were built from.
