# GPU acceleration in standard packages

`npm run package:build` builds unsigned Linux x64 DEB/AppImage packages with
NVIDIA CUDA acceleration and a CPU companion. Apple Silicon DMG builds
explicitly enable Metal. A new profile defaults to Auto: prefer an eligible
GPU, then use CPU if none is available or a recoverable inference error occurs.
Saved CPU or GPU selections remain unchanged. Explicit GPU reports an error
instead of silently changing the user's choice.

End users need no compiler, toolkit, or separate GPU package. A compatible
existing NVIDIA driver is required for CUDA. These packages never install or
replace drivers or system CUDA libraries. AMD/Intel inference on Linux is not
enabled by this CUDA package; those machines use CPU. Metal serves Apple GPUs.

## Build and toolchain

The owner-triggered package workflow prepares the pinned CUDA 13.4.92 and
cuBLAS 13.8.0.4 toolkit in `.cache/cuda-toolkit`, builds both current production
executables, and assembles the standard packages. macOS builds use the runner's
Apple toolchain. No workflow publishes a release automatically.

For local development, reuse an existing toolkit through
`GRAPHITE_CUDA_TOOLKIT` or a `.cache/cuda-toolkit` symlink. The normal
`npm run desktop:dev` detects it and the available NVIDIA device, configures
`nvcc` explicitly, and enables shared CUDA inference libraries. No download
occurs. Without a usable toolkit/device pair it builds CPU inference.

For a fresh Linux build runner with Conda already available:

```sh
npm ci
npm run cuda:prepare
npm run package:build
npm run package:smoke -- linux-x64
```

Preparation is explicit and refuses to overwrite an existing toolkit. Every
archive in `scripts/cuda-toolchain.lock.json` is SHA256-verified before offline
extraction into the isolated prefix. The CUDA and cuBLAS redistribution notices
are retained in `.cache/cuda-notices` and included in the resulting packages.
When reusing a toolkit, place its matching `CUDA-LICENSE.txt` and
`cuBLAS-LICENSE.txt` there before packaging.

Only the CUDA runtime directory is added to the child library search path;
the whole Conda environment is never activated. CPU and GPU builds use
separate caches. Both release executables include `tauri/custom-protocol` and
the same embedded frontend. Build paths are remapped before compilation.

The release architecture list is centralized in `scripts/native-build.mjs`:
the CUDA 13 defaults of the pinned llama.cpp engine, with PTX for SM75, 80,
and 90 and compiled kernels for SM86, 89, 120a, and 121a. This allows
compatible devices to compile PTX on first use. Development compiles for the
detected local architecture. `CMAKE_CUDA_ARCHITECTURES` can override
this for targeted builds, which must not be described as general releases.
A compiled target list does not establish that every driver/card combination
has been tested. Unsupported or unavailable devices use the existing Auto CPU
fallback. No driver stubs are bundled.

## Package layout and validation

`usr/bin/graphite` is a relative launcher. The private
`usr/lib/graphite-cuda` directory contains the CUDA executable, CPU companion,
matching llama/ggml libraries, CUDA runtime, and cuBLAS. System GTK/WebKit
libraries remain DEB dependencies; AppImage keeps its desktop-library support.
No models, research data, profiles, or development server are bundled.

If `libcuda.so.1` is absent, the launcher chooses the CPU companion. If the
driver loads but initializes no eligible GPU, the application's Auto setting
uses CPU. Missing bundled libraries are package errors. Automatic inference
can retry on CPU before any answer text is streamed; native process aborts
remain unrecoverable. CPU and GPU controls apply to the next question.

Package inspection checks both executable paths, the exact private runtime,
architecture, loader paths, notices, and absence of private build identities.
The workflow's GPU-free Linux runner exercises installed CPU fallback. Native
GPU verification still requires a suitable GPU host; macOS build and launch
checks run on its native workflow runner.

The lower-level `scripts/package-linux-cuda.mjs INPUTS.json OUTPUT_DIRECTORY`
assembler remains available for isolated local checks. It requires absolute
paths for `baseDeb`, `baseAppImage`, matching `cpuBinary` and `cudaBinary`,
`llamaLibraries`, `cudaLibraries`, `appImageTool`, and the three runtime
`licenses`. `sourceRevision` is a full commit ID or null for a local build;
input binaries and libraries are always hashed. `versionSuffix: ""` preserves
the normal release version; omission retains the earlier `+cuda1` candidate
naming. The output directory must not exist. It installs nothing.

Run `npm run test:unit` for launcher, selection, and archive-verification
regressions. Verify actual Auto/GPU inference, explicit CPU inference and
unavailable-device CPU fallback against the exact package on suitable
hardware, using a disposable profile and an explicitly installed model.
Extraction-mode AppImage startup does not establish FUSE startup; software
WebView rendering does not establish accelerated window rendering. Native
macOS verification requires its own build and launch checks. Signing and
publication remain separate actions.
