# Releasing Symphony Desktop

Everything between a merged pull request and a published release, what is automated, and what only the repository
owner can do. The release **gate list and current status** are in `docs/PRODUCTION_RELEASE_REPORT.md`.

## Automatic releases (every merged pull request)

`.github/workflows/auto-release.yml` runs on every push to `main` and releases each merged pull request:

1. It finds the pull request that brought the commit into `main`. A direct push without a pull request is not released.
2. It chooses the version (`scripts/next-version.mjs`): a **patch** bump of the newest release, or a **minor** / **major**
   bump when the pull request carries the label `release:minor` / `release:major`. A pull request that sets a new version
   in the manifests on purpose is released as that version.
3. It writes the version into every manifest (`scripts/set-version.mjs`: the Cargo workspace, every `package.json`,
   `tauri.conf.json` and the workspace crates in `Cargo.lock`), commits it to `main` as `Release vX.Y.Z` and tags
   `vX.Y.Z`. The first release keeps the version the manifests already declare (`2.0.0`).
4. It calls `release.yml`, which builds, tests, smoke-tests and **publishes** the installer as the latest GitHub release.
   The release notes are GitHub's generated notes from the merged pull requests.

The version commit and the tag are pushed with the workflow's own token, which does not start new workflow runs, so
nothing loops. Releases run one at a time, in merge order. Installed apps are offered each new release by the in-app
updater (below).

Put the label on the pull request **before** merging it. Create the two labels once (*Issues → Labels*, or
`gh label create "release:minor"` and `gh label create "release:major"`).

## Pipeline (`.github/workflows/release.yml`)

Trigger: called by `auto-release.yml`, a pushed `v<version>` tag, or *Run workflow* with an existing tag. The workflow:

1. checks that the tag and every manifest version agree (`scripts/release-version.mjs`). A tag or manual release also
   needs a **dated** `CHANGELOG.md` section for its version; an automatic release takes its notes from GitHub instead;
2. runs the Rust tests (including the Windows-only native tests) and the frontend tests;
3. optionally prepares **Authenticode** signing (below);
4. builds the desktop app and the **NSIS** installer (`tauri build --bundles nsis`) and the CLI;
5. runs the native Windows smoke test against the release binaries (`scripts/windows-native-smoke.ps1`);
6. stages the assets, CycloneDX SBOMs (`sbom-npm`, `sbom-symphony-desktop`, `sbom-symphony-cli`),
   `BUILD-INFO.txt` (version, tag, **source commit**, run URL, signed flag), `SHA256SUMS.txt` over every file and,
   with the update key, `latest.json`;
7. creates the GitHub release. A pushed tag leaves it as a **draft** (a maintainer presses *Publish*). An automatic
   release, or a manual run with `publish` ticked, publishes it: a stable version becomes the **latest** release, a
   pre-release version (one with a `-`) is marked as a pre-release.

Unsigned builds (no Authenticode certificate) carry `UNSIGNED` in their file names and a warning at the top of the release
notes. Publishing them needs `allow_unsigned`, which automatic releases always pass. Without a certificate Windows
SmartScreen warns when the installer runs. In-app updates do not depend on Authenticode: they are verified with the update
key.

Every commit on `main` and every pull request also builds an unsigned installer and runs the same installer smoke test
(`desktop-native.yml`); that installer is downloadable from the run for 7 days.

## Manual releases

Every merge to `main` is released automatically, so there is no separate step for a release on `main`. Two manual
routes remain:

- **Draft from a tag**: pushing `v<version>` (for example `git push origin v2.1.0`) builds the release and leaves it as a
  **draft** for review. The tag must match the manifests (`node scripts/set-version.mjs 2.1.0` writes them) and the
  `CHANGELOG.md` section for that version must be dated. The workflow runs the **installer smoke test** (silent install →
  installed app passes the native smoke test → upgrade → uninstall → user data survives) and attaches the installer, the
  CLI, SBOMs, `BUILD-INFO.txt` and `SHA256SUMS.txt`. Press **Publish** under *Releases* once reviewed. Do not tag a
  version that an automatic release already published; the tag already exists.
- **Re-run or publish an existing tag**: *Actions → Release (Windows) → Run workflow* with the tag. Leave `publish`
  unticked to keep the draft, or tick it to publish when the build passes.

## What the owner must set up

1. **Update key** (needed for in-app updates; details below): the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Without them the release is still published, but without `latest.json`, so
   installed apps send their users to the releases page instead of updating.
2. **Authenticode certificate** (optional; removes the SmartScreen warning): the secrets `WINDOWS_CERTIFICATE` (base64 of a
   `.pfx`) and `WINDOWS_CERTIFICATE_PASSWORD`. Never commit a key or certificate.
3. **Branch protection on `main`** for the checks in `ci.yml`, `desktop-native.yml`, `api-contract.yml`, `ui-e2e.yml`
   and `supply-chain.yml`. The release commit is pushed by the workflow, so if protection forbids direct pushes to
   `main`, let the GitHub Actions bot bypass it. Otherwise the automatic release stops with
   "could not push the version commit to main".
4. Run the manual Windows 11 checklist (`docs/WINDOWS_NATIVE_SMOKE_TEST.md`) once with an installed release, including
   install → upgrade → uninstall and "data survives".
5. Decide who may merge to `main`. Every merge now publishes a release that is signed for the in-app updater. If each
   release should wait for a human, put the two update-key secrets in a `release` environment with required reviewers
   and add `environment: release` to the `release` job in `auto-release.yml`.

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

### The update key (owner)

Update signing is a **different mechanism from Authenticode**: Windows/SmartScreen checks Authenticode on the
installer, the app checks the update signature. The pair was created with `pnpm tauri signer generate` and replaced the
key that was committed earlier before any release had been published:

- the **public key** is committed in `apps/desktop/src-tauri/tauri.conf.json` and compiled into every build;
- the **private key** and its password are never committed. They live in the repository secrets
  `TAURI_SIGNING_PRIVATE_KEY` (the *contents* of the key file) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
  (*Settings → Secrets and variables → Actions*). Keep a backup of both outside the repository.

Losing the private key or its password means installed apps can never be updated again (users would have to install
by hand once, with a new public key); leaking the private key lets anyone ship an update. To rotate, generate a new
pair, replace `pubkey`, ship a release signed with the **old** key that carries the new public key, then switch the
secrets.

What the workflow does when the secret exists: after the installer is built (and smoke-tested), a separate step
runs `tauri signer sign` on it, so the key is visible to that step only and never to compilers, build scripts or
the frontend toolchain; `scripts/updater-manifest.mjs` then checks the signature's structure and writes
`latest.json` (version, notes, installer URL, signature); both are uploaded and covered by `SHA256SUMS.txt`.
Releases built UNSIGNED (Authenticode) are published like the others: the update signature does not depend on
Authenticode.

Anyone who can push a tag or edit the workflow on a branch can run a workflow that reads repository secrets, and
since merges to `main` now release, anyone who can merge there does too. If more than one person can merge, use the
*environment* described in *What the owner must set up* and restrict it to `main`.

What protects an installed app from a doctored manifest: the installer must carry a valid signature for the
compiled-in key, the manifest's version must match the release the GitHub API announced, the installer URL must
live in that same release's download path, and the version must be newer than the running one.

Not enforced yet: `require_signed_version` (the updater would also check that the signature names the same
version as the manifest, which blocks downgrade tricks through a forged manifest). The Tauri CLI used here
does not record the version in the signature; enable it once it does. The downgrade guard that exists today is
the updater's own "must be newer" comparison over HTTPS from github.com.

## Verifying a download

`Get-FileHash <file> -Algorithm SHA256` against `SHA256SUMS.txt`; `Get-AuthenticodeSignature <exe>` must say
`Valid` for a release signed with the certificate (unsigned releases say so in their notes); `BUILD-INFO.txt` names the
commit the assets were built from.
