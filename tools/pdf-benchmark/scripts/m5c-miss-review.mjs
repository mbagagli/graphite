#!/usr/bin/env node

import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import process from "node:process";

const ROOT = resolve(".");
const RESULTS_ROOT = resolve("tools/pdf-benchmark/results");
const PROVIDERS = [
  ["xberg", "recommended-auto"],
  ["pdf-inspector", "recommended-auto"],
  ["docling", "recommended-auto"],
  ["zotero", "native"],
];
const SNIPPET_RADIUS = 120;

const options = parseArguments(process.argv.slice(2));
await createPacket(options);

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
  const required = ["sample", "manifest", "aggregate", "run", "output"];
  for (const name of required) {
    if (!values.has(name)) {
      throw new Error(`Missing --${name}.\n\n${usage()}`);
    }
  }
  return Object.fromEntries(
    required.map((name) => [name, resolve(values.get(name))]),
  );
}

function usage() {
  return [
    "Create a local M5-C miss-adjudication packet.",
    "",
    "The command joins a tracked neutral sample with frozen benchmark scores,",
    "human-verified annotations, and ignored raw extraction envelopes. It writes",
    "small source-text snippets only below tools/pdf-benchmark/results.",
    "",
    "Processing flow:",
    "  1. Validate the neutral sample and human-verified anchors.",
    "  2. Join frozen deterministic scoring outcomes for four providers.",
    "  3. Find a short relevant snippet without changing any score.",
    "  4. Write local JSON and Markdown packets for human adjudication.",
    "",
    "Usage:",
    "  node tools/pdf-benchmark/scripts/m5c-miss-review.mjs \\",
    "    --sample SAMPLE_JSON --manifest MANIFEST_JSON \\",
    "    --aggregate AGGREGATE_JSON --run RAW_RUN_DIR \\",
    "    --output RESULTS_OUTPUT_PREFIX",
  ].join("\n");
}

async function createPacket(input) {
  assertIgnoredOutput(input.output);
  const sample = await readJson(input.sample);
  const manifest = await readJson(input.manifest);
  const aggregate = await readJson(input.aggregate);
  validateSample(sample);
  const documents = new Map(
    manifest.documents.map((document) => [document.id, document]),
  );
  const scoreIndex = new Map(
    aggregate.reports.map((report) => [attemptKey(report), report]),
  );
  const anchorFiles = new Map();
  const cases = [];
  for (const selectedCase of sample.cases) {
    const document = documents.get(selectedCase.documentId);
    if (!document) {
      throw new Error(`Unknown document ${selectedCase.documentId}.`);
    }
    const annotationPath = resolve(dirname(input.manifest), document.anchors);
    let annotation = anchorFiles.get(annotationPath);
    if (!annotation) {
      annotation = await readJson(annotationPath);
      anchorFiles.set(annotationPath, annotation);
    }
    const anchor = annotation.anchors.find(
      (entry) => entry.id === selectedCase.anchorId,
    );
    if (!anchor) {
      throw new Error(`Unknown anchor ${selectedCase.anchorId}.`);
    }
    if (anchor.verificationStatus !== "human-verified" || !anchor.required) {
      throw new Error(
        `${anchor.id} must be required and human-verified for review.`,
      );
    }
    const results = [];
    for (const [candidateId, profile] of PROVIDERS) {
      const score = scoreIndex.get(
        `${selectedCase.documentId}\0${candidateId}\0${profile}`,
      );
      if (!score) {
        throw new Error(
          `Missing score for ${selectedCase.documentId}/${candidateId}/${profile}.`,
        );
      }
      const anchorResult = score.anchorResults.find(
        (entry) => entry.anchorId === anchor.id,
      );
      if (!anchorResult) {
        throw new Error(`Missing score for anchor ${anchor.id}.`);
      }
      const envelopePath = resolve(
        input.run,
        candidateId,
        profile,
        `${selectedCase.documentId}.json`,
      );
      const envelope = await readJson(envelopePath);
      results.push({
        candidateId,
        profile,
        outcome: score.outcome,
        projectionMatched: anchorResult.projectionMatched,
        pageQualifiedMatched: anchorResult.pageQualifiedMatched,
        structuralMatched: anchorResult.structuralMatched,
        scorerReason: anchorResult.reason,
        relevantSnippet: findRelevantSnippet(
          envelope.searchableProjection ?? "",
          anchor.matcher,
        ),
      });
    }
    cases.push({
      documentId: selectedCase.documentId,
      anchorId: anchor.id,
      stratum: selectedCase.stratum,
      anchorKind: anchor.kind,
      expectedPages: anchor.expectedPages,
      humanVerifiedExpectation: matcherSummary(anchor.matcher),
      results,
      adjudication: {
        status: "pending-human-review",
        allowedDecisions: [
          "real-extraction-loss",
          "representation-or-layout-loss",
          "deterministic-matcher-false-negative",
          "unclear",
        ],
        decision: null,
        note: null,
      },
    });
  }
  const packet = {
    schemaVersion: 1,
    packetId: sample.sampleId,
    sourceAggregateSha256: await sha256(input.aggregate),
    caseCount: cases.length,
    scoreMutationPolicy: "review-only; frozen scores remain unchanged",
    privacy: "local-only; contains small extracted source-text snippets",
    cases,
  };
  await mkdir(dirname(input.output), { recursive: true });
  await writeFile(
    `${input.output}.json`,
    `${JSON.stringify(packet, null, 2)}\n`,
  );
  await writeFile(`${input.output}.md`, renderMarkdown(packet));
  process.stdout.write(
    `wrote ${cases.length} local review cases to ${relative(ROOT, input.output)}.{json,md}\n`,
  );
}

function validateSample(sample) {
  if (sample.schemaVersion !== 1 || !Array.isArray(sample.cases)) {
    throw new Error("Unsupported M5-C miss-sample schema.");
  }
  if (sample.cases.length < 15 || sample.cases.length > 20) {
    throw new Error("M5-C review sample must contain 15 to 20 cases.");
  }
  const keys = new Set();
  for (const selectedCase of sample.cases) {
    const key = `${selectedCase.documentId}\0${selectedCase.anchorId}`;
    if (keys.has(key)) {
      throw new Error(`Duplicate review case ${selectedCase.anchorId}.`);
    }
    keys.add(key);
  }
}

function assertIgnoredOutput(output) {
  const relativeOutput = relative(RESULTS_ROOT, output);
  if (relativeOutput.startsWith("..") || relativeOutput === "") {
    throw new Error(
      "Review packets must remain below tools/pdf-benchmark/results.",
    );
  }
}

function attemptKey(report) {
  return `${report.documentId}\0${report.candidateId}\0${report.profile}`;
}

function matcherSummary(matcher) {
  switch (matcher.type) {
    case "exact":
    case "normalized":
      return `${matcher.type}: ${matcher.value}`;
    case "ordered_tokens":
      return `ordered tokens: ${matcher.tokens.join(" ... ")}`;
    case "numeric_context":
      return [
        `numeric value: ${matcher.value}`,
        matcher.unit ? `unit: ${matcher.unit}` : null,
        `context: ${matcher.context.join(", ")}`,
      ]
        .filter(Boolean)
        .join("; ");
    default:
      throw new Error(`Unsupported matcher ${matcher.type}.`);
  }
}

function findRelevantSnippet(projection, matcher) {
  const normalized = normalizeWhitespace(projection);
  if (!normalized) return null;
  const probes = matcherProbes(matcher);
  for (const probe of probes) {
    const index = normalized
      .toLocaleLowerCase("en")
      .indexOf(probe.toLocaleLowerCase("en"));
    if (index >= 0) {
      const start = Math.max(0, index - SNIPPET_RADIUS);
      const end = Math.min(
        normalized.length,
        index + probe.length + SNIPPET_RADIUS,
      );
      return `${start > 0 ? "..." : ""}${normalized.slice(start, end)}${
        end < normalized.length ? "..." : ""
      }`;
    }
  }
  return null;
}

function matcherProbes(matcher) {
  const probes = [];
  if (matcher.value) probes.push(normalizeWhitespace(matcher.value));
  if (matcher.tokens) probes.push(...matcher.tokens);
  if (matcher.context) probes.push(...matcher.context);
  if (matcher.unit) probes.push(matcher.unit);
  return [...new Set(probes.map(normalizeWhitespace).filter(Boolean))].sort(
    (left, right) => right.length - left.length || left.localeCompare(right),
  );
}

function normalizeWhitespace(value) {
  return String(value).replace(/\s+/gu, " ").trim();
}

function renderMarkdown(packet) {
  const lines = [
    "# M5-C local miss-adjudication packet",
    "",
    `Cases: ${packet.caseCount}`,
    "",
    "This file is local-only because it contains short source-text snippets.",
    "Review does not mutate the frozen M5 scores.",
    "",
  ];
  for (const selectedCase of packet.cases) {
    lines.push(
      `## ${selectedCase.documentId} / ${selectedCase.anchorId}`,
      "",
      `- Stratum: ${selectedCase.stratum}`,
      `- Type/page: ${selectedCase.anchorKind} / ${selectedCase.expectedPages.join(", ")}`,
      `- Human-verified expectation: ${selectedCase.humanVerifiedExpectation}`,
      "- Human decision: pending",
      "",
    );
    for (const result of selectedCase.results) {
      lines.push(
        `### ${result.candidateId} / ${result.profile}`,
        "",
        `- Outcome: ${result.outcome}`,
        `- Projection/page/structure: ${result.projectionMatched} / ${result.pageQualifiedMatched} / ${result.structuralMatched}`,
        `- Scorer reason: ${result.scorerReason}`,
        `- Relevant snippet: ${result.relevantSnippet ?? "not found"}`,
        "",
      );
    }
  }
  return `${lines.join("\n")}\n`;
}

async function readJson(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

async function sha256(path) {
  const content = await readFile(path);
  return createHash("sha256").update(content).digest("hex");
}
