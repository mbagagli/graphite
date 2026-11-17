import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import process from "node:process";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const bundle = resolve(root, "evaluation/upstream-rag-v1");

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function json(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

function unique(values, label) {
  assert.equal(new Set(values).size, values.length, `Duplicate ${label}`);
}

export function validateReviewGate(protocol, key) {
  assert.ok(
    !protocol.scoredRunAllowed ||
      key.judgments.every(
        (entry) => entry.verificationStatus === "human-verified",
      ),
    "Scoring requires human approval of every judgment",
  );
}

export async function loadBundle() {
  const locks = await json(resolve(bundle, "checksums.json"));
  const required = [
    "corpus.json",
    "queries.json",
    "review-key.json",
    "protocol.json",
  ];
  assert.deepEqual(
    locks.files.map((entry) => entry.path).sort(),
    required.sort(),
    "Unexpected bundle lock entries",
  );
  for (const entry of locks.files) {
    assert.equal(
      digest(await readFile(resolve(bundle, entry.path))),
      entry.sha256,
      `Checksum mismatch: ${entry.path}`,
    );
  }
  const corpus = await json(resolve(bundle, "corpus.json"));
  const queries = await json(resolve(bundle, "queries.json"));
  const key = await json(resolve(bundle, "review-key.json"));
  const protocol = await json(resolve(bundle, "protocol.json"));
  unique(
    corpus.documents.map((doc) => doc.id),
    "document",
  );
  unique(
    queries.questions.map((query) => query.id),
    "query",
  );
  unique(
    key.judgments.map((entry) => entry.queryId),
    "judgment",
  );
  const docs = new Set(corpus.documents.map((doc) => doc.id));
  for (const query of queries.questions) {
    assert.deepEqual(
      Object.keys(query).sort(),
      ["id", "selectedDocumentIds", "category", "question"].sort(),
      "Retriever inputs must not contain answer or relevance fields",
    );
    assert.ok(
      query.selectedDocumentIds.length >= 1 &&
        query.selectedDocumentIds.length <= 5,
      "Invalid selection size",
    );
    unique(query.selectedDocumentIds, "selected document");
    assert.ok(
      query.selectedDocumentIds.every((id) => docs.has(id)),
      "Unknown selected document",
    );
    const judgment = key.judgments.find((entry) => entry.queryId === query.id);
    assert.ok(judgment, "Missing judgment");
    assert.ok(
      ["suggested-unverified", "human-verified", "human-rejected"].includes(
        judgment.verificationStatus,
      ),
      "Unknown review status",
    );
    for (const evidence of judgment.evidence.concat(judgment.additionalPages)) {
      assert.ok(
        query.selectedDocumentIds.includes(evidence.documentId),
        "Expected evidence is outside selection",
      );
    }
  }
  assert.equal(key.judgments.length, queries.questions.length);
  assert.deepEqual(
    protocol.queryOrder,
    queries.questions.map((q) => q.id),
  );
  assert.deepEqual(protocol.switchSequence, ["q13", "q14", "q15"]);
  validateReviewGate(protocol, key);
  return { corpus, queries, key, protocol };
}

export async function verifyCorpus(corpus) {
  for (const doc of corpus.documents) {
    const manifestPath = resolve(root, doc.manifest);
    assert.equal(
      digest(await readFile(manifestPath)),
      doc.manifestSha256,
      `${doc.id}: source manifest changed`,
    );
    const manifest = await json(manifestPath);
    const source = manifest.documents.find((entry) => entry.id === doc.id);
    assert.ok(source, `${doc.id}: source document missing`);
    assert.equal(source.sha256, doc.pdfSha256);
    assert.equal(source.anchorsSha256, doc.anchorsSha256);
    for (const [path, hash] of [
      [source.file, doc.pdfSha256],
      [source.anchors, doc.anchorsSha256],
      ...(source.zoteroBaseline
        ? [[source.zoteroBaseline, source.zoteroBaselineSha256]]
        : []),
    ]) {
      assert.equal(
        digest(await readFile(resolve(dirname(manifestPath), path))),
        hash,
        `${doc.id}: input checksum mismatch`,
      );
    }
  }
  return corpus.documents.length;
}

// Check observable evidence boundaries; never infer scientific correctness.
export function inspectResults(
  corpus,
  queries,
  run,
  evaluationId = "upstream-rag-v1",
) {
  assert.equal(run.evaluationId, evaluationId);
  assert.equal(run.executionKind, "measured", "Reject fabricated runs");
  assert.ok(typeof run.profile === "string" && run.profile.length > 0);
  assert.ok(
    typeof run.softwareIdentity === "string" && run.softwareIdentity.length > 0,
    "Missing pinned runtime identity",
  );
  assert.ok(Array.isArray(run.results));
  unique(
    run.results.map((row) => row.queryId),
    "result",
  );
  assert.deepEqual(
    run.results.map((row) => row.queryId),
    queries.questions.map((query) => query.id),
    "Results must preserve the complete execution order",
  );
  const docs = new Map(corpus.documents.map((doc) => [doc.id, doc]));
  return run.results.map((row, index) => {
    const query = queries.questions[index];
    assert.ok(["succeeded", "failed", "unavailable"].includes(row.status));
    const issues = [];
    if (row.status !== "succeeded") {
      assert.ok(typeof row.reason === "string" && row.reason.length > 0);
      return {
        queryId: row.queryId,
        status: row.status,
        issues: [row.reason],
        answerCorrectness: "not-evaluated",
      };
    }
    assert.equal(typeof row.answer, "string");
    assert.ok(row.answer.trim().length > 0, "Empty successful answer");
    assert.equal(typeof row.abstained, "boolean");
    assert.ok(Array.isArray(row.evidence));
    unique(
      row.evidence.map((entry) => entry.id),
      "evidence",
    );
    const evidence = new Map(row.evidence.map((entry) => [entry.id, entry]));
    for (const item of row.evidence) {
      const doc = docs.get(item.documentId);
      assert.ok(doc, "Evidence names an unknown document");
      assert.equal(
        item.documentSha256,
        doc.pdfSha256,
        "Evidence revision differs from the frozen document",
      );
      assert.ok(
        typeof item.passage === "string" && item.passage.trim(),
        "Evidence must include the actual passage",
      );
      if (item.pageStart === null && item.pageEnd === null) {
        issues.push(`unknown-page:${item.id}`);
      } else {
        assert.ok(
          Number.isInteger(item.pageStart) &&
            Number.isInteger(item.pageEnd) &&
            item.pageStart >= 1 &&
            item.pageEnd >= item.pageStart &&
            item.pageEnd <= doc.expectedPages,
          "Invalid physical PDF page range",
        );
      }
    }
    for (const stage of [
      "retrievedEvidenceIds",
      "contextEvidenceIds",
      "citationEvidenceIds",
    ]) {
      assert.ok(Array.isArray(row[stage]), `Missing ${stage}`);
      for (const id of row[stage]) {
        const item = evidence.get(id);
        assert.ok(item, "Unknown evidence reference");
        if (!query.selectedDocumentIds.includes(item.documentId)) {
          issues.push(`scope-leak:${stage}:${id}`);
        }
      }
    }
    for (const id of row.contextEvidenceIds) {
      assert.ok(
        row.retrievedEvidenceIds.includes(id),
        "Context evidence was not retrieved",
      );
    }
    for (const id of row.citationEvidenceIds) {
      assert.ok(
        row.contextEvidenceIds.includes(id),
        "Cited evidence was not supplied as context",
      );
    }
    if (!row.abstained && row.citationEvidenceIds.length === 0) {
      issues.push("uncited-answer");
    }
    assert.ok(
      Number.isInteger(row.selectionReindexCount) &&
        row.selectionReindexCount >= 0,
      "Missing selection indexing counter",
    );
    if (row.selectionReindexCount !== 0) issues.push("selection-reindexed");
    return {
      queryId: row.queryId,
      status: row.status,
      issues,
      answerCorrectness: "requires-independent-review",
    };
  });
}

async function main(args) {
  if (args.includes("--help")) {
    process.stdout.write(`Graphite upstream comparison preparation checker

Validate the separate six-paper evaluation and optional measured outputs.
Inputs: frozen corpus files, comparison JSON, optional local result JSON.
Output: integrity status or evidence-boundary findings; never a QA score.
Flow:
  1. Verify bundle locks and selection contracts.
  2. Optionally verify local PDFs, anchors and Zotero sidecars.
  3. Optionally inspect measured evidence for scope and page failures.
Defaults: no models, network, installs, source writes or inference.
Examples:
  node tools/rag-comparison/check.mjs --corpus
  node tools/rag-comparison/check.mjs --results /path/to/run.json
`);
    return;
  }
  assert.ok(
    args.every(
      (arg, index) =>
        ["--corpus", "--results"].includes(arg) ||
        (index > 0 && args[index - 1] === "--results"),
    ),
    "Unknown option",
  );
  const { corpus, queries, key } = await loadBundle();
  const report = {
    phase: "preparation",
    documents: corpus.documents.length,
    questions: queries.questions.length,
    pendingReview: key.judgments.filter(
      (entry) => entry.verificationStatus === "suggested-unverified",
    ).length,
  };
  if (args.includes("--corpus")) {
    report.verifiedDocuments = await verifyCorpus(corpus);
  }
  if (args.includes("--results")) {
    const path = args[args.indexOf("--results") + 1];
    assert.ok(path && !path.startsWith("--"), "Missing results path");
    report.observations = inspectResults(corpus, queries, await json(path));
  }
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  main(process.argv.slice(2)).catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
