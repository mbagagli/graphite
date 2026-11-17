import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { relative, resolve } from "node:path";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";
import { URL, fileURLToPath } from "node:url";
import { prepareQaRuntime } from "./qa-runtime.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const manifest = JSON.parse(
  await readFile(resolve(root, "demo/manifest.json"), "utf8"),
);
const record = manifest.papers.find((paper) => paper.id === "se-12-1683-2021");
const source = await readFile(
  resolve(process.argv[2] ?? "artifacts/real-demo/pdfs", `${record.id}.pdf`),
);
assert.equal(createHash("sha256").update(source).digest("hex"), record.sha256);
const runtime = await prepareQaRuntime("pdf-editor");
let sessionId;
const request = (method, path, body) =>
  runtime.request(method, `/session/${sessionId}${path}`, body);
const evaluate = (script, args = []) =>
  request("POST", "/execute/sync", { script, args });
async function waitFor(script) {
  for (let attempt = 0; attempt < 120; attempt++) {
    const value = await evaluate(script);
    if (value) return value;
    await delay(100);
  }
  throw new Error(`PDF interaction timed out: ${script}`);
}
async function click(selector, index = 0) {
  await waitFor(
    `return document.querySelectorAll(${JSON.stringify(selector)})[${index}];`,
  );
  const elements = await request("POST", "/elements", {
    using: "css selector",
    value: selector,
  });
  await request(
    "POST",
    `/element/${elements[index]["element-6066-11e4-a52e-4f735466cecf"]}/click`,
    {},
  );
}
async function type(selector, value) {
  const elements = await request("POST", "/elements", {
    using: "css selector",
    value: selector,
  });
  const id = elements[0]["element-6066-11e4-a52e-4f735466cecf"];
  await request("POST", `/element/${id}/click`, {});
  await request("POST", "/actions", {
    actions: [
      {
        type: "key",
        id: "pdf-keyboard",
        actions: [
          { type: "keyDown", value: "\uE009" },
          { type: "keyDown", value: "a" },
          { type: "keyUp", value: "a" },
          { type: "keyUp", value: "\uE009" },
        ],
      },
    ],
  });
  await request("POST", "/actions", {
    actions: [
      {
        type: "key",
        id: "pdf-keyboard",
        actions: [...value].flatMap((character) => [
          { type: "keyDown", value: character },
          { type: "keyUp", value: character },
        ]),
      },
    ],
  });
}
async function key(value) {
  await request("POST", "/actions", {
    actions: [
      {
        type: "key",
        id: "pdf-keyboard",
        actions: [
          { type: "keyDown", value },
          { type: "keyUp", value },
        ],
      },
    ],
  });
}
async function capture(name) {
  const png = await request("GET", "/screenshot");
  await writeFile(
    resolve(runtime.outputDirectory, `${name}.png`),
    Buffer.from(png, "base64"),
  );
}
async function editorReady() {
  await waitFor(
    "return document.querySelector('.pdf-editor-page[aria-busy=false]') && document.querySelector('.pdf-editor-footer [role=status]').textContent !== 'Preparing PDF...';",
  );
}

try {
  ({ sessionId } = await runtime.createFirefoxSession());
  await request("POST", "/window/rect", { width: 1440, height: 900 });
  await writeFile(resolve(runtime.outputDirectory, "paper.pdf"), source);
  await writeFile(
    resolve(runtime.outputDirectory, "paper.json"),
    JSON.stringify(record),
  );
  const fixture = resolve(runtime.outputDirectory, "reader.html");
  await writeFile(
    fixture,
    `<!doctype html><html lang="en" data-theme="dark" data-theme-storage-key="graphite-pdf-qa-theme" data-theme-dark-color="#292b2e" data-theme-light-color="#f4f3ef"><head><meta charset="utf-8"><title>Verified PDF editing check</title></head><body><div id="root"></div><script type="module">
import { createElement, useState } from "react";
import { createRoot } from "react-dom/client";
import "/src/design-system/tokens.css";
import "/src/design-system/global.css";
import "/src/features/reader/reader.css";
import "@fontsource-variable/inter/wght.css";
import { ReaderView } from "/src/features/reader/ReaderView.tsx";
import { ThemeProvider } from "/src/design-system/ThemeProvider.tsx";
import { services } from "/src/services/runtime.ts";
window.qaErrors = [];
window.addEventListener("error", event => window.qaErrors.push(event.message));
window.addEventListener("unhandledrejection", event => window.qaErrors.push(String(event.reason)));
window.pdfResolutions = 0;
const record = await (await fetch("./paper.json")).json();
const paper = { id: record.id, title: record.title, year: record.year,
 authors: record.authors.map((name, i) => ({id: String(i), name, creatorType: "author"})),
 abstractText: null, topics: [], tags: [], collectionIds: [], notes: [],
 publicationTitle: record.journal, doi: record.doi, url: record.sourceUrl,
 attachments: [{id: record.id, documentId: record.id, title: record.title,
 mediaType: "application/pdf", pageCount: record.pdfPages, availability: "available"}] };
services.document.resolve = async () => { window.pdfResolutions++; return {
 documentId: record.id, status: "ready", mediaType: "application/pdf",
 contentLength: record.pdfBytes, revisionKey: record.sha256, message: null,
 url: new URL("./paper.pdf", location.href).href } };
services.document.getText = async () => null;
services.document.getReaderState = async () => null;
services.document.saveReaderState = async () => {};
function Harness() {
 const [context, setContext] = useState(null);
 return createElement("div", {style:{height:"100vh"}},
  createElement(ReaderView, {paper, topics: [], onContextChange:setContext,
   onBack:()=>{}, onAsk:()=>{}}),
  createElement("output", {id:"reader-context-check", hidden:true}, JSON.stringify(context)));
}
createRoot(document.getElementById("root")).render(createElement(ThemeProvider, null, createElement(Harness)));
</script></body></html>`,
  );
  const route = relative(root, fixture);
  assert.ok(!route.startsWith(".."));
  const url = new URL(route, `${runtime.appUrl}/`).href;
  await request("POST", "/url", { url });
  await waitFor(
    "return document.querySelector('.pdf-page-stage canvas[data-page-number=\"1\"][aria-busy=false]');",
  );
  await evaluate(
    "window.originalCanvas = document.querySelector('.pdf-page-stage canvas'); window.originalResolutions = window.pdfResolutions;",
  );
  await click('[aria-label="Next page"]');
  await waitFor(
    "return document.querySelector('.pdf-page-stage canvas[data-page-number=\"2\"][aria-busy=false]');",
  );
  assert.equal(
    await evaluate(
      "return originalCanvas === document.querySelector('.pdf-page-stage canvas') && pdfResolutions === originalResolutions;",
    ),
    true,
  );
  await type('.reader-page-tools input[name="reader-page-number"]', "12");
  assert.equal(
    await evaluate(
      "return document.querySelector('.pdf-page-stage canvas').dataset.pageNumber;",
    ),
    "2",
  );
  await key("\uE007");
  await waitFor(
    "return document.querySelector('.pdf-page-stage canvas[data-page-number=\"12\"][aria-busy=false]');",
  );
  await evaluate(`const area = document.querySelector('.reader-page-area'); area.scrollTop = area.scrollHeight;
    for (let i=0;i<8;i++) area.dispatchEvent(new WheelEvent('wheel', {deltaY:100,bubbles:true}));`);
  await waitFor(
    "return document.querySelector('.pdf-page-stage canvas[data-page-number=\"13\"][aria-busy=false]');",
  );
  assert.equal(
    await evaluate("return pdfResolutions === originalResolutions;"),
    true,
  );
  await capture("reader-stable-page-change");
  process.stdout.write(
    "Stable rendering, page entry, and one-turn wheel gesture verified.\n",
  );

  await evaluate(
    "[...document.querySelectorAll('.reader-actions button')].find(b => b.textContent.includes('Edit PDF')).click();",
  );
  await editorReady();
  await click(".pdf-editor-toolbar button", 1);
  const selection =
    await waitFor(`const spans = [...document.querySelectorAll('.pdf-editor-text-layer .textLayer span')];
    const span = spans.find(element => element.textContent.length > 20);
    if (!span) return null;
    const rect = span.getBoundingClientRect();
    if (rect.width < 10 || rect.height < 5) return null;
    return {x:Math.round(rect.left+2), y:Math.round(rect.top+rect.height/2), end:Math.round(rect.right-2)};`);
  await request("POST", "/actions", {
    actions: [
      {
        type: "pointer",
        id: "pdf-pointer",
        parameters: { pointerType: "mouse" },
        actions: [
          { type: "pointerMove", duration: 0, x: selection.x, y: selection.y },
          { type: "pointerDown", button: 0 },
          {
            type: "pointerMove",
            duration: 250,
            x: selection.end,
            y: selection.y,
          },
          { type: "pointerUp", button: 0 },
        ],
      },
    ],
  });
  await waitFor(
    "return document.querySelector('.pdf-annotation-list').textContent.includes('Highlight');",
  );
  await editorReady();
  await click(".pdf-editor-toolbar button", 3);
  const point = await evaluate(
    "const rect=document.querySelector('.pdf-editor-drawing-layer').getBoundingClientRect(); return {x:Math.round(rect.left+80),y:Math.round(rect.top+150)};",
  );
  await request("POST", "/actions", {
    actions: [
      {
        type: "pointer",
        id: "pdf-pointer",
        parameters: { pointerType: "mouse" },
        actions: [
          { type: "pointerMove", duration: 0, x: point.x, y: point.y },
          { type: "pointerDown", button: 0 },
          {
            type: "pointerMove",
            duration: 150,
            x: point.x + 40,
            y: point.y + 10,
          },
          { type: "pointerMove", duration: 150, x: point.x + 80, y: point.y },
          { type: "pointerUp", button: 0 },
        ],
      },
    ],
  });
  await waitFor(
    "return document.querySelector('.pdf-annotation-list').textContent.includes('Drawing');",
  );
  await editorReady();
  await click(".pdf-editor-toolbar button", 4);
  await waitFor("return document.querySelector('.pdf-text-target');");
  await click(".pdf-text-target");
  await waitFor(
    "return document.querySelector('.pdf-edit-detail[open] textarea');",
  );
  await type(".pdf-edit-detail textarea", "Graphite editing check");
  await click(".pdf-edit-detail input[type=radio]", 1);
  await click(".pdf-edit-detail .pdf-edit-dialog-actions button", 1);
  await waitFor("return !document.querySelector('.pdf-edit-detail');");
  await editorReady();
  await waitFor(
    "return document.querySelector('[aria-label=\"Edit text: Graphite editing check\"]');",
  );
  await capture("edited-text");
  await click(".pdf-editor-toolbar button", 5);
  await editorReady();
  await waitFor(
    "return !document.querySelector('[aria-label=\"Edit text: Graphite editing check\"]');",
  );
  await click(".pdf-editor-toolbar button", 6);
  await editorReady();
  await waitFor(
    "return document.querySelector('[aria-label=\"Edit text: Graphite editing check\"]');",
  );

  await click(".pdf-editor-toolbar button", 7);
  const originalHeight = await evaluate(
    "return document.querySelector('.pdf-editor-page canvas').height;",
  );
  await click(".pdf-editor-page-actions button", 1);
  await editorReady();
  assert.notEqual(
    await evaluate(
      "return document.querySelector('.pdf-editor-page canvas').height;",
    ),
    originalHeight,
  );
  await click(".pdf-editor-toolbar button", 5);
  await editorReady();
  await click(".pdf-editor-page-actions button", 3);
  await editorReady();
  assert.equal(
    await evaluate(
      "return document.querySelector('.pdf-editor-navigation input').max;",
    ),
    String(record.pdfPages + 1),
  );
  await click(".pdf-delete-button");
  await waitFor("return document.querySelector('#pdf-delete-title');");
  await click(
    "dialog[aria-labelledby=pdf-delete-title] .pdf-edit-dialog-actions button",
    1,
  );
  await waitFor("return !document.querySelector('#pdf-delete-title');");
  await editorReady();
  assert.equal(
    await evaluate(
      "return document.querySelector('.pdf-editor-navigation input').max;",
    ),
    String(record.pdfPages),
  );
  await click(".pdf-editor-toolbar button", 7);
  await click(".pdf-editor-toolbar button", 2);
  await click(".pdf-editor-drawing-layer");
  await waitFor("return document.querySelector('.pdf-edit-detail textarea');");
  await type(".pdf-edit-detail textarea", "A local review note");
  await click(".pdf-edit-detail .pdf-edit-dialog-actions button", 1);
  await waitFor("return !document.querySelector('.pdf-edit-detail');");
  await editorReady();
  await waitFor(
    "return document.querySelector('.pdf-annotation-list').textContent.includes('A local review note');",
  );
  await click(".pdf-save-button");
  await waitFor(
    "return document.querySelector('.pdf-editor-footer [role=status]').textContent === 'Saved in Graphite';",
  );
  await capture("pdf-editor-saved");
  await click('[aria-label="Close Edit PDF"]');
  await waitFor(
    "return !document.querySelector('.pdf-editor-dialog') && document.querySelector('.reader-copy-control')?.textContent.includes('Edited copy');",
  );
  await waitFor(
    "return document.querySelector('.pdf-page-stage canvas[aria-busy=false]');",
  );
  assert.equal(
    await evaluate(
      "return JSON.parse(document.querySelector('#reader-context-check').textContent).pageNumber;",
    ),
    null,
  );
  await click(".reader-copy-control button");
  await waitFor(
    "return document.querySelector('.reader-copy-control')?.textContent.includes('View edited copy');",
  );
  await click(".reader-copy-control button");
  await waitFor(
    "return document.querySelector('.reader-copy-control')?.textContent.includes('View original');",
  );
  await request("POST", "/url", { url });
  await waitFor(
    "return document.querySelector('.reader-copy-control')?.textContent.includes('Edited copy');",
  );
  await waitFor(
    "return document.querySelector('.reader-edit-button:not(:disabled)');",
  );
  await request("POST", "/window/rect", { width: 1024, height: 700 });
  await evaluate(
    "[...document.querySelectorAll('.reader-actions button')].find(b => b.textContent.includes('Edit PDF')).click();",
  );
  await editorReady();
  await capture("pdf-editor-compact");
  assert.equal(
    await evaluate(`const box=document.querySelector('.pdf-editor-dialog').getBoundingClientRect();
    return box.left>=0 && box.right<=innerWidth && box.top>=0 && box.bottom<=innerHeight;`),
    true,
  );
  assert.deepEqual(await evaluate("return qaErrors;"), []);
  await writeFile(
    resolve(runtime.outputDirectory, "verification.json"),
    JSON.stringify(
      {
        paper: record.id,
        sourceSha256: record.sha256,
        stablePageCanvas: true,
        noDocumentReloadOnPageChange: true,
        originalTextEdit: true,
        undoRedo: true,
        pageRotationInsertionDeletion: true,
        highlightsAndDrawing: true,
        notePersistence: true,
        savedCopyReopens: true,
        sourceContextUsesOriginal: true,
        compactViewport: true,
      },
      null,
      2,
    ),
  );
  process.stdout.write(
    `PDF editing smoke passed; evidence: ${relative(root, runtime.outputDirectory)}\n`,
  );
} catch (error) {
  if (sessionId) {
    await capture("failure");
    process.stderr.write(
      JSON.stringify(
        await evaluate(
          "return {text:document.body.innerText.slice(-3500),errors:window.qaErrors};",
        ),
      ) + "\n",
    );
  }
  throw error;
} finally {
  if (sessionId) await request("DELETE", "");
  await runtime.stop();
}
