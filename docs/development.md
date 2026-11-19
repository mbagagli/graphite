# Development

## Supported workflow

Graphite uses one React/Vite frontend and one Tauri/Rust application. Local
retrieval and generation remain in the Rust process. The application itself
requires no Python environment, application server or container runtime. The
optional Linux Docling extraction action requires an already provisioned,
isolated Python runtime; see the [reader setup](../README.md). Do not install
or modify that runtime as an implicit part of normal application setup.

The repository pins Node.js 24.19.0, npm 11.17.0, and Rust 1.97.1. Install the
platform prerequisites for Tauri 2, plus CMake, a C/C++ compiler, and libclang
development files for the linked llama.cpp runtime. FastEmbed also builds an
ONNX Runtime dependency, and USearch builds native code. Optional CUDA, Metal,
and Vulkan builds require their platform toolkits. The native C++ dependency
graph has a configured macOS 11.0 floor. Only macOS 15 is proposed for
release acceptance; lower runtime compatibility remains unverified.

Before installation, verify that the selected tools match the repository:

```bash
node --version
npm --version
rustc --version
```

The expected versions are `v24.19.0`, `11.17.0`, and `rustc 1.97.1`. Use the
Node/Rust version selector already available on the host; the repository does
not require another environment manager. The exact npm engine is enforced, so
select npm 11.17.0 before running the install if the selected Node distribution
provides a different npm version.

Install JavaScript dependencies from the lockfile:

```bash
npm ci
```

The first clean Rust build may download and compile native dependency
artifacts. Cargo dependencies remain locked by `src-tauri/Cargo.lock`.
Downloaded ONNX Runtime archives use the ignored repository cache at
`.cache/ort/`; Cargo verifies the archive supplied by the pinned dependency.

## Continuing development from a fresh clone

The public repository retains application and website source, required assets,
unit tests and fixtures, build and release scripts, dependency lockfiles, and
pinned toolchain configuration. Installed dependencies and generated output
are recreated locally; application data and optional models are separate.

After installing the prerequisites above, run:

```bash
npm ci
npm run check:hygiene
npm run package:check
npm run check
npm test
npm run build
npm run test:site
npm run site:build
```

Use `npm run desktop:dev` to continue development. The initial dependency
installation and native build need network access. Optional demo PDFs and
models are downloaded explicitly in the app; they are not required to
compile the source or run the ordinary automated tests. Linux release
packaging also needs the documented CUDA toolkit; the hosted package job
prepares it on its own runner.

## Managed inference hardware

CPU/RAM detection and linked-runtime GPU enumeration are automatic. On
Linux, installed display adapters are also listed independently using local
system hardware records. A listed card is not a claim of inference support.
The GPU list under "Available to Graphite's inference engine" contains only
initialized devices the linked runtime can actually use.

Inference backend is a saved choice, captured with each question:

- **Auto** (default): choose the eligible GPU with most free memory, with
  CPU fallback after recoverable GPU failures.
- **CPU**: explicitly disable GPU model layers, attention and operation
  offload for answer generation.
- **GPU**: require an eligible GPU. Insufficient memory or a load/execution
  failure is an error; this explicit choice never silently switches to CPU.

Changes apply to the next question, and changing backend reloads the model.
A question already running keeps its original choices. Model loading checks
weights plus a 512 MiB reserve when free memory is reported; this does not
promise that every context fits. Automatic inference retries once only
before visible answer text and when not cancelled. CPU failures remain errors.
Native process aborts cannot be recovered in process. Unloading clears a
sticky automatic fallback; device initialization failures require restart.

`npm run desktop:dev` selects CUDA on Linux when both an existing toolkit
and an NVIDIA device reported by `nvidia-smi` are available. It uses Metal on
macOS. Linux otherwise builds CPU inference and explains the choice.
The toolkit lookup checks `GRAPHITE_CUDA_TOOLKIT`, `CUDA_PATH`, `CUDA_HOME`,
`.cache/cuda-toolkit`, PATH, then `/usr/local/cuda`. An invalid explicit
override is an error. A project-local symlink can reuse an isolated toolkit.
The launcher sets compiler and runtime paths only for its child processes;
it does not activate a Conda environment or change host drivers/libraries.

`npm run desktop:dev:cuda` remains available as a strict diagnostic command:
it errors if the existing toolkit/device pair is unavailable. Do not run two
development instances simultaneously; both use port 1420. Exiting the wrapper
also stops its development process group. GPU development uses a separate
`.cache/cargo-cuda` build cache. Its first compilation can take several minutes.
Window rendering acceleration is independent of inference acceleration.

`npm run package:build` produces the standard GPU-enabled Linux DEB/AppImage
with a CPU companion, or an Apple Silicon DMG with Metal. Linux release builds
require a toolkit even on CPU-only build runners. The owner-triggered workflow
prepares the checksum-pinned toolkit in the project cache with
`npm run cuda:prepare`; this explicit preparation requires an existing Conda
command and never installs system packages or drivers. Reuse an existing
toolkit locally instead. See [GPU packaging](linux-cuda-package.md) for details.

Context window is saved with **Medium** as the default, including after the
settings migration. The four steps are fractions of the selected profile's
supported maximum, shown as actual token counts:

| Profile     |   Low | Medium |   High | Extra-High |
| ----------- | ----: | -----: | -----: | ---------: |
| Lightweight | 2,048 |  4,096 |  6,144 |      8,192 |
| Balanced    | 4,096 |  8,192 | 12,288 |     16,384 |

The limit includes the question, prompt, evidence, and output. Retrieval may
use less due to relevance, diversity, and its existing evidence budgets.
The runtime allocates only the required bounded context. Higher settings can
permit more evidence but may use more RAM/VRAM and processing time; they are
not a quality switch. Answer detail remains a separate output-length choice.
The native engine enforces the selected limit, including final token checks.

"Advanced: answer model" separates Graphite's passage retrieval from the
model that writes the answer. **Built-in model** is the recommended choice
and owns model downloads, loading and device selection. **Another app on
this computer** reuses an existing local server. An external app such as
Ollama or LM Studio owns its own models,
GPU selection, and context configuration. Graphite continues retrieval and
citation validation in both cases. The external connection fields describe
that app's existing server and context; they do not reconfigure it. The
managed context slider and device choice are hidden for external providers.
The optional key is labeled for the local server, with session-only storage
and no GPT/Claude cloud access. Only explicit loopback HTTP(S) connections
are accepted, with proxies and redirects disabled. This update adds no
service or listening port.

A failed status refresh shows Unknown rather than stale GPU success.
Follow the [CUDA packaging workflow](linux-cuda-package.md) to validate the
standard GPU/CPU packages. Exercise GPU inference on supported hardware;
a CPU-only build runner cannot establish GPU, FUSE or window-system
compatibility for end users.

## Development modes and isolation

Browser-only development uses the real 60-paper metadata collection. It has
no Zotero, SQLite, model, or local PDF access:

```bash
npm run dev
```

Desktop development must use the development configuration:

```bash
npm run desktop:dev
```

The frontend watcher excludes native sources and generated build/benchmark
data. Tauri watches native source changes separately. This prevents large
local build trees from exhausting Linux file-watcher limits.

This command uses application identity `org.graphite.research.dev`. The
production identity remains `org.graphite.research`. Rust startup rejects a
development run that carries the production identity before resolving
directories, opening SQLite, or running product-rename migration. This remains
true if `tauri dev` is asked to compile in release mode.

Desktop development starts with the real demo metadata. Data Source offers
**Prepare library for AI**, using the configured Zenodo ZIP or a local ZIP
through **Already downloaded?**. Downloaded PDFs and derived text live in a
separate Graphite-owned partition. The published ZIP is pinned in
`demo/distribution.json`. To test Zotero integration, explicitly select Use Zotero. The development build may then read the
loopback Zotero Local API, but all Graphite databases, models, indexes,
conversations, and WebView state remain scoped to the development identity.
Zotero remains read-only.

If Zotero is stopped or its local API is disabled, Graphite keeps the empty or
previously synchronized development workspace available and shows a recoverable
connection notice. Start or configure Zotero and choose `Retry Zotero`, or
return immediately to the real demo metadata with `Use demo data`. Connection failures
do not modify Zotero.

New profiles use Tauri's machine-local application-data directory. On Windows,
an existing current-identity database in the former roaming data directory is
used in place rather than moved. Existing model and index directories are also
reused in place. A database takes priority over an auxiliary directory in the
other root. If both roots contain databases or otherwise ambiguous state,
startup fails without modifying either and asks the developer to back up and
resolve the duplicate roots. Only the exact production identity with no current
application state may run the older-product identity migration; development
never inspects or mutates that source. Close the previous application before a
first production launch that will perform this migration.

If Graphite closes during semantic indexing, the next startup refreshes
coverage from saved documents and reports the interrupted operation. Continue
with **Resume preparation** in library setup. Completed documents are
retained and unchanged documents are not embedded again. Processing a document
can still yield zero usable passages; check passage coverage before asking
questions about it.

An answer table containing only the exact insufficient-evidence response for
known sources is normalized to the standard refusal. Mixed factual rows and
unknown source labels still pass through the normal grounding checks.

Final model installations and vector sidecars remain in persistent local data,
not the OS-evictable cache. Their database records currently require explicit
reconciliation if files disappear, and model downloads can be several
gigabytes. Moving transient download state or rebuildable indexes into the OS
cache is deferred until that recovery behavior is implemented and tested.

## Validation

### Automatic checks before pushing

Install the pinned pre-commit tool in an isolated environment, then install
this checkout's pre-push hook from the repository root:

```bash
uv tool install pre-commit==4.7.0
pre-commit --version
pre-commit install --hook-type pre-push
```

If you already use pipx instead of uv, `pipx install pre-commit==4.7.0`
provides the same isolated tool. Keep the existing Node/npm and Rust tools
available on PATH, including in the terminal or Git client used to push.
Run `npm ci` first and install the native prerequisites described above.
The tool does not become a Graphite runtime dependency or change the
application's Python environment.

Hook installation is local to each checkout; repeat it after cloning.
The checked-in `.pre-commit-config.yaml` defaults to the pre-push stage,
so plain `pre-commit install` also installs the correct hook. Commits do
not trigger these checks. The hook runs before a normal Git push and stops
the push on the first failure. It never formats source files automatically.

The configured checks reuse the existing project commands:

- Prettier checks supported files in the pushed changes and respects
  `.prettierignore`. A first push can include the entire tracked tree.
- Repository hygiene and packaging configuration, including version consistency.
- ESLint, application/packaging tests and comparison contract tests.
  Website tests run separately with `npm run test:site` in the Pages workflow.
- TypeScript checking and the production frontend build.
- Rust formatting, Clippy with warnings rejected, and native tests using
  the locked dependencies and default CPU features.

All checks except changed-file formatting run even for documentation-only
pushes. Initial native compilation can take several minutes and may fetch
locked build dependencies; later runs reuse the normal build cache. This
does not download AI models. GPU builds, installers, browser/desktop QA and
live-library/model tests remain separate checks, as described below and in
the release workflows.

To check the current working files without pushing or temporarily hiding
uncommitted changes, run:

```bash
pre-commit validate-config
pre-commit run --hook-stage pre-push --all-files
```

For a focused formatting pass plus all project-wide checks, replace
`--all-files` with `--files` followed by the changed file paths. A formatting
failure lists the affected files. Review required corrections and rerun the
check rather than skipping hooks.

Commit the hook configuration with the project changes before pushing.
pre-commit may temporarily hide unstaged tracked edits during a normal
push and restores them afterward. An unstaged edit to its configuration
must be staged or reverted before that automatic run.

If a push does not show the check names above, rerun the install command
and check whether the Git client overrides `core.hooksPath` or disables
hooks. Local hooks can be deliberately bypassed; the independent CI checks
remain necessary. See the [pre-commit documentation](https://pre-commit.com/)
for hook behavior and the [uv tool guide](https://docs.astral.sh/uv/guides/tools/)
for isolated installation.

### Individual validation commands

Run the frontend checks:

```bash
npm run format:check
npm run lint
npm run check
npm test
npm run build
```

`npm test` runs the frontend and script unit tests with Vitest, then the
comparison contract tests with Node's test runner. To filter frontend tests,
use `npm run test:unit -- <filter>`; to run only the comparison contracts,
use `npm run test:comparison`.

Run the native checks:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo build --manifest-path src-tauri/Cargo.toml --locked
```

GPU features must be checked separately on a host with the corresponding
toolkit. Live model and real-library tests are intentionally opt-in because
they require explicit local data or multi-gigabyte downloads.

## Assistant interface

Tyron starts as a compact launcher. Opening it focuses the question input;
clicking elsewhere leaves it open. The close button returns to the launcher
without clearing the in-session draft. The single scope disclosure keeps the
active scope visible and exposes the existing context controls when expanded.

The composer and scope controls remain visible and usable across all four
assistant tabs, including Local setup, with identical panel bounds. Switching
tabs does not move focus into the composer or clear the draft. Setup shows
model availability and answer detail first. CPU, RAM, and detected GPUs use matching icon/text rows. Hardware, provider configuration, and runtime details are expandable.
Model, provider, backend, context-window, and answer-detail choices save immediately; advanced text
fields save when focus leaves the field. Saves are serialized, with visible
pending, saved, and retryable failure states. Invalid external configuration
still fails the existing native validation. Credentials remain session-only.
Downloads require an explicit button click. Returning to Answer restores the
composer draft; drafts are not newly persisted across application restarts.

## Portable browser QA

Visual and performance QA exercise the browser metadata application. Start
the loopback-only Vite server in one terminal:

```bash
npm run dev
```

Run QA from another terminal:

```bash
npm run qa:visual
npm run qa:graph-picking
npm run qa:performance
npm run qa:performance:large
```

Firefox and Geckodriver are native prerequisites. The scripts resolve
`geckodriver` from `PATH` and let Firefox use its normal platform discovery.
Use these overrides only when discovery is insufficient:

| Variable                     | Purpose                                                                   |
| ---------------------------- | ------------------------------------------------------------------------- |
| `GRAPHITE_QA_GECKODRIVER`    | Explicit Geckodriver executable path                                      |
| `GRAPHITE_QA_FIREFOX_BINARY` | Explicit Firefox executable path                                          |
| `GRAPHITE_QA_DRIVER_PORT`    | Fixed loopback WebDriver port; the default is an available ephemeral port |
| `GRAPHITE_QA_ARTIFACTS_DIR`  | Alternate artifact root, relative to the repository unless absolute       |

Missing executables fail with installation and override guidance. Inspectable
results go to ignored directories under `artifacts/qa/` by default. Performance
profile labels are restricted to safe filename characters.

Visual and performance QA refuse to continue unless the page is served from `127.0.0.1`,
reports the demo source, has no Tauri runtime, and loaded no cross-origin
resources. It neither opens Graphite application storage nor contacts Zotero.
The visual journey covers real metadata search, themes, publication years,
source capability guidance, and desktop/compact layouts. It does not claim
browser PDF, citation extraction, semantic indexing, or model execution.
Use `qa:pdf-editor` with a verified real PDF for editing checks and native
integration tests for actual retrieval and generation.

The unified preparation interface has a separate browser check:

```bash
node scripts/library-preparation-smoke.mjs
```

This check uses the same local preview and browser overrides, plus a
transient page with simulated native commands and the real public demo
metadata. It exercises the real preparation coordinator and setup controls
at desktop and compact sizes, including keyboard access, ZIP picker
cancellation, progress after reopening setup, explicit model installation
followed by resume, partial coverage and download cancellation. It never
opens application storage or contacts Zotero, publishers or Zenodo.
Its screenshots and receipt are browser UI evidence only; archive integrity,
PDF extraction and model execution require the native tests. The temporary
page is removed when the check finishes.

The site build verifies screenshot and collection hashes, and requires a
recorded capture version. The workspace caption displays that version
independently of the current installer version. Version bumps do not rewrite
capture provenance. Recapture screenshots when the illustrated interface or
collection changes, updating their hashes and capture version together.

## Continuous integration

`.github/workflows/ci.yml` runs development validation and is also reused
as a required gate by the release workflow. It installs the
pinned Node/npm and Rust toolchains from a clean checkout, then runs repository
hygiene, publication-content and privacy checks, formatting, linting,
TypeScript, frontend tests/build, Rust
formatting, Clippy, Rust tests/build, and native Tauri compile/link checks
without creating installers.

The declared native coverage is:

- Ubuntu 24.04 x64: full Rust validation and linked Tauri debug/no-bundle build.
- macOS 15 ARM64: linked Tauri debug/no-bundle build.

`.github/workflows/native-compatibility.yml` runs independently on pushes to
main, pull requests and manual dispatch. It is not called by the release:

- Windows Server 2025 x64: linked Tauri debug/no-bundle build.
- macOS 15 Intel x64: frontend build and native all-target Cargo check only.

Windows is currently unavailable to users. The compatibility lane is
development coverage, not a Windows release. See the
[Windows enablement plan](release/WINDOWS.md).

Website build, screenshot provenance and `npm run test:site` belong to the
Pages workflow. `npm test` covers application, packaging and comparison
contracts without running the website suite.

The Intel lane is not a link, execution, packaging, or distribution claim. The
pinned ONNX Runtime has no prebuilt Intel macOS archive, so this check explicitly
skips its binary download and defers its missing-library error to a link step that
the lane does not perform. Rust application code and native build scripts still
compile. Resolving the Intel support decision belongs to packaging work. Platform
coverage is authoritative only after the corresponding hosted job succeeds.

CI uses mock or deterministic fixtures, requires no secrets, starts no
application or QA server, uploads no artifacts, and never reads installed
Graphite or Zotero data. No devcontainer is required: the repository pins and
native hosted jobs provide useful reproducibility, while a Linux container
would not validate Windows or macOS integration.

## Generated files and cleanup

The repository ignores dependencies, frontend output, Cargo output, tool
caches, QA artifacts, databases, models, indexes, logs, and temporary files.
Repository-local tool caches and Cargo output are centralized under `.cache/`.
QA artifacts use `artifacts/qa/`.

`npm ci` recreates `node_modules` from the lockfile. Use Cargo's normal cleanup
for the configured repository-local target directory:

```bash
cargo clean --manifest-path src-tauri/Cargo.toml
```

To reclaim only downloaded npm cache entries, use the repository-scoped npm
configuration:

```bash
npm cache clean --force
```

This affects `.cache/npm` only; it does not remove installed application data.
The ONNX Runtime download cache at `.cache/ort/` is also disposable and is
recreated by a later native build.

An older checkout may still have ignored output under `src-tauri/target`. It is
not used after the repository Cargo configuration takes effect.

Do not automate deletion of installed application data. If a full development
reset is necessary, close Graphite, verify that the target directory ends in
the exact development identity `org.graphite.research.dev`, back up anything
needed, and remove only that development directory. Never use the production
identity as a development reset target. Prefer the application's scoped
rebuild controls when only derived indexes need regeneration.

## Sidebar finder and unified graph navigation

Use **Find a paper** immediately above My Library to search the active library
without an embedding model or semantic index. Suggestions open upward. Use
Up/Down and Enter to choose a paper, Escape to dismiss, or **Open full search**
to continue with the same query. The collapsed rail's search button expands
and focuses the finder. A result outside the current collection or projection
loads in the complete active-library view before the camera focuses it.

Every launch starts in **3D** with a gentle 1.8-second camera arrival. The
vertically stacked **3D / 2D** controls expand the network or collapse it into
a readable plane over 600 ms. Both states use the same canvas. Mode and
separate cameras survive Reader round-trips within the session; returning
from Reader does not replay startup.

In 3D, drag to orbit, right-drag to pan and scroll/pinch to dive. In 2D, drag
to pan and scroll/pinch to zoom; rotation is locked. Double-click a paper or
use **Focus selection**. Shift/Command-click adds to selection. **Fit graph**
restores the overview. Focus the canvas and use arrow keys to orbit in 3D or
pan in 2D, plus/minus to zoom, and Home to fit. Reduced motion removes spatial
transitions. Graph proximity aids navigation; it is not a similarity score.

A graphics interruption shows **Retry graph**. Search and Reader remain
accessible while the graph is unavailable. Retrying restores the available
layout, selection and camera without replaying the arrival.

Model downloads remain in application data across restarts. **Release loaded
model memory** only unloads RAM; it does not delete downloaded files. Missing
or incomplete files produce an explicit error and require an explicit install
operation. Models and parser defaults remain unchanged.

The 3D dependency is loaded only on demand. Its optimized chunk is about
1.50 MB (400 KB gzip); the build's large-chunk warning remains visible rather
than being suppressed. Startup and default library/collection requests load
all available papers. Explicitly bounded requests retain a 1,200-node cap;
search paging and focused citation/similarity limits are unchanged. Layout
preparation remains in a worker, with memory and preparation time growing
with library size.
