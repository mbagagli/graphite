# Packaging and installed-runtime validation

The current release contract is in [Build and publication](release/BUILD.md).
It uses version 0.3.4 and prepares Linux x64 DEB/AppImage and macOS Apple
Silicon DMG candidates. Windows is currently unavailable; we are working
on support. See the [Windows status](release/WINDOWS.md). Build and test the
exact versioned artifacts before publishing; earlier packages do not validate
changed source.

Packages use GNU GPL version 3 (`GPL-3.0-only`) and carry the repository
LICENSE. Metadata uses the neutral Graphite identity. The candidate workflow
has no signing credentials and cannot create tags or publish releases.
The [release workflow](release/BUILD.md#build-installers-and-create-a-release-draft)
starts from a matching version tag, reuses CI and package validation, and
attaches the verified installers to an unpublished GitHub release draft.
Publishing that draft remains a manual owner action.
macOS distribution is intentionally unsigned and unnotarized; a free local
ad-hoc signature supports Apple Silicon without an Apple account. See the
[signing policy](../CODE_SIGNING_POLICY.md) and
[first-launch instructions](release/BUILD.md#reproduction-and-installation).

Build on a native host with the repository's pinned dependencies:

```bash
npm ci
npm run version:check
npm run package:check
npm run check:publication
npm run check:privacy
npm run package:build
```

Publication and privacy checks inspect tracked source. Run them after staging
only the intended public files in the fresh repository. Ignore rules prevent
new local-only files from being added normally; they do not remove files
already tracked in an older checkout. Do not force-add ignored material.

Then run `npm run package:smoke -- linux-x64` or
`npm run package:smoke -- macos-arm64`. The build wrapper remaps private
build paths in Rust, C/C++, and Objective-C/Objective-C++ sources, including
Metal assertion strings. Structural inspection stages only
packages, checksums, and synthetic metadata below the ignored
`artifacts/package-smoke/` directory. Raw bundles are below the ignored
`.cache/cargo-target/release/bundle/` directory.

Package privacy failures report the file and finding category (build
identity, credential-shaped token, or private key), never the matched value.
The macOS unit suite compiles a small Objective-C/Objective-C++ fixture to
check path remapping before the full application build. CI also builds Pages
so screenshot provenance problems are caught before release packaging.

Both macOS smoke checks mount the DMG read-only in a dedicated temporary
directory. They retry normal unmounting up to three times and verify that the
volume is no longer mounted before deleting that directory. If unmounting
fails or cannot be verified, the check fails and retains the directory for
runner cleanup. Diagnostics preserve inspection and cleanup errors together;
no forced unmount or skipped package check is used.

macOS dependency inspection accepts Apple's system SQLite library under
`/usr/lib/`, while still rejecting external copies of the checked runtimes.
Installed-startup failures include captured standard output and error output,
including timeouts. If process termination also fails, both errors remain
visible. A vanished unrelated volume does not prevent DMG cleanup; other
mount-inventory errors still fail the check without deleting its directory.

The installed-runtime smoke command is deliberately restricted to clean
GitHub Actions runners. Do not bypass its guards or test installation over
a valuable application profile. Native interactive acceptance needs a
disposable VM/account or isolated filesystem and profile. Older acceptance
reports do not validate this changed candidate.

## Models, runtime dependencies, and updates

ONNX Runtime, llama.cpp, Tokenizers/Oniguruma, USearch/NumKong, and SQLite
are native dependencies. Packages do not include research PDFs, databases,
models, credentials, or user settings. Model downloads remain explicit
in-app operations. Standard Linux packages now include the CUDA runtime and
a CPU companion; Apple Silicon packages enable Metal. Auto prefers a
supported GPU and falls back to CPU. The workflow prepares an isolated pinned
CUDA toolkit on its Linux runner; local builds reuse the existing toolkit.
See [GPU packaging](linux-cuda-package.md) for prerequisites and checks.

There is no automatic updater. Preserve a complete profile backup before
upgrading; installing an older executable does not undo database migration.
Restore the matching pre-upgrade profile only while Graphite is closed.
Zotero data remains untouched.
