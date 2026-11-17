import { expect, it } from "vitest";
import { ignoredPublicationPaths } from "./check-publication.mjs";

it("excludes local plans, reports, tool state and unknown root material", () => {
  const paths = [
    "AGENTS.md",
    "ROADMAP.md",
    "GOAL.md",
    "BIGPLAN.md",
    "PRODUCT.md",
    ".agents/state.json",
    ".codex/config.toml",
    ".impeccable/design.json",
    "docs/release/HANDOFF.md",
    "docs/release/LEDGER.md",
    "website/DESIGN.md",
    "notes/private.txt",
    "archive/config",
    "evaluation/upstream-rag-v1/run.json",
    "src/AGENTS.md",
    "src/ARCHITECTURE.md",
    "website/PRODUCT.md",
    "tools/example/.venv/bin/python",
    "tools/example/venv/bin/python",
    "src-tauri/signing/private.key",
    "src-tauri/signing/certificate.pem",
    "src-tauri/signing/identity.p12",
    "src-tauri/signing/identity.pfx",
    "artifacts/private-output.json",
    ".env",
  ];
  expect(ignoredPublicationPaths(paths)).toEqual(paths);
});

it("retains sources, public docs, workflows and required evaluation fixtures", () => {
  expect(
    ignoredPublicationPaths([
      "README.md",
      "LICENSE",
      "PRIVACY.md",
      "package-lock.json",
      ".github/workflows/ci.yml",
      ".github/workflows/release.yml",
      ".cargo/config.toml",
      "src/app/App.tsx",
      "src-tauri/Cargo.lock",
      "src-tauri/tauri.conf.json",
      "src-tauri/tauri.macos.conf.json",
      "src-tauri/tauri.linux.conf.json",
      "scripts/check-release-tag.mjs",
      "scripts/cuda-toolchain.lock.json",
      "docs/release/BUILD.md",
      "docs/library-preparation.md",
      "website/assets/workspace.png",
      "demo/manifest.json",
      "tools/rag-comparison/docling_worker.py",
      "tools/pdf-benchmark/fixtures/canonical-mapped.json",
      "evaluation/upstream-rag-v1/corpus.json",
      "evaluation/pdf-search-v1/qa-manifest-v1.json",
    ]),
  ).toEqual([]);
});
