import assert from "node:assert/strict";
import { test } from "node:test";
import { inspectResults, loadBundle, validateReviewGate } from "./check.mjs";

const corpus = {
  documents: [
    { id: "a", pdfSha256: "revision-a", expectedPages: 3 },
    { id: "b", pdfSha256: "revision-b", expectedPages: 2 },
  ],
};
const queries = {
  questions: [
    { id: "q1", selectedDocumentIds: ["a"] },
    { id: "q2", selectedDocumentIds: ["b"] },
  ],
};

// Synthetic observations test the checker, not any RAG implementation.
function run() {
  return {
    evaluationId: "upstream-rag-v1",
    executionKind: "measured",
    profile: "unit-test-only",
    softwareIdentity: "synthetic-unit-test",
    results: queries.questions.map((query, index) => ({
      queryId: query.id,
      status: "succeeded",
      answer: "Test claim [e1].",
      abstained: false,
      selectionReindexCount: 0,
      evidence: [
        {
          id: "e1",
          documentId: corpus.documents[index].id,
          documentSha256: corpus.documents[index].pdfSha256,
          pageStart: 1,
          pageEnd: 1,
          passage: "Synthetic supporting passage",
        },
      ],
      retrievedEvidenceIds: ["e1"],
      contextEvidenceIds: ["e1"],
      citationEvidenceIds: ["e1"],
    })),
  };
}

test("prepared bundle has six papers, eighteen human-approved questions", async () => {
  const data = await loadBundle();
  assert.equal(data.corpus.documents.length, 6);
  assert.equal(data.queries.questions.length, 18);
  assert.equal(data.protocol.scoredRunAllowed, true);
  assert.ok(
    data.key.judgments.every(
      (entry) => entry.verificationStatus === "human-verified",
    ),
  );
  const heading = data.key.judgments.find((entry) => entry.queryId === "q11");
  assert.ok(heading.verificationNote.includes("30"));
  const sequence = data.protocol.switchSequence.map((id) =>
    data.queries.questions.find((query) => query.id === id),
  );
  assert.ok(sequence.every((query) => query.question === sequence[0].question));
  assert.deepEqual(
    sequence[0].selectedDocumentIds,
    sequence[2].selectedDocumentIds,
  );
  assert.ok(!sequence[1].selectedDocumentIds.includes("paper20"));
});

test("scoring rejects pending and rejected reference judgments", () => {
  for (const verificationStatus of ["suggested-unverified", "human-rejected"]) {
    const key = { judgments: [{ verificationStatus }] };
    assert.throws(() => validateReviewGate({ scoredRunAllowed: true }, key));
    assert.doesNotThrow(() =>
      validateReviewGate({ scoredRunAllowed: false }, key),
    );
  }
});

test("valid citations do not assert scientific answer correctness", () => {
  const rows = inspectResults(corpus, queries, run());
  assert.ok(rows.every((row) => row.issues.length === 0));
  assert.ok(
    rows.every(
      (row) => row.answerCorrectness === "requires-independent-review",
    ),
  );
});

test("removed-paper evidence is detected even if the citation is filtered", () => {
  const candidate = run();
  candidate.results[1].evidence.push(candidate.results[0].evidence[0]);
  candidate.results[1].evidence[1] = {
    ...candidate.results[1].evidence[1],
    id: "stale",
  };
  candidate.results[1].retrievedEvidenceIds.push("stale");
  const rows = inspectResults(corpus, queries, candidate);
  assert.deepEqual(rows[1].issues, ["scope-leak:retrievedEvidenceIds:stale"]);
});

test("context and citation leakage are independently reported", () => {
  const candidate = run();
  candidate.results[1].evidence = candidate.results[0].evidence;
  const rows = inspectResults(corpus, queries, candidate);
  assert.equal(
    rows[1].issues.filter((issue) => issue.startsWith("scope-leak:")).length,
    3,
  );
});

test("missing pages are failures of evidence precision, not invented page one", () => {
  const candidate = run();
  candidate.results[0].evidence[0].pageStart = null;
  candidate.results[0].evidence[0].pageEnd = null;
  assert.deepEqual(inspectResults(corpus, queries, candidate)[0].issues, [
    "unknown-page:e1",
  ]);
});

test("invalid page ranges and stale revisions are rejected", () => {
  for (const patch of [
    { pageStart: 0 },
    { pageEnd: 4 },
    { pageStart: 2, pageEnd: 1 },
    { pageEnd: null },
    { documentSha256: "old-revision" },
  ]) {
    const candidate = run();
    Object.assign(candidate.results[0].evidence[0], patch);
    assert.throws(() => inspectResults(corpus, queries, candidate));
  }
});

test("missing, duplicate and reordered result rows are rejected", () => {
  for (const mutate of [
    (rows) => rows.pop(),
    (rows) => rows.push(rows[0]),
    (rows) => rows.reverse(),
  ]) {
    const candidate = run();
    mutate(candidate.results);
    assert.throws(() => inspectResults(corpus, queries, candidate));
  }
});

test("unknown citations and context not retrieved are rejected", () => {
  const candidate = run();
  candidate.results[0].citationEvidenceIds = ["invented"];
  assert.throws(() => inspectResults(corpus, queries, candidate));
  const other = run();
  other.results[0].retrievedEvidenceIds = [];
  assert.throws(() => inspectResults(corpus, queries, other));
});

test("selection-triggered reindexing is reported", () => {
  const candidate = run();
  candidate.results[1].selectionReindexCount = 1;
  assert.deepEqual(inspectResults(corpus, queries, candidate)[1].issues, [
    "selection-reindexed",
  ]);
});

test("failure remains visible and is never counted as a good abstention", () => {
  const candidate = run();
  candidate.results[0] = {
    queryId: "q1",
    status: "unavailable",
    reason: "Local model is not installed",
  };
  const row = inspectResults(corpus, queries, candidate)[0];
  assert.equal(row.status, "unavailable");
  assert.equal(row.answerCorrectness, "not-evaluated");
});

test("oracle output cannot masquerade as a measured run", () => {
  const candidate = run();
  candidate.executionKind = "oracle";
  assert.throws(() => inspectResults(corpus, queries, candidate));
});

test("a fresh evaluation requires its explicit expected identity", () => {
  const candidate = run();
  candidate.evaluationId = "docling-app-v1";
  assert.throws(() => inspectResults(corpus, queries, candidate));
  assert.equal(
    inspectResults(corpus, queries, candidate, "docling-app-v1").length,
    queries.questions.length,
  );
});
