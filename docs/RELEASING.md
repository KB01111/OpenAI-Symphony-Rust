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

## Cutting a release, step by step

1. Put the date on the version's `CHANGELOG.md` section (`## [2.0.0] - YYYY-MM-DD`) and make sure every
   manifest carries the same version (`node scripts/release-version.mjs --tag v2.0.0`).
2. Merge to `main`, then tag and push: `git tag v2.0.0 && git push origin v2.0.0`. The workflow builds,
   tests, runs the **installer smoke test** (silent install → installed app passes the native smoke test →
   upgrade → uninstall → user data survives) and drafts the release with the installer, the CLI, SBOMs,
   `BUILD-INFO.txt` and `SHA256SUMS.txt`.
3. Review the draft under *Releases* and press **Publish** — or skip the click: *Actions → Release (Windows) →
   Run workflow* with the tag and `publish` ticked publishes it when the build passes.
4. Without the signing secrets the assets are `UNSIGNED`. Publishing them needs `allow_unsigned` as well
   and produces a **pre-release** that keeps the SmartScreen warning in its notes. Add the secrets (next
   section) for a normal, "latest" release.

Every commit on `main` and every pull request also builds an unsigned installer and runs the same installer
smoke test (`desktop-native.yml`); the installer is downloadable from that run for 7 days.

## What a maintainer must do before the first public release

1. **Authenticode certificate** (`REL-03`): add the repository secrets `WINDOWS_CERTIFICATE` (base64 of the
   `.pfx`) and `WINDOWS_CERTIFICATE_PASSWORD`. Never commit a key or certificate.
2. **Date the changelog** (`## [2.0.0] - YYYY-MM-DD`), merge, tag `v2.0.0`, let the workflow finish.
3. **Run the manual Windows 11 checklist** (`docs/WINDOWS_NATIVE_SMOKE_TEST.md`) with the *drafted*
   installer, including install → upgrade → uninstall and "data survives".
4. Confirm `SHA256SUMS.txt`, the SBOMs and `BUILD-INFO.txt` match the commit, then publish the draft.
5. Branch protection on `main` for the checks in `ci.yml`, `desktop-native.yml`, `api-contract.yml`,
   `ui-e2e.yml`, `supply-chain.yml`.

## In-app updates

*Settings → Application → Updates* (and a banner at the top of the window) offer the newest release:

1. The host asks the GitHub releases API for the newest release that suits the running build
   (pre-releases are only offered to pre-release builds). This runs at startup and every six hours unless the
   user turns *Check for updates automatically* off; the button always works.
2. If that release has a `latest.json` asset, the Tauri updater reads it from
   `releases/download/<tag>/latest.json`, and **Update now** downloads the installer it names, verifies its
   minisign signature against the public key in `tauri.conf.json` (`plugins.updater.pubkey`) and only then
   runs it (NSIS `passive` mode: progress bar, no questions). The installer replaces the app and starts it again.
   Data directory and credential vault are untouched.
3. If tasks are running, the app asks first: the installer ends the app and its agent processes, and unfinished
   attempts are re-queued by crash recovery when it starts again.
4. A release without `latest.json` (built before the key was configured, or a rebuilt draft) is still announced;
   the button then opens the releases page.

The webview supplies no URL or path to any of this: `check_for_updates` and `install_update` take at most a
`confirmed` flag, and the plugin's own commands are not granted to the window.

### The update key (one time, owner)

Update signing is a **different mechanism from Authenticode**: Windows/SmartScreen checks Authenticode on the
installer, the app checks the update signature. The pair was created with `pnpm tauri signer generate`:

- the **public key** is committed in `apps/desktop/src-tauri/tauri.conf.json` and compiled into every build;
- the **private key** and its password are never committed. Add them as the repository secrets
  `TAURI_SIGNING_PRIVATE_KEY` (the *contents* of the key file) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
  (*Settings → Secrets and variables → Actions*).

Without the secrets the release workflow still builds and publishes, but without `latest.json`. Losing the
private key means installed apps can never be updated again (users would have to install by hand once, with a
new public key); leaking it lets anyone ship an update. To rotate, generate a new pair, replace `pubkey`, ship
a release signed with the **old** key that carries the new public key, then switch the secrets.

What the workflow does when the secret exists: after the installer is built (and smoke-tested), a separate step
runs `tauri signer sign` on it, so the key is visible to that step only and never to compilers, build scripts or
the frontend toolchain; `scripts/updater-manifest.mjs` then checks the signature's structure and writes
`latest.json` (version, notes, installer URL, signature); both are uploaded and covered by `SHA256SUMS.txt`.
Releases built UNSIGNED (Authenticode) are pre-releases, so only pre-release builds are offered them.

Anyone who can push a tag or edit the workflow on a branch can run a workflow that reads repository secrets. If
more than one person has write access, create a GitHub *environment* (e.g. `release`) restricted to `v*` tags
and required reviewers, store the two secrets there and add `environment: release` to the release job.

What protects an installed app from a doctored manifest: the installer must carry a valid signature for the
compiled-in key, the manifest's version must match the release the GitHub API announced, the installer URL must
live in that same release's download path, and the version must be newer than the running one.

The signature must also name the manifest's version: the workflow signs with
`tauri signer sign --app-version <version>`, which records the version in the signature's trusted comment, and
`plugins.updater.requireSignedVersion` makes the app compare it with the version the manifest announces. A
forged manifest therefore cannot pair a newer version number with an older, genuinely signed installer.
`scripts/updater-manifest.mjs` refuses to write `latest.json` for a signature that carries no version or a
different one, and a unit test keeps the setting in `tauri.conf.json`.

Consequence: a release signed *without* `--app-version` is rejected by builds that have this setting (the user
is sent to the releases page instead). Every release the app should update to must be cut by the current
workflow; apps installed before this setting existed are unaffected and update normally.

## Verifying a download

`Get-FileHash <file> -Algorithm SHA256` against `SHA256SUMS.txt`; `Get-AuthenticodeSignature <exe>` must say
`Valid` for a published release; `BUILD-INFO.txt` names the commit the assets were built from.
