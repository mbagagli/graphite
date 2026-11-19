# Windows x64 beta

The first Windows beta uses the CPU for built-in answer generation, including
on computers with NVIDIA GPUs. Use **Auto** or **CPU** in the local model
settings. The model runs offline after you download it in Graphite.

[Find your beta](https://github.com/mbagagli/graphite/releases) and look for
the `Graphite_<version>_x64-setup.exe` asset on a published release.
Check its SHA-256 against the release's `SHA256SUMS.txt` before running the
unsigned NSIS installer. Setup may need internet access if Microsoft
Edge WebView2 Runtime is missing. Published releases list their available
platform assets.

## Existing starting point

The `Native compatibility` workflow checks Windows x64 on `windows-2025`,
independently of Linux/macOS releases. Its regular run links the development
application. The manual `windows_installer` input uses NSIS and the existing
`windows-x64` inspection and installed-runtime checks.

The release workflow builds its Windows x64 candidate on `windows-2025`,
then runs the same inspection and installed lifecycle before including it
in the release draft. See the [release workflow](BUILD.md).

## First Windows installer experiment

Start with native Windows x64 and CPU answer generation. The locked Windows ONNX
dependency enables DirectML; inspect its actual runtime dependencies even
when answer generation uses CPU.

Use **Run workflow** on **Native compatibility** and enable
`windows_installer` on the branch containing the changes. The workflow
builds an unsigned NSIS installer, inspects its contents, and exercises its
installed lifecycle on the hosted runner. A successful inspection makes
the installer and checksums available as the
`graphite-windows-x64-unsigned-experiment` artifact for seven days. Check the
lifecycle step separately before using that artifact for desktop acceptance.

Use a disposable Windows host with the repository's pinned Node/npm and
Rust versions, Microsoft C++ Build Tools, WebView2, CMake and LLVM/libclang.
Use an x64 MSVC developer shell. For prerequisites and NSIS/WebView2
options, see the official [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
and [Windows installer guide](https://v2.tauri.app/distribute/windows-installer/).

In PowerShell, from the repository root:

```powershell
npm ci
$clang = Get-Command clang.exe -ErrorAction Stop
$env:LIBCLANG_PATH = Split-Path $clang.Source
npm run package:check
npm run tauri -- build --ci --bundles nsis -- --locked
npm run package:smoke -- windows-x64
```

These are proposed validation commands, not a record of a successful run.
The native build uses the Windows platform configuration automatically.
Omitting an explicit target keeps output in the existing package inspector's
expected directory. Inspection requires 7-Zip on PATH. Do not use
`npm run package:build` for this experiment: its current platform guard
rejects Windows.

## Beta acceptance

1. Run the manual packaging experiment. Retain the inspected NSIS installer
   and checksums for testing.
2. Inspect the executable's DLL dependencies and ship required redistributable
   components. Validate the ONNX/DirectML runtime on a clean host rather than
   relying on libraries already installed on the build runner. Decide whether
   WebView2 is downloaded during setup or included for offline installation.
3. Run `npm run package:installed-smoke -- windows-x64` on a clean hosted
   Actions runner after inspection. It checks first launch, state isolation,
   uninstall/reinstall and state preservation. Keep its runner guards; do not
   enable them artificially on a personal Windows profile.
4. Test the exact installer on a clean Windows desktop: graph rendering,
   PDF reading/export, read-only Zotero access, explicit model download,
   semantic indexing, CPU answers and reopening saved research. Check paths
   containing spaces/non-ASCII characters and both existing storage layouts.
   Test upgrade from an earlier candidate as well as reinstalling one build.
5. Publish the release draft after the exact Windows installer completes
   acceptance. Check the asset and checksum on the public Releases page.

After the CPU installer has a reproducible install, application and upgrade
baseline, test a CUDA build on a Windows NVIDIA host. Check that the package
includes the necessary CUDA redistributable DLLs and that Graphite still
launches and answers on a machine without an NVIDIA driver. Validate the two
paths before making CUDA part of the standard Windows installer.
