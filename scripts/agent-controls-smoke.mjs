import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { writeFile } from "node:fs/promises";
import { relative, resolve } from "node:path";
import { fileURLToPath, URL } from "node:url";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";
import {
  prepareQaRuntime,
  assertBrowserQaIsolation,
  firefoxOptions,
} from "./qa-runtime.mjs";

const runtime = await prepareQaRuntime("agent-controls");
let sessionId;
const checks = [];
async function evaluate(script, args = []) {
  return runtime.request("POST", `/session/${sessionId}/execute/sync`, {
    script,
    args,
  });
}
async function waitFor(script) {
  for (let i = 0; i < 80; i++) {
    const value = await evaluate(script);
    if (value) return value;
    await delay(75);
  }
  throw new Error(`UI condition timed out: ${script}`);
}
async function click(selector, index = 0) {
  await waitFor(
    `return document.querySelectorAll(${JSON.stringify(selector)})[${index}];`,
  );
  const elements = await runtime.request(
    "POST",
    `/session/${sessionId}/elements`,
    { using: "css selector", value: selector },
  );
  const id = elements[index]["element-6066-11e4-a52e-4f735466cecf"];
  await runtime.request(
    "POST",
    `/session/${sessionId}/element/${id}/click`,
    {},
  );
}
async function key(value) {
  await runtime.request("POST", `/session/${sessionId}/actions`, {
    actions: [
      {
        type: "key",
        id: "controls-keyboard",
        actions: [
          { type: "keyDown", value },
          { type: "keyUp", value },
        ],
      },
    ],
  });
}
async function type(selector, text) {
  const [element] = await runtime.request(
    "POST",
    `/session/${sessionId}/elements`,
    { using: "css selector", value: selector },
  );
  await runtime.request(
    "POST",
    `/session/${sessionId}/element/${element["element-6066-11e4-a52e-4f735466cecf"]}/value`,
    { text },
  );
}
async function capture(name) {
  await delay(300);
  const png = await runtime.request("GET", `/session/${sessionId}/screenshot`);
  await writeFile(
    resolve(runtime.outputDirectory, `${name}.png`),
    Buffer.from(png, "base64"),
  );
}
async function checkDockMotion(reduced) {
  for (const selector of [".agent-launcher", ".agent-identity"]) {
    const frames = await runtime.request(
      "POST",
      `/session/${sessionId}/execute/async`,
      {
        script: `
          const done = arguments[arguments.length - 1];
          const started = performance.now();
          const frames = [];
          document.querySelector(arguments[0]).click();
          function sample() {
            const dock = document.querySelector('.agent-composer-container');
            if (dock) {
              const matrix = new DOMMatrix(getComputedStyle(dock).transform);
              frames.push(matrix.a);
            }
            if (performance.now() - started < 600) requestAnimationFrame(sample);
            else done(frames);
          }
          requestAnimationFrame(sample);
        `,
        args: [selector],
      },
    );
    const moves = frames.some((scale) => Math.abs(scale - 1) > 0.001);
    assert.equal(moves, !reduced, `Unexpected dock motion for ${selector}`);
    if (!reduced && selector === ".agent-launcher") {
      assert.ok(
        Math.max(...frames) > 1.001,
        "The spring must settle after overshoot.",
      );
    }
    checks.push({ selector, reducedMotion: reduced, spatialAnimation: moves });
  }
  await waitFor(
    "return document.activeElement === document.querySelector('.agent-launcher');",
  );
}

async function openAgent() {
  await click(".agent-launcher");
  await waitFor(
    "return document.activeElement === document.querySelector('.agent-bar input');",
  );
}
async function closeAgent(selector) {
  await click(selector);
  await waitFor(
    "return !document.querySelector('.agent-bar') && !document.querySelector('.agent-workspace') && document.activeElement === document.querySelector('.agent-launcher');",
  );
}
async function checkPreparationNotice() {
  const fixturePath = resolve(runtime.outputDirectory, "preparation.html");
  const fixtureRoute = relative(
    fileURLToPath(new URL("../", import.meta.url)),
    fixturePath,
  );
  assert.ok(
    !fixtureRoute.startsWith(".."),
    "Keep QA fixtures inside the checkout.",
  );
  await writeFile(
    fixturePath,
    `<!doctype html>
<html lang="en" data-theme="dark" data-theme-storage-key="graphite-qa-theme">
<head><meta charset="utf-8"><title>Assistant preparation fixture</title></head>
<body><div id="root"></div><script type="module">
import { services } from "/src/services/runtime.ts";
const load = services.workspace.load.bind(services.workspace);
services.workspace.load = async (...args) => {
  const workspace = await load(...args);
  return {
    ...workspace,
    source: { ...workspace.source, mode: "zotero", health: "ready" },
    semantic: { ...workspace.semantic, modelInstalled: true, indexedChunks: 0 },
  };
};
await import("/src/main.tsx");
</script></body></html>`,
  );
  const url = new URL(fixtureRoute, `${runtime.appUrl}/`).href;
  await runtime.request("POST", `/session/${sessionId}/url`, { url });
  await openAgent();
  await waitFor("return document.querySelector('.agent-preparation-action');");
  assert.match(
    await evaluate(
      "return document.querySelector('.agent-preparation').textContent;",
    ),
    /Local research models run in the desktop app/,
  );
  await waitFor("return document.querySelector('.graph-canvas canvas');");
  await capture("preparation-notice-compact-dark");
  await click(".agent-preparation-close");
  assert.equal(
    await evaluate("return document.querySelector('.agent-preparation');"),
    null,
  );
  assert.equal(
    await evaluate(
      "return document.activeElement === document.querySelector('.agent-bar input');",
    ),
    true,
  );
  await closeAgent(".agent-identity");
  await openAgent();
  assert.equal(
    await evaluate("return document.querySelector('.agent-preparation');"),
    null,
  );

  await runtime.request("POST", `/session/${sessionId}/url`, { url });
  await openAgent();
  await waitFor("return document.querySelector('.agent-preparation-action');");
  await click(".agent-preparation-action");
  await waitFor(
    "return document.querySelector('.source-popover:popover-open');",
  );
  assert.equal(
    await evaluate("return document.querySelector('.agent-preparation');"),
    null,
  );
  assert.equal(
    await evaluate(
      "return document.activeElement === document.querySelector('.source-popover');",
    ),
    true,
  );
  await capture("library-setup-compact-dark");
  await key("\uE00C");
  await waitFor("return !document.querySelector('.source-popover');");

  assert.equal(
    await evaluate(
      "return document.activeElement === document.querySelector('.source-trigger');",
    ),
    true,
  );

  // Setup stays above an already-open assistant workspace.
  await click(".agent-setup-trigger");
  await click(".source-trigger");
  await waitFor(
    "return document.querySelector('.source-popover:popover-open');",
  );
  assert.equal(
    await evaluate(`
    const popup = document.querySelector('.source-popover');
    const workspace = document.querySelector('.agent-workspace');
    const a = popup.getBoundingClientRect();
    const b = workspace.getBoundingClientRect();
    const x = (Math.max(a.left, b.left) + Math.min(a.right, b.right)) / 2;
    const y = (Math.max(a.top, b.top) + Math.min(a.bottom, b.bottom)) / 2;
    return popup.contains(document.elementFromPoint(x, y));
  `),
    true,
  );
  await capture("library-setup-above-assistant-dark");
  await click(".source-popover-close");
  await waitFor("return !document.querySelector('.source-popover');");
  await click(".source-trigger");
  await waitFor(
    "return document.querySelector('.source-popover:popover-open');",
  );
  await click(".brand-lockup");
  await waitFor("return !document.querySelector('.source-popover');");
  checks.push({
    preparationDismissal: true,
    setupDismissesNotice: true,
    setupAboveWorkspace: true,
    setupEscapeCloseAndOutsideDismissal: true,
  });
}

async function checkBounds(label) {
  const result = await evaluate(`
    const elements = ['.agent-bar','.agent-controls','.agent-bar form','.agent-close','.agent-workspace'];
    return elements.flatMap(selector => {
      const node = document.querySelector(selector);
      if (!node) return [];
      const box = node.getBoundingClientRect();
      return [{selector, width:box.width, left:box.left, right:box.right, top:box.top, bottom:box.bottom,
        fits:box.left >= 0 && box.right <= innerWidth + 1 && box.top >= 60 && box.bottom <= innerHeight,
        noOverflow:node.scrollWidth <= node.clientWidth + 1}];
    });`);
  checks.push({ label, bounds: result });
  assert.ok(
    result.every((x) => x.fits && x.noOverflow),
    JSON.stringify({ label, result }),
  );
  assert.ok(result.find((x) => x.selector === ".agent-bar form").width >= 220);
}
try {
  ({ sessionId } = await runtime.createFirefoxSession());
  await runtime.request("POST", `/session/${sessionId}/window/rect`, {
    width: 1440,
    height: 900,
  });
  await runtime.request("POST", `/session/${sessionId}/url`, {
    url: runtime.appUrl,
  });
  await waitFor("return document.querySelector('.graph-canvas canvas');");
  await assertBrowserQaIsolation(runtime.request, sessionId);
  await evaluate(
    "document.querySelector('[aria-label=\"Switch to dark mode\"]')?.click();",
  );
  await click(".graph-scope-trigger");
  await waitFor(
    "return document.querySelector('[aria-label=\"Similarity search scope\"]:popover-open');",
  );
  assert.equal(
    await evaluate(
      "return document.querySelector('.graph-exploration select');",
    ),
    null,
  );
  const chosen = await evaluate(
    "return document.querySelector('.graph-control-popover:popover-open label:nth-of-type(2)').textContent.trim();",
  );
  await click(".graph-control-popover:popover-open input", 1);
  await key("\uE00C");
  assert.equal(
    await evaluate(
      "return document.querySelector('.graph-scope-trigger').textContent.trim();",
    ),
    chosen,
  );
  assert.equal(
    await evaluate(
      "return document.activeElement === document.querySelector('.graph-scope-trigger');",
    ),
    true,
  );
  await click(".graph-scope-trigger");
  await capture("similarity-scope-dark");
  await click(".graph-control-popover:popover-open input");
  await key("\uE00C");
  checks.push({
    similarityScopePopover: true,
    collectionChange: true,
    escapeRestoresFocus: true,
  });
  assert.equal(
    await evaluate(
      "return document.querySelector('.agent-launcher').textContent.trim();",
    ),
    "TYRION",
  );
  const fontSize = await evaluate(
    "return parseFloat(getComputedStyle(document.querySelector('.agent-launcher')).fontSize);",
  );
  assert.ok(Math.abs(fontSize - (12 - (2 * 96) / 72)) < 0.01);
  assert.equal(
    await evaluate("return document.querySelector('.semantic-zoom');"),
    null,
  );
  await capture("agent-launcher-dark");
  await checkDockMotion(false);
  await openAgent();
  assert.equal(
    await evaluate(
      "return document.querySelector('.agent-context-disclosure');",
    ),
    null,
  );
  assert.equal(
    await evaluate(
      "return document.querySelector('.agent-controls').textContent.trim();",
    ),
    "SetupSearch Context",
  );
  await type(".agent-bar input", "Preserve this draft");
  await capture("agent-bar-dark");
  await click(".agent-context-trigger");
  await waitFor(
    "return document.querySelector('.agent-context-popover:popover-open');",
  );
  await click(".agent-scope-choice input", 1);
  assert.equal(
    await evaluate(
      "return document.querySelector('.agent-scope-hint').textContent;",
    ),
    "Select papers to ask a question.",
  );
  await capture("search-context-dark");
  await key("\uE00C");
  assert.equal(
    await evaluate(
      "return document.activeElement === document.querySelector('.agent-context-trigger');",
    ),
    true,
  );
  await closeAgent(".agent-identity");
  await openAgent();
  assert.equal(
    await evaluate("return document.querySelector('.agent-bar input').value;"),
    "Preserve this draft",
  );
  await click(".agent-context-trigger");
  assert.equal(
    await evaluate(
      "return document.querySelector('.agent-scope-choice input[value=\"selected-only\"]').checked;",
    ),
    true,
  );
  await click(".agent-scope-choice input");
  await key("\uE00C");
  await click(".agent-setup-trigger");
  await waitFor("return document.querySelector('.agent-setup-panel');");
  await capture("agent-setup-dark");
  await checkBounds("desktop-setup");
  await closeAgent(".agent-identity");
  await openAgent();
  await closeAgent(".agent-close");
  await openAgent();
  await click(".agent-setup-trigger");
  await closeAgent(".agent-workspace-header > button");
  checks.push({
    iconAndBothCloseButtonsCollapse: true,
    draftAndScopePreserved: true,
    setupOpens: true,
  });
  await click(".search-trigger");
  await type('[aria-label="Research library query"]', "surface");
  await waitFor("return document.querySelector('.search-result-open');");
  await click(".search-result-open");
  await waitFor(
    "return document.querySelector('.paper-inspector') && !document.querySelector('.search-backdrop');",
  );
  await openAgent();
  await runtime.request("POST", `/session/${sessionId}/window/rect`, {
    width: 1024,
    height: 768,
  });
  await capture("agent-bar-compact-dark");
  await checkBounds("compact-inspector");
  await click(".agent-context-trigger");
  await capture("search-context-compact-dark");
  await key("\uE00C");
  await click(".agent-setup-trigger");
  await capture("agent-setup-compact-dark");
  await checkBounds("compact-inspector-setup");
  await closeAgent(".agent-close");
  await evaluate(
    "document.querySelector('[aria-label=\"Switch to light mode\"]')?.click();",
  );
  await openAgent();
  await capture("agent-bar-compact-light");
  await checkBounds("compact-light");
  await checkPreparationNotice();
  await runtime.request("DELETE", `/session/${sessionId}`);
  sessionId = null;
  ({ sessionId } = await runtime.request("POST", "/session", {
    capabilities: {
      alwaysMatch: {
        browserName: "firefox",
        "moz:firefoxOptions": {
          ...firefoxOptions(),
          prefs: { "ui.prefersReducedMotion": 1 },
        },
      },
    },
  }));
  await runtime.request("POST", `/session/${sessionId}/url`, {
    url: runtime.appUrl,
  });
  await waitFor("return document.querySelector('.agent-launcher');");
  assert.equal(
    await evaluate(
      "return matchMedia('(prefers-reduced-motion: reduce)').matches;",
    ),
    true,
  );
  await checkDockMotion(true);
  await openAgent();
  assert.equal(
    await evaluate(
      "return getComputedStyle(document.querySelector('.agent-composer-container')).transform;",
    ),
    "none",
  );
  await closeAgent(".agent-identity");
  checks.push({ reducedMotionOpenCloseAndFocus: true });
  await writeFile(
    resolve(runtime.outputDirectory, "verification.json"),
    JSON.stringify(checks, null, 2) + "\n",
  );
  process.stdout.write("Agent and similarity controls smoke passed.\n");
} catch (error) {
  await capture("failure");
  throw error;
} finally {
  if (sessionId)
    await runtime.request("DELETE", `/session/${sessionId}`).catch(() => {});
  await runtime.stop();
}
