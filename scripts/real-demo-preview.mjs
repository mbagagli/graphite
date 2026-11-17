import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, relative, resolve } from "node:path";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath, URL } from "node:url";

import { artifactDirectory, prepareQaRuntime } from "./qa-runtime.mjs";

const replayRag = process.argv[3] === "--rag-replay";
if (
  process.argv[2] === "--help" ||
  !(process.argv.length === 3 || (replayRag && process.argv.length === 5))
) {
  process.stdout.write(
    "Graphite verified collection metadata preview\n\n" +
      "Usage: node scripts/real-demo-preview.mjs <workspace.json>\n" +
      "       node scripts/real-demo-preview.mjs <workspace-indexed.json>\n" +
      "         --rag-replay <verified-pdf-directory>\n" +
      "Input: the real-demo native import test's exported workspace.\n" +
      "Flow: validate manifest identity, render the current app in an isolated\n" +
      "browser, then capture publication-year metadata. Start npm run dev first.\n" +
      "Outputs: artifacts/qa/real-demo-preview image and provenance sidecar.\n" +
      "RAG replay checks saved native responses, citation navigation, and the\n" +
      "matching checksum-verified PDF; evidence: artifacts/qa/real-demo-citations.\n" +
      "The preview does not verify a packaged runtime or model answer.\n",
  );
  process.exit(process.argv[2] === "--help" ? 0 : 1);
}

const root = fileURLToPath(new URL("../", import.meta.url));
const manifest = JSON.parse(
  await readFile(new URL("../demo/manifest.json", import.meta.url), "utf8"),
);
const input = await readFile(process.argv[2], "utf8");
const workspace = JSON.parse(input);
const nativeEvidence = JSON.parse(
  await readFile(resolve(dirname(process.argv[2]), "evidence.json"), "utf8"),
);
assert.equal(nativeEvidence.manifestId, manifest.id);
assert.equal(nativeEvidence.pdfsResolvedAndChecksummed, 60);
assert.equal(nativeEvidence.duplicateSyncPreservedIds, true);
assert.ok(
  ["zotero-local-cache", "real-demo"].includes(workspace.snapshot.provenance),
);
assert.equal(workspace.snapshot.items.length, 60);
assert.equal(workspace.snapshot.totalItemCount, 60);
assert.equal(workspace.corpus.fullTextCount, 60);
assert.equal(workspace.corpus.resolvedPdfCount, 60);
assert.deepEqual(
  workspace.snapshot.items.map((paper) => paper.doi).sort(),
  manifest.papers.map((paper) => paper.doi).sort(),
);

const suiteName = replayRag ? "real-demo-citations" : "real-demo-preview";
const outputDirectory = artifactDirectory(suiteName);
await mkdir(outputDirectory, { recursive: true });
let replay;
if (replayRag) {
  const [supported, unsupported] = await Promise.all(
    ["rag-supported.json", "rag-unsupported.json"].map(async (name) =>
      JSON.parse(
        await readFile(resolve(dirname(process.argv[2]), name), "utf8"),
      ),
    ),
  );
  assert.ok(supported.citations.length > 0);
  assert.equal(unsupported.citations.length, 0);
  assert.match(unsupported.answerMarkdown, /insufficient evidence/i);
  const documents = {};
  await mkdir(resolve(outputDirectory, "pdfs"), { recursive: true });
  for (const citation of supported.citations) {
    const source = supported.sources.find(
      (entry) => entry.sourceId === citation.sourceId,
    );
    assert.ok(source?.eligibleForScientificClaim);
    assert.ok(supported.answerMarkdown.includes(citation.marker));
    const item = workspace.snapshot.items.find(
      (paper) => paper.id === source.itemId,
    );
    assert.ok(
      item?.attachments.some(
        (attachment) => attachment.documentId === source.documentId,
      ),
    );
    const record = manifest.papers.find((paper) => paper.doi === item.doi);
    const pdf = await readFile(resolve(process.argv[4], `${record.id}.pdf`));
    assert.equal(createHash("sha256").update(pdf).digest("hex"), record.sha256);
    await writeFile(resolve(outputDirectory, "pdfs", `${record.id}.pdf`), pdf);
    documents[source.documentId] = {
      url: `./pdfs/${record.id}.pdf`,
      bytes: pdf.length,
      sha256: record.sha256,
      doi: record.doi,
      pages: record.pdfPages,
    };
  }
  replay = { supported, unsupported, documents };
  await writeFile(
    resolve(outputDirectory, "recorded-runs.json"),
    JSON.stringify(replay),
  );
}
const sourceFile = resolve(outputDirectory, "workspace.json");
const pageFile = resolve(outputDirectory, "preview.html");
await writeFile(sourceFile, input);
await writeFile(
  pageFile,
  `<!doctype html>
<html lang="en" data-theme="dark" data-theme-storage-key="graphite-preview-theme:v1"
  data-theme-dark-color="#292b2e" data-theme-light-color="#f4f3ef">
<head><meta charset="utf-8"><title>${replayRag ? "Recorded native response UI replay" : "Verified collection metadata preview"}</title>
<style>
#root { height: calc(100% - 28px) !important; }
.preview-caption { height: 28px; padding: 6px 20px; font-size: 11px;
  color: var(--color-text-secondary); background: var(--color-background-deep); }
</style></head>
<body><div id="root"></div>
<div class="preview-caption">${
    replayRag
      ? "Recorded native response UI replay - no model is running in this browser."
      : "Verified collection metadata preview - 60 real seismology papers. Browser rendering of the native import; no model answer shown."
  }</div>
<script>
window.addEventListener("error", event => {
  document.body.dataset.previewError = event.message;
});
window.addEventListener("unhandledrejection", event => {
  document.body.dataset.previewError = String(event.reason);
});
</script><script type="module">
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource-variable/inter/wght.css";
import "/src/design-system/tokens.css";
import "/src/design-system/global.css";
import "/src/features/library/library.css";
import "/src/features/search/search.css";
import "/src/features/reader/reader.css";
import "/src/features/agent/agent.css";
import { App } from "/src/app/App.tsx";
import { ThemeProvider } from "/src/design-system/ThemeProvider.tsx";
import { services } from "/src/services/runtime.ts";
const payload = await (await fetch("./workspace.json")).json();
services.workspace.load = async () => payload;
services.workspace.sync = async () => payload;
services.citations.index = async () => payload;
services.citations.reresolve = async () => payload;
services.library.loadSnapshot = async () => payload.snapshot;
services.library.getItem = async id =>
  payload.snapshot.items.find(paper => paper.id === id) ?? null;
services.library.query = async request => {
  const filtered = payload.snapshot.items.filter(paper =>
    (!request.collectionId || paper.collectionIds.includes(request.collectionId)) &&
    (request.yearFrom === undefined || paper.year >= request.yearFrom) &&
    (request.yearTo === undefined || paper.year <= request.yearTo));
  const offset = request.offset ?? 0;
  const limit = request.limit ?? 50;
  return { items: filtered.slice(offset, offset + limit),
    total: filtered.length, offset, limit };
};
services.analytics.compute = async () => payload.analytics;
services.graph.loadGraph = async () => payload.graph;
services.graph.loadProjection = async () => ({
  items: payload.snapshot.items, topics: payload.snapshot.topics,
  graph: payload.graph,
});
const unavailable = async () => {
  throw new Error("This metadata preview does not execute research tools.");
};
services.agent.run = unavailable;
services.search.search = unavailable;
services.document.resolve = unavailable;
services.document.getText = unavailable;
services.semantic.similar = unavailable;
services.semantic.neighborhood = unavailable;
${
  replayRag
    ? `
const replay = await (await fetch("./recorded-runs.json")).json();
window.__graphiteCitationReplay = { resolvedDocuments: [], readerStates: [] };
const previewEnvironment = services.agent.environment.bind(services.agent);
services.agent.environment = async () => ({
  ...await previewEnvironment(),
  settings: { ...(await previewEnvironment()).settings,
    providerKind: "openai-compatible", externalEndpoint: null,
    externalModel: "Recorded result only" },
  runtimeName: "Recorded native response UI replay",
  trustStatement: "No model is running. Only the two saved native responses are replayed.",
});
services.agent.run = async request => {
  const result = [replay.supported, replay.unsupported]
    .find(run => run.question === request.question);
  if (!result) throw new Error("Only the two recorded questions can be replayed.");
  if (!result.sources.some(source => source.itemId === request.context.currentItemId)) {
    throw new Error("Select the recorded answer's paper before replaying it.");
  }
  return result;
};
services.document.resolve = async documentId => {
  const record = replay.documents[documentId];
  if (!record) throw new Error("No verified replay PDF matches this document.");
  window.__graphiteCitationReplay.resolvedDocuments.push(documentId);
  return { documentId, status: "ready", mediaType: "application/pdf",
    contentLength: record.bytes, revisionKey: record.sha256,
    message: null, url: new URL(record.url, location.href).href };
};
services.document.getText = async () => null;
services.document.getReaderState = async () => null;
services.document.saveReaderState = async state => {
  window.__graphiteCitationReplay.readerStates.push(state);
};
services.document.doclingStatus = async () => ({
  available: false, message: "Extraction is disabled during response replay.",
  job: { documentId: "", status: "idle", message: "" }, choices: [],
});
services.document.indexExtraction = unavailable;
services.document.extractWithDocling = unavailable;
`
    : ""
}
createRoot(document.getElementById("root")).render(
  createElement(ThemeProvider, null, createElement(App)));
</script></body></html>`,
);

const runtime = await prepareQaRuntime(suiteName);
let sessionId;
async function evaluate(script, args = []) {
  return runtime.request("POST", `/session/${sessionId}/execute/sync`, {
    script,
    args,
  });
}

async function captureReplay(name) {
  const encoded = await runtime.request(
    "GET",
    `/session/${sessionId}/screenshot`,
  );
  await writeFile(
    resolve(runtime.outputDirectory, `${name}.png`),
    Buffer.from(encoded, "base64"),
  );
}

async function submitRecordedQuestion(question) {
  const element = await runtime.request(
    "POST",
    `/session/${sessionId}/element`,
    { using: "css selector", value: '[name="research-agent-query"]' },
  );
  const id = element["element-6066-11e4-a52e-4f735466cecf"];
  await runtime.request("POST", `/session/${sessionId}/element/${id}/value`, {
    text: question,
    value: [...question],
  });
  await evaluate(
    "document.querySelector('[aria-label=\"Submit question\"]').click();",
  );
}

async function runCitationReplay() {
  const { supported, unsupported, documents } = replay;
  const firstSource = supported.sources.find(
    (source) => source.sourceId === supported.citations[0].sourceId,
  );
  const firstPaper = workspace.snapshot.items.find(
    (paper) => paper.id === firstSource.itemId,
  );
  await evaluate(`
    document.querySelector('[aria-label="Switch to dark mode"]')?.click();
    document.querySelectorAll('.view-switcher button')[1].click();
  `);
  await waitFor("return document.querySelector('.year-chart > button');");
  await evaluate(
    `
    const year = arguments[0];
    [...document.querySelectorAll('.year-chart > button')]
      .find(button => button.getAttribute('aria-label').startsWith(year + ':')).click();
  `,
    [String(firstPaper.year)],
  );
  await waitFor("return document.querySelector('.year-papers-dialog[open]');");
  await evaluate(
    `
    [...document.querySelectorAll('.year-paper-list button')]
      .find(button => button.querySelector('strong').textContent === arguments[0]).click();
  `,
    [firstPaper.title],
  );
  await waitFor("return document.querySelector('.paper-inspector');");
  await evaluate("document.querySelector('.agent-launcher').click();");
  await waitFor(
    "return document.querySelector('[name=\"research-agent-query\"]');",
  );
  await submitRecordedQuestion(supported.question);
  await waitFor(
    "return document.querySelectorAll('.agent-inline-citation').length > 0;",
  );
  assert.equal(
    await evaluate(
      "return document.querySelector('.agent-answer-text').textContent;",
    ),
    supported.answerMarkdown,
  );
  await delay(400);
  await captureReplay("supported-answer");

  const checks = [];
  for (const citation of supported.citations) {
    const source = supported.sources.find(
      (entry) => entry.sourceId === citation.sourceId,
    );
    const paper = workspace.snapshot.items.find(
      (entry) => entry.id === source.itemId,
    );
    await evaluate("document.querySelector('.agent-tabs button').click();");
    await evaluate(
      `
      [...document.querySelectorAll('.agent-inline-citation')]
        .find(button => button.textContent === arguments[0]).focus();
    `,
      [citation.marker],
    );
    await runtime.request("POST", `/session/${sessionId}/actions`, {
      actions: [
        {
          type: "key",
          id: "citation-keyboard",
          actions: [
            { type: "keyDown", value: "\uE007" },
            { type: "keyUp", value: "\uE007" },
          ],
        },
      ],
    });
    await waitFor(
      "return document.querySelector('.agent-source-list article.is-expanded');",
    );
    const detail = await evaluate(`
      const entry = document.querySelector('.agent-source-list article.is-expanded');
      return {
        sourceId: entry.querySelector('.agent-source-heading > span').textContent,
        title: entry.querySelector('strong').textContent,
        passage: entry.querySelector('.agent-source-detail > p').textContent,
        pageLabel: entry.querySelector('small').textContent,
      };
    `);
    assert.equal(detail.sourceId, citation.sourceId);
    assert.equal(detail.title, paper.title);
    assert.equal(detail.passage, source.excerpt);
    assert.ok(
      detail.pageLabel.includes(
        source.pageStart ? `p. ${source.pageStart}` : "page unavailable",
      ),
    );
    await captureReplay(`source-${citation.sourceId}`);
    await evaluate(`document.querySelector(
      '.agent-source-list article.is-expanded .agent-source-actions button').click();`);
    await waitFor("return document.querySelector('.paper-inspector h2');");
    assert.equal(
      await evaluate(
        "return document.querySelector('.paper-inspector h2').textContent;",
      ),
      paper.title,
    );
    await evaluate(`document.querySelector(
      '.agent-source-list article.is-expanded .agent-source-actions button:last-child').click();`);
    await waitFor("return document.querySelector('.pdf-page-stage canvas');");
    const reader = await evaluate(`return {
      title: document.querySelector('.reader-title strong').textContent,
      page: Number(document.querySelector('[name="reader-page-number"]').value),
      pages: Number(document.querySelector('[name="reader-page-number"]').max),
      documentId: window.__graphiteCitationReplay.resolvedDocuments.at(-1),
    };`);
    assert.equal(reader.title, paper.title);
    assert.equal(reader.documentId, source.documentId);
    assert.equal(reader.page, source.pageStart ?? 1);
    assert.equal(reader.pages, documents[source.documentId].pages);
    checks.push({
      marker: citation.marker,
      sourceId: citation.sourceId,
      doi: paper.doi,
      excerptMatchesRecordedSource: true,
      keyboardActivation: true,
      graphPaperMatches: true,
      pdfSha256: documents[source.documentId].sha256,
      readerDocumentMatches: true,
      sourcePageStart: source.pageStart,
      unavailablePageDisclosed: source.pageStart === null,
      readerPage: reader.page,
      readerPageCount: reader.pages,
    });
    await delay(350);
    await captureReplay(`reader-${citation.sourceId}`);
  }

  await submitRecordedQuestion(unsupported.question);
  await waitFor(`return document.querySelector('.agent-answer-text')?.textContent
    .includes('Insufficient evidence');`);
  assert.equal(
    await evaluate(
      "return document.querySelector('.agent-answer-text').textContent;",
    ),
    unsupported.answerMarkdown,
  );
  assert.equal(
    await evaluate(
      "return document.querySelectorAll('.agent-inline-citation').length;",
    ),
    0,
  );
  await captureReplay("unsupported-answer");
  const evidence = {
    kind: "recorded-native-response-ui-replay",
    manifestId: manifest.id,
    supportedAnswerMatchesSavedNativeResponse: true,
    supportedAnswerSha256: createHash("sha256")
      .update(supported.answerMarkdown)
      .digest("hex"),
    citations: checks,
    unsupportedAnswerMatchesSavedNativeResponse: true,
    unsupportedAnswer: unsupported.answerMarkdown,
    unsupportedCitationCount: 0,
    limitations: [
      "Browser UI replay of saved native model responses; no new model run.",
      "Not a packaged native citation interaction test.",
      "Recorded passages have no page coordinates. The UI discloses this and opens the correct PDF at page 1; passage-page localization is not verified.",
    ],
  };
  await writeFile(
    resolve(runtime.outputDirectory, "verification.json"),
    `${JSON.stringify(evidence, null, 2)}\n`,
  );
}

async function waitFor(script) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    if (await evaluate(script)) return;
    await delay(100);
  }
  const detail = await evaluate(`return document.body.dataset.previewError ??
    document.querySelector('.fatal-state')?.textContent ?? 'No rendered app.';`);
  throw new Error(
    `The verified metadata preview did not become ready: ${detail}`,
  );
}

try {
  ({ sessionId } = await runtime.createFirefoxSession());
  await runtime.request("POST", `/session/${sessionId}/window/rect`, {
    width: 1440,
    height: 980,
  });
  await runtime.request("POST", `/session/${sessionId}/url`, {
    url: new URL(relative(root, pageFile), `${runtime.appUrl}/`).href,
  });
  await waitFor("return document.querySelector('.view-switcher');");
  if (replayRag) {
    await runCitationReplay();
  } else {
    await evaluate(`
    document.querySelector('[aria-label="Switch to dark mode"]')?.click();
    document.querySelectorAll('.view-switcher button')[1].click();
  `);
    await waitFor("return document.querySelector('.year-chart > button');");
    await evaluate(`
    document.querySelector('.year-chart > button[aria-label="2024: 2 papers"]').click();
  `);
    await waitFor(
      "return document.querySelector('.year-papers-dialog[open]');",
    );
    await delay(400);
    const titles = await evaluate(`return [...document.querySelectorAll(
    '.year-paper-list strong')].map(element => element.textContent);`);
    assert.deepEqual(
      [...titles].sort(),
      workspace.snapshot.items
        .filter((paper) => paper.year === 2024)
        .map((paper) => paper.title)
        .sort(),
    );
    const viewport = await evaluate(
      "return { width: innerWidth, height: innerHeight };",
    );
    const encoded = await runtime.request(
      "GET",
      `/session/${sessionId}/screenshot`,
    );
    const bytes = Buffer.from(encoded, "base64");
    await writeFile(resolve(runtime.outputDirectory, "real-demo.png"), bytes);
    const provenance = {
      schemaVersion: 1,
      kind: "verified-collection-metadata-preview",
      origin: "Graphite React UI rendered in an isolated browser session",
      manifestId: manifest.id,
      manifest: "demo/manifest.json",
      nativeEvidence:
        "imports_real_demo_without_duplicates_and_exports_preview",
      nativeChecks: {
        papers: 60,
        pdfsResolvedAndChecksummed: 60,
        fullTextRecords: 60,
        repeatedSyncPreservesIds: true,
      },
      screenshotChecks: { year: 2024, exactDisplayedPaperCount: titles.length },
      limitations: [
        "Browser rendering of a native-synchronized workspace export.",
        "Not evidence of installed desktop launch or live model answer quality.",
        "Only public bibliographic metadata is displayed; no PDF pages or excerpts.",
      ],
      viewport,
      sha256: createHash("sha256").update(bytes).digest("hex"),
      attribution: manifest.papers
        .filter((paper) => paper.year === 2024)
        .map((paper) => ({
          doi: paper.doi,
          sourceUrl: paper.sourceUrl,
          license: paper.license,
          licenseUrl: paper.licenseUrl,
          attribution: paper.attribution,
        })),
    };
    const serialized = JSON.stringify(provenance, null, 2).replace(
      /[\u0080-\uffff]/g,
      (character) =>
        `\\u${character.charCodeAt(0).toString(16).padStart(4, "0")}`,
    );
    await writeFile(
      resolve(runtime.outputDirectory, "real-demo.provenance.json"),
      `${serialized}\n`,
    );
  }
} finally {
  if (sessionId) {
    await runtime.request("DELETE", `/session/${sessionId}`).catch(() => {});
  }
  await runtime.stop();
}
process.stdout.write(
  replayRag
    ? "Recorded native response UI replay passed: artifacts/qa/real-demo-citations\n"
    : "Verified metadata preview saved: artifacts/qa/real-demo-preview\n",
);
