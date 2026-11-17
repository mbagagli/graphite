# Graphite PDF benchmark harness

This directory contains Milestone 5 benchmark tooling. It is deliberately
outside `src-tauri`: candidate parser crates, OCR runtimes, model files, raw
PDFs, and benchmark binaries are not production dependencies or bundle
resources.

## Layout

```text
common/                 Parser-neutral contracts, canonical mapping, metrics
runner/                 JSON-lines subprocess runner and validators
zotero-cache-capture/    Read-only preparation helper for production-cached Zotero text
quality-oracle/          Production quality gate and unchanged-chunker oracles
policy-simulator/        Observable-signal routing simulations and post-hoc scoring
adapters/zotero/        Byte-faithful source-text baseline
adapters/xberg/         Xberg 1.0.14 adapter
adapters/pdf-inspector/ PDF Inspector 1.15.0 adapter
adapters/docling/       Docling.rs 1.16.0 adapter
adapters/test-double/   Crash, timeout, and outcome containment fixture
scripts/                Local review and cross-platform asset audits
config/                 Frozen profiles and checksum-locked assets
corpus/                 Tracked templates and ignored local documents
fixtures/               Tracked parser-neutral contract fixtures
results/                Ignored raw and local aggregate output
```

Every package has its own `Cargo.toml` and `Cargo.lock`. There is no benchmark
workspace and no candidate dependency in the production Tauri manifest. The
runner starts a fresh subprocess for every extraction, so a parser crash or
timeout cannot corrupt another candidate or document attempt.

## Protocol

Adapters read one JSON object per line from standard input and write one JSON
object per line to standard output. Protocol messages contain only the common
benchmark request, descriptor, and extraction envelope. Candidate-native
types are converted inside the adapter.

The outcome vocabulary is:

```text
succeeded
partial
suspicious
empty_success
unsupported
unavailable
timeout
crashed
failed
```

`unsupported` means the candidate does not support the input or profile.
`unavailable` means it supports the operation but a configured runtime or
asset is absent. Neither is treated as a parser failure.

## Profiles

- `native`: use the embedded text layer and deterministic native logic only.
- `recommended-auto`: use the candidate's own native/OCR routing policy.
- `forced-ocr`: force OCR for the selected document or annotated pages.

Unsupported profiles are not invented. A native-only build advertises only
`native`.

## Build and prepare

Build each package independently:

```text
cargo build --manifest-path tools/pdf-benchmark/runner/Cargo.toml --offline
cargo build --manifest-path tools/pdf-benchmark/adapters/zotero/Cargo.toml --offline
cargo build --manifest-path tools/pdf-benchmark/adapters/xberg/Cargo.toml --offline
cargo build --manifest-path tools/pdf-benchmark/adapters/pdf-inspector/Cargo.toml --offline
cargo build --manifest-path tools/pdf-benchmark/adapters/docling/Cargo.toml --offline
```

OCR builds remain isolated feature builds:

```text
cargo build --manifest-path tools/pdf-benchmark/adapters/xberg/Cargo.toml --no-default-features --features ocr
cargo build --manifest-path tools/pdf-benchmark/adapters/pdf-inspector/Cargo.toml --no-default-features --features ocr
cargo build --manifest-path tools/pdf-benchmark/adapters/docling/Cargo.toml --no-default-features --features ocr
```

Validate and explicitly prepare runtime assets before disconnecting the
network:

```text
graphite-pdf-benchmark-runner validate-assets tools/pdf-benchmark/config/assets.lock.json
graphite-pdf-benchmark-runner prepare tools/pdf-benchmark/config/assets.lock.json pdf-inspector /path/to/local/assets
graphite-pdf-benchmark-runner verify-prepared tools/pdf-benchmark/config/assets.lock.json pdf-inspector /path/to/local/assets
graphite-pdf-benchmark-runner verify-protocol tools/pdf-benchmark/config/smoke-protocol.lock.json .
```

The preparation command accepts HTTPS only, verifies every SHA-256 and optional
byte count, filters platform assets, and unpacks locked `.tgz` archives. Set
`GRAPHITE_PDF_BENCH_ASSET_ROOT` to that local asset root when running. The
runner also sets the Hugging Face and benchmark offline variables in every
adapter subprocess.

`config/docling-linux-x64.prepared.json` records the current local preparation
result by asset ID and refers back to the single authoritative asset lock for
names, versions, sources, checksums, sizes, licenses, and required profiles. It
does not contain a local asset path or any model bytes.

Xberg 1.0.14 has an upstream clean-build limitation: enabling its Tesseract
feature invokes build-script downloads for Leptonica 1.87.0 and Tesseract
5.5.3 source archives. The runtime language data is pinned here, but it does
not satisfy those build-time requests. A fresh native-only build succeeds
offline; a fresh OCR build does not. Treat Xberg OCR as unavailable for a
reproducible package unless the native sources are explicitly locked and
injected through a maintained patch or vendor process.

## Corpus and annotations

Copy `corpus/manifest.template.json` and
`corpus/anchors/paper.template.json`. Put non-sensitive local PDFs and optional
Zotero text sidecars under `corpus/local/`; that directory is ignored by Git.

Every manifest record requires:

- a relative PDF path and SHA-256;
- a declared license or local evaluation basis;
- scientific-document categories and expected page count when known;
- native, scanned, mixed, broken, or unknown text-layer classification;
- expected OCR pages where known;
- a relative anchor path and its SHA-256;
- an optional relative Zotero sidecar path and its SHA-256.

Anchors use exact, whitespace-normalized, ordered-token, alternatives, or
numeric-with-context matchers. Semantic anchors must be represented by
reviewed deterministic alternatives or ordered tokens. No LLM judge is used.
Anchor schema v2 records `suggested-unverified`, `human-verified`, or
`human-rejected`. Validation accepts suggestions during annotation, but
scoring refuses them. Only human-verified anchors enter fidelity metrics; see
`corpus/ANNOTATION-REVIEW.md` for the review workflow.

The real smoke manifest is `corpus/smoke-manifest.json`. Its local PDFs use
neutral `paper01.pdf` through `paper10.pdf` aliases under the ignored local
corpus directory. Descriptive source filenames and absolute paths do not enter
the tracked manifest.

The representative expansion is `corpus/representative-manifest.json` and uses
neutral `paper11.pdf` through `paper30.pdf` aliases. It remains a separate
manifest so reports can show adversarial, representative, and combined results
without allowing the stress subset to distort ordinary-document summaries.
Its anchors are frozen benchmark evidence.

The M5-C difficult-class preparation is a third, separate manifest at
`corpus/targeted-manifest.json`. It uses neutral `paper31.pdf` through
`paper35.pdf` aliases and contains only suggested anchors. It must not be
combined with or scored as part of the frozen 30-document corpus until those
suggestions have been reviewed.

Validate before running:

```text
graphite-pdf-benchmark-runner validate /path/to/manifest.json
```

Where a benchmark paper is present in the local Zotero library, capture the
exact production full-text response with the read-only helper:

```text
graphite-pdf-benchmark-runner capture-zotero MANIFEST DOCUMENT_ID user LIBRARY_ID ATTACHMENT_KEY
```

The command uses only Zotero local-API `GET` requests, stores the exact decoded
`content` string in the ignored local corpus directory, and records its
checksum. It does not normalize text or edit the tracked manifest. Missing
Zotero items remain an explicit unavailable baseline.

If the live local API is unavailable but Graphite has already cached the same
production GET response, use the separate preparation-only helper:

```text
graphite-pdf-benchmark-zotero-cache-capture REPRESENTATIVE_MANIFEST DOCUMENT_ID GRAPHITE_DATABASE ATTACHMENT_KEY
```

The helper opens SQLite with read-only flags, accepts only `zotero-indexed`
rows, rejects conflicting latest rows, preserves content bytes, and labels the
sidecar as production-cached rather than freshly captured. Its own Cargo lock
keeps the permanent smoke runner and scoring protocol unchanged. The database
path and attachment key are local command inputs and never enter tracked
manifests or sidecars.

## Synthetic validation

Generate the tracked-code, temporary 10-document harness corpus:

```text
graphite-pdf-benchmark-runner fixtures /tmp/graphite-pdf-fixtures
```

The generator also writes a four-document malformed/encrypted/failure manifest
and an eight-outcome protocol manifest. The scanned pages are actual image-only
PDF pages; their anchor strings are not present as PDF text bytes.

Synthetic results verify the harness and adapters. They must not be used to
choose a production parser.

## Run, score, and compare

```text
graphite-pdf-benchmark-runner run MANIFEST RUN_DIR config/timeout-policy.json zotero=PATH xberg=PATH pdf-inspector=PATH docling=PATH
graphite-pdf-benchmark-runner score MANIFEST RUN_DIR AGGREGATE_JSON
graphite-pdf-benchmark-runner privacy AGGREGATE_JSON
graphite-pdf-benchmark-runner compare RUN_DIR_A RUN_DIR_B
graphite-pdf-benchmark-runner measure MANIFEST OPERATIONAL_JSON config/timeout-policy.json zotero=PATH xberg=PATH pdf-inspector=PATH docling=PATH
cargo run --offline --locked --manifest-path tools/pdf-benchmark/quality-oracle/Cargo.toml --bin graphite-pdf-benchmark-quality-oracle -- RUN_DIR AGGREGATE_JSON QUALITY_JSON
cargo run --offline --locked --manifest-path tools/pdf-benchmark/quality-oracle/Cargo.toml --bin rag -- MANIFEST RUN_DIR RAG_JSON
cargo run --offline --locked --manifest-path tools/pdf-benchmark/policy-simulator/Cargo.toml -- SMOKE_AGGREGATE SMOKE_QUALITY SMOKE_OPERATIONAL REPRESENTATIVE_AGGREGATE REPRESENTATIVE_QUALITY REPRESENTATIVE_OPERATIONAL POLICY_JSON
```

Raw extraction envelopes contain local text and canonical payloads. Keep them
only under ignored local result directories. Aggregate output contains metrics,
hashes, IDs, and statuses only. The privacy validator rejects payload fields,
file URLs, and common absolute-path forms.

The comparison command removes timing fields only. All other normalized output
must match between repeated runs for a parser/profile/input that claims
determinism.

The policy simulator receives only privacy-safe aggregate and quality files.
Its routing functions can access outcome, usability, quality, coverage,
page/truncation/encoding signals, and source availability, but cannot access
anchor data. Human anchor results are joined only after each decision. An
active Zotero safety-net policy retains both projections; it does not discard
candidate-only structure.

The operational command issues two extraction requests in one isolated
adapter process. Cold time includes process start, handshake, and the first
extraction; warm time covers the second extraction. On Linux it samples
process-tree RSS, user/system CPU time, CPU utilization, thread count, isolated
temporary-directory use, response bytes, adapter binary bytes, and prepared
asset bytes. Unsupported platform signals remain null. The report explicitly
states that the OS page cache is not controlled.

If cold or warm extraction reaches the timeout, the operational report retains
an explicit timeout phase and the sampled resource peaks. A cold timeout has no
warm phase because that adapter process is no longer reusable.

The quality oracle constructs Graphite-owned attempts from normalized output
and invokes the unchanged production `ExtractionQualityEvaluator`. The RAG
oracle includes the production `scientific-paragraph-v1` implementation and
records its source hash. It uses the deterministic whitespace token counter
used by production chunker tests, so its ordinals are benchmark-oracle
ordinals and do not require loading an embedding model.

The frozen timeout policy assigns 90 seconds to ordinary papers and 120
seconds to scanned/OCR or 500-page-and-longer stress documents. Every result
records the timeout class, ceiling, and whether the stage was parsing, OCR, or
an indeterminate automatic pipeline. A timeout remains distinct from parser
failure, and the runner never retains incomplete JSON-lines output.

## M5-C local review and platform checks

Create the local, human-adjudicable miss packet without changing frozen
scores:

```text
node tools/pdf-benchmark/scripts/m5c-miss-review.mjs \
  --sample tools/pdf-benchmark/config/m5c-miss-sample.json \
  --manifest tools/pdf-benchmark/corpus/representative-manifest.json \
  --aggregate tools/pdf-benchmark/results/m5-full-representative.aggregate.json \
  --run tools/pdf-benchmark/results/m5-full-representative-a \
  --output tools/pdf-benchmark/results/m5c/miss-review
```

The JSON and Markdown packet remain ignored because they contain short local
extraction snippets. The tracked sample contains neutral document and anchor
IDs only.

Audit whether the frozen asset lock covers the three proposed provider roles:

```text
node tools/pdf-benchmark/scripts/m5c-platform-audit.mjs \
  --asset-lock tools/pdf-benchmark/config/assets.lock.json \
  --output tools/pdf-benchmark/results/m5c/platform-asset-audit.json
```

This is an asset-lock audit, not runtime proof. The manual
`pdf-benchmark-isolated-platforms.yml` workflow prepares assets online, then
builds and exercises relocated adapters with Cargo and model runtimes set to
offline mode. A platform is not considered validated until that workflow, or
an equivalent isolated host run, actually completes there. Signing and
notarization remain separate release gates.

## Metrics

The score report keeps raw dimensions separate. It includes:

- required and optional projection recall;
- page-qualified recall when explicit page provenance exists;
- structural recall for typed blocks and explicit reading-order constraints;
- expected/extracted pages, missing pages, and per-page character anomalies;
- character and benchmark-token coverage relative to Zotero text when present;
- gross-coverage and truncation flags;
- replacement, control, and private-use character counts;
- duplicate five-gram ratio;
- expected OCR-page recall, false OCR pages, and explicit index unavailability;
- projection and canonical hashes;
- benchmark quality and quality-gate false-good/false-degraded flags.

Coverage ratios are inspection signals, not proof that either output is the
ground truth. The annotated anchors remain the primary fidelity evidence.
Likewise, production quality `good` means that no material problem was detected
from available runtime signals; it is not proof of complete extraction. A
benchmark false-good requires external human ground truth showing a loss that
the runtime signals could not reveal.

## Canonical mapping

Adapters map into the parser-neutral benchmark document first, then into the
existing `CanonicalDocument v1`. Mapping loss is explicit in every envelope.
The common model and production Rust model both deserialize and validate the
unstructured and page/block/region fixtures in `fixtures/`. Production tests
also prove that a byte-identical canonical projection produces the same
existing scientific chunks and stable chunk IDs.
