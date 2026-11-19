# Build and publication contract

## Version and license

The current version **0.3.4** is authoritative in `package.json`.
No compatibility or stable-release claim is added by retaining it.

```bash
npm run version:set -- 0.3.4
npm run version:check
```

The bump command validates stable `MAJOR.MINOR.PATCH` components from 0 to
65535, reports all affected files, and updates `package.json`,
`package-lock.json`, `src-tauri/Cargo.toml`, and the application's entry in
`src-tauri/Cargo.lock`. Repeating it is a no-op. Prerelease/build suffixes are
not supported by this native release command. It creates no commit or tag.
Tauri, About, and landing-page downloads derive the package version.

Project licensing is GNU GPL version 3 only (`GPL-3.0-only`). Native bundles
carry the complete authoritative LICENSE. Public binary distribution must
include the corresponding source for the exact revision, build inputs and
applicable third-party notices. Final dependency-license and notice approval remains
an owner publication gate; changing the project license does not remove
third-party obligations.

## Proposed native matrix

| Target              | Native build host | Candidate files                                             | Runtime acceptance                                    |
| ------------------- | ----------------- | ----------------------------------------------------------- | ----------------------------------------------------- |
| Linux x64           | Ubuntu 24.04      | `Graphite_0.3.4_amd64.deb`, `Graphite_0.3.4_amd64.AppImage` | Current package build and installed lifecycle pending |
| macOS Apple Silicon | macOS 15          | `Graphite_0.3.4_aarch64.dmg`                                | Native build/install/launch unverified here           |

These are candidate targets, not a claim of completed platform acceptance.
Linux binaries inherit the build host's glibc/system-library constraints;
do not advertise older distributions without native verification. Tauri
uses WebKitGTK 4.1 and GTK 3. DEB declares system dependencies; AppImage
still needs compatible system graphics, libc, and FUSE or extraction mode.

The Apple Silicon configuration uses macOS 11.0 as a metadata floor because
ARM Macs cannot run 10.15. This is not evidence that every native dependency
works on macOS 11. The interface requires Safari/WebKit 15.4 or newer for native dialogs.
The proposed tested baseline is macOS 15; minimum-runtime
acceptance on an older OS is unverified. The locked `ort-sys` distribution
contains an ARM Darwin CoreML archive and no Intel Darwin archive. Intel
macOS and universal packages are not advertised. Linux ARM is not part of
the existing release matrix. Windows is currently unavailable; we are working on
[support](WINDOWS.md).

## Reproduction and installation

Read `docs/development.md` for native prerequisites. Use Node.js 24.19.0,
npm 11.17.0, Rust 1.97.1, and the committed locks. Run:

```bash
npm ci
npm run package:check
npm run check:publication
npm run check:privacy
npm run package:build
npm run package:smoke -- linux-x64
```

On an Apple Silicon Mac the same build command creates a DMG; use
`npm run package:smoke -- macos-arm64`. Linux builds require the pinned CUDA
toolkit and include both GPU and CPU executables. macOS enables Metal. Auto
prefers a supported GPU with CPU fallback; saved user choices are preserved.
The manual workflow prepares its Linux toolkit with `npm run cuda:prepare`
in the project cache. Locally, reuse an existing toolkit as described in
[GPU packaging](../linux-cuda-package.md); no host driver changes are needed.
The wrapper remaps home and checkout paths in Rust and C/C++ compilation.
Package validation checks native dependencies, required resources, licenses,
installation and startup. Private-path and credential checks cover source and
packaged text resources; arbitrary strings inside compiled binaries are not a
release gate. See the [inspection boundary](SECURITY.md#boundary-checks).

For a clean disposable Linux host, install the DEB using the native package
manager, then start Graphite from the desktop menu. AppImage is portable:
mark it executable and run it; if FUSE is unavailable, explicitly test
`APPIMAGE_EXTRACT_AND_RUN=1`. On a disposable Mac, mount the DMG and copy
Graphite.app into Applications. macOS releases are unsigned and unnotarized
by design. A free local ad-hoc signature supports Apple Silicon; it does not
identify an Apple-verified publisher. No Apple account or paid certificate
is required. The build wrapper omits inherited Apple credentials and keeps
Metal enabled. See [the signing policy](../../CODE_SIGNING_POLICY.md).

After checking the release source and SHA-256, try opening Graphite. If
macOS blocks it as an unidentified developer and you trust this download,
open **System Settings > Privacy & Security > Open Anyway**, then confirm
the app-specific prompt. Follow [Apple's current instructions](https://support.apple.com/en-us/102445).
Do not disable Gatekeeper globally or remove quarantine recursively. A
damaged-app or malware warning needs investigation, not a blanket override.
Verify this first-launch journey with a browser-downloaded DMG on a clean
Mac; the hosted runner's ordinary launch test does not exercise quarantine.

## Workflow boundaries and owner actions

Build checks, native package jobs and static-site publication are separate
workflows. Actions are pinned to full revisions, artifacts are temporary,
and installers use versioned checksums. A version-tag push starts validation,
packaging and creation of an unpublished release draft. Public publication
remains manual. Ordinary CI and pull requests perform validation only.
The release requires application CI and both distributed platforms. Website
build/tests and Windows/Intel macOS compatibility checks run independently;
they do not block creation of the Linux/Apple Silicon draft.

`package-smoke.yml` runs on explicit `workflow_dispatch` or as a reusable
workflow called by `release.yml`. It validates
frontend/version metadata, builds native packages, inspects contents, and
runs the guarded install/remove/reinstall lifecycle. Upload names include
the version and platform. Candidate assembly requires both platforms,
rechecks every checksum, records `GITHUB_SHA`, and keeps `publicRelease`
and `signed` false. Package jobs have read-only repository permissions.

The optional `build_demo_archive` dispatch input defaults to false. When
selected, a separate Ubuntu job retrieves and verifies the reviewed public
PDFs, runs `npm run demo:archive`, and uploads
`graphite-demo-seismology-ml-v2` with seven-day retention. This 412 MB archive
is separate from installers and the aggregate candidate. Its content,
rights review, and offline import flow are described in [DEMO.md](DEMO.md).
The archive build needs Python 3's standard `zipfile` module; the installer
build does not gain a Python runtime dependency.

`pages.yml` builds pull-request previews and manual runs, including
`npm run test:site` and screenshot provenance checks. Deployment requires
an explicit manual `publish=true` input on main and the `github-pages`
environment. Only that deploy job has `pages: write` and `id-token: write`.
The owner must configure Pages to use GitHub Actions and review environment
approval rules. The build derives the project base path from the repository
name; local previews default to `/graphite/`. The public repository must be
chosen deliberately, because its URL becomes visible in page links.

### Build installers and create a release draft

1. Commit the intended public source, including the workflow files and both
   dependency lockfiles. Publication and privacy checks must pass first.
2. Set the release version with `npm run version:set -- 0.3.4`, review the
   versioned release docs, and commit the resulting files. Screenshot
   provenance is validated separately by the Pages workflow.
3. Create and push the matching version tag, for example `v0.3.4`. This starts
   **Actions > Release installers**. The tag must match all app manifests.
   To retry an existing tag, rerun its workflow or dispatch Release installers
   with that tag as the ref. A branch ref is deliberately rejected; use the
   package smoke workflow for branch-only builds.
4. The workflow runs application CI, then reuses the Linux/macOS package
   and installed-runtime checks. Both platforms must succeed before it
   assembles the candidate and creates a GitHub release draft.
5. Open the draft under **Releases**. It contains the DEB, AppImage, Apple
   Silicon DMG, `SHA256SUMS.txt`, and `release-manifest.json`. Review the
   release notes, corresponding source, third-party notices and native
   acceptance evidence before manually publishing it.

Only the final draft job has `contents: write`, using the temporary Actions
`GITHUB_TOKEN`; no personal credentials or Apple account are required. It
verifies the candidate version, source revision and checksums before upload.
Reruns may replace assets on an unpublished draft, but never overwrite a
published release. Do not move a released tag to another commit. The workflow
creates no tags and does not publish the draft or the website automatically.
Repository or organization Actions policy must allow the draft job's write
permission. See the [GitHub reusable workflow documentation](https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows)
and [draft release CLI options](https://cli.github.com/manual/gh_release_create).

### Run the macOS compilation workflow

1. Make the reviewed commits available in the GitHub repository. The
   workflow file must be present on its default branch for
   [manual dispatch](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow).
2. Open **Actions > Unsigned package and installed runtime smoke** and
   select **Run workflow** for the reviewed branch.
3. The macOS job uses the native Apple Silicon `macos-15` runner, checks
   Xcode/Clang and the SDK, activates `rust-toolchain.toml`, installs the
   pinned Node/npm dependencies and compiles `Graphite_0.3.4_aarch64.dmg`.
4. Download `graphite-0.3.4-unsigned-macos-arm64` from the run artifacts.
   It contains the DMG, checksums and build metadata. Artifacts remain
   available for seven days. No Apple signing credentials are needed.

A package that passes structural inspection is retained even if its
later install/launch check fails. Read the job result before treating
that artifact as accepted. The combined release candidate is uploaded
only when both platform jobs and the aggregate checks pass. macOS
compilation and acceptance require successful results from the native job
and the first-launch check above.

### Owner publication follow-up

Before publication, the owner must:

1. Create the fresh repository from reviewed current source files. Stage
   only the intended source, tests, workflows and public documentation, then
   run `npm run check:publication` and the [privacy checks](SECURITY.md).
   Ignore rules do not remove files already tracked in an older checkout.
   Review the version, license obligations, test evidence and source revision.
2. Run the manual native workflow from that reviewed revision; verify both
   targets on clean hosts, including Help/About, Zotero access, the real demo,
   year navigation, document rendering, and citations.
3. Verify the macOS ad-hoc package and first-launch instructions without
   Developer ID signing or notarization. Generate checksums from the exact
   final files; do not modify a package after recording its checksum.
4. Review the automatically prepared release draft and its matching
   installers, checksums, source and notices. Publish it manually only after
   acceptance, then verify each download independently.
5. Preview Pages, confirm the correct repository and links, then dispatch
   Pages with publication enabled. Check download URLs after publication.

Release download links become usable only after the matching release assets
are published. Check their availability and checksums before announcing the
release.

## Rollback

Keep the previous verified artifacts, source revision, checksums and Pages
artifact. Withdraw a bad release through the owner interface, restore links
to the previous release, restore the prior Pages sources on main through
an owner-reviewed change, and manually dispatch Pages with publication
enabled. Do not overwrite artifacts behind an existing checksum. Stop
Graphite before restoring a compatible pre-upgrade profile backup; an older
binary alone is not a database rollback. If no previous public release or
Pages deployment exists, disable publication instead of inventing one.

## Sources checked

- [Tauri Debian packaging](https://v2.tauri.app/distribute/debian/)
- [Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/)
- `ort-sys` 2.0.0-rc.13 locked distribution table and local native configs.

External documentation describes the framework. It does not establish that
this candidate passed native verification.

[WebKit 15.4 release notes](https://webkit.org/blog/12445/new-webkit-features-in-safari-15-4/)
document the native dialog support used by Help and Analytics.
