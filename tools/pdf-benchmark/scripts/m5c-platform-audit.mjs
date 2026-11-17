#!/usr/bin/env node

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import process from "node:process";

const ROOT = resolve(".");
const RESULTS_ROOT = resolve("tools/pdf-benchmark/results");
const PLATFORMS = ["linux-x64", "win-x64", "mac-arm64"];

const options = parseArguments(process.argv.slice(2));
await auditPlatforms(options);

function parseArguments(argumentsList) {
  const values = new Map();
  for (let index = 0; index < argumentsList.length; index += 2) {
    const name = argumentsList[index];
    const value = argumentsList[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      throw new Error(usage());
    }
    values.set(name.slice(2), value);
  }
  const assetLock = values.get("asset-lock");
  const output = values.get("output");
  if (!assetLock || !output) {
    throw new Error(usage());
  }
  return {
    assetLock: resolve(assetLock),
    output: resolve(output),
  };
}

function usage() {
  return [
    "Audit M5-C role assets across the supported desktop platforms.",
    "",
    "The command reads the frozen asset lock, checks whether each role has",
    "portable models plus its required platform runtime, and writes a local",
    "privacy-safe report. It does not download assets or claim runtime proof.",
    "",
    "Processing flow:",
    "  1. Locate every locked candidate and platform-specific runtime.",
    "  2. Check the Xberg native, PDF Inspector OCR, and Docling structure roles.",
    "  3. Record missing lock coverage separately from unexecuted runtime tests.",
    "  4. Write the audit below the ignored benchmark results directory.",
    "",
    "Usage:",
    "  node tools/pdf-benchmark/scripts/m5c-platform-audit.mjs \\",
    "    --asset-lock ASSETS_LOCK_JSON --output LOCAL_RESULT_JSON",
  ].join("\n");
}

async function auditPlatforms(input) {
  assertIgnoredOutput(input.output);
  const lock = JSON.parse(await readFile(input.assetLock, "utf8"));
  if (lock.schemaVersion !== 1 || !Array.isArray(lock.candidates)) {
    throw new Error("Unsupported benchmark asset-lock schema.");
  }
  const candidates = new Map(
    lock.candidates.map((candidate) => [candidate.candidateId, candidate]),
  );
  const matrix = PLATFORMS.flatMap((platform) => [
    xbergPrimary(candidates, platform),
    pdfInspectorOcr(candidates, platform),
    doclingStructure(candidates, platform),
  ]);
  const report = {
    schemaVersion: 1,
    scope: "asset-lock coverage only; runtime execution is a separate gate",
    assetLock: relative(ROOT, input.assetLock),
    platforms: PLATFORMS,
    matrix,
  };
  await mkdir(dirname(input.output), { recursive: true });
  await writeFile(input.output, `${JSON.stringify(report, null, 2)}\n`);
  process.stdout.write(`${renderTable(matrix)}\n`);
  process.stdout.write(`wrote ${relative(ROOT, input.output)}\n`);
}

function xbergPrimary(candidates, platform) {
  requireCandidate(candidates, "xberg");
  return roleResult({
    platform,
    role: "xberg-low-cost-native-primary",
    requiredAssetIds: [],
    missingAssetClasses: [],
    note: "Native extraction has no runtime model or native-library asset.",
  });
}

function pdfInspectorOcr(candidates, platform) {
  const candidate = requireCandidate(candidates, "pdf-inspector");
  return nativeRuntimeRole({
    candidate,
    platform,
    role: "pdf-inspector-scanned-page-ocr-fallback",
    portableAssetIds: [
      "pp-ocrv6-small-detection",
      "pp-ocrv6-small-recognition",
      "pp-ocrv6-dictionary",
    ],
  });
}

function doclingStructure(candidates, platform) {
  const candidate = requireCandidate(candidates, "docling");
  return nativeRuntimeRole({
    candidate,
    platform,
    role: "docling-structure-on-demand",
    portableAssetIds: [
      "docling-layout-heron",
      "docling-ocr-recognition",
      "docling-ocr-dictionary",
      "docling-tableformer-encoder",
      "docling-tableformer-decoder",
      "docling-tableformer-decoder-data",
      "docling-tableformer-bbox",
      "docling-tableformer-bbox-data",
    ],
  });
}

function nativeRuntimeRole({ candidate, platform, role, portableAssetIds }) {
  const lockedIds = new Set(candidate.assets.map((asset) => asset.id));
  const missingPortable = portableAssetIds.filter((id) => !lockedIds.has(id));
  const platformAssets = candidate.assets.filter((asset) =>
    asset.platforms?.includes(platform),
  );
  const pdfium = platformAssets.find((asset) => asset.id.includes("pdfium"));
  const onnxruntime = platformAssets.find((asset) =>
    asset.id.includes("onnxruntime"),
  );
  const missingClasses = [];
  if (!pdfium) missingClasses.push("pdfium");
  if (!onnxruntime) missingClasses.push("onnxruntime");
  return roleResult({
    platform,
    role,
    requiredAssetIds: [
      ...portableAssetIds.filter((id) => lockedIds.has(id)),
      ...platformAssets.map((asset) => asset.id),
    ],
    missingAssetIds: missingPortable,
    missingAssetClasses: missingClasses,
    note: "Build, relocation, and offline runtime still require execution.",
  });
}

function roleResult({
  platform,
  role,
  requiredAssetIds,
  missingAssetIds = [],
  missingAssetClasses,
  note,
}) {
  return {
    platform,
    role,
    assetLockComplete:
      missingAssetIds.length === 0 && missingAssetClasses.length === 0,
    requiredAssetIds,
    missingAssetIds,
    missingAssetClasses,
    runtimeStatus: "pending-platform-execution",
    note,
  };
}

function requireCandidate(candidates, candidateId) {
  const candidate = candidates.get(candidateId);
  if (!candidate) {
    throw new Error(`Missing candidate ${candidateId} in asset lock.`);
  }
  return candidate;
}

function assertIgnoredOutput(output) {
  const relativeOutput = relative(RESULTS_ROOT, output);
  if (relativeOutput.startsWith("..") || relativeOutput === "") {
    throw new Error(
      "Platform audit output must remain below tools/pdf-benchmark/results.",
    );
  }
}

function renderTable(matrix) {
  const lines = [
    "Platform     Role                                      Asset lock",
    "------------ ----------------------------------------- ----------",
  ];
  for (const result of matrix) {
    const status = result.assetLockComplete ? "READY" : "INCOMPLETE";
    lines.push(
      `${result.platform.padEnd(12)} ${result.role.padEnd(41)} ${status}`,
    );
    if (!result.assetLockComplete) {
      const missing = [
        ...result.missingAssetIds,
        ...result.missingAssetClasses,
      ].join(", ");
      lines.push(`  missing: ${missing}`);
    }
  }
  return lines.join("\n");
}
