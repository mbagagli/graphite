import { Buffer } from "node:buffer";
import { writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import process from "node:process";
import { setTimeout as sleep } from "node:timers/promises";

import {
  assertBrowserQaIsolation,
  parsePerformanceArguments,
  prepareQaRuntime,
} from "./qa-runtime.mjs";

const { largeGraphMode, profileLabel } = parsePerformanceArguments(
  process.argv.slice(2),
);
let appUrl;
let driverRequest;
let outputDirectory;

function delay(milliseconds) {
  return sleep(milliseconds);
}

function elementId(element) {
  return element["element-6066-11e4-a52e-4f735466cecf"];
}

async function findElements(sessionId, selector) {
  return driverRequest("POST", `/session/${sessionId}/elements`, {
    using: "css selector",
    value: selector,
  });
}

async function waitForElement(sessionId, selector, index = 0) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    const elements = await findElements(sessionId, selector);
    if (elements[index]) return elementId(elements[index]);
    await delay(100);
  }
  throw new Error(`Timed out waiting for ${selector}.`);
}

async function waitForAbsence(sessionId, selector) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    const elements = await findElements(sessionId, selector);
    if (elements.length === 0) return;
    await delay(100);
  }
  throw new Error(`Timed out waiting for ${selector} to close.`);
}

async function click(sessionId, selector, index = 0) {
  const id = await waitForElement(sessionId, selector, index);
  await driverRequest("POST", `/session/${sessionId}/element/${id}/click`, {});
}

async function executeScript(sessionId, script, args = []) {
  return driverRequest("POST", `/session/${sessionId}/execute/sync`, {
    script,
    args,
  });
}

async function graphMutationCount(sessionId) {
  return executeScript(
    sessionId,
    "return window.__graphitePerformance?.graphCanvasMutations ?? 0;",
  );
}

async function measureClick(sessionId, selector, index = 0, settleMs = 180) {
  const beforeMutations = await graphMutationCount(sessionId);
  const before = await executeScript(sessionId, "return performance.now();");
  await click(sessionId, selector, index);
  await delay(settleMs);
  const after = await executeScript(sessionId, "return performance.now();");
  const afterMutations = await graphMutationCount(sessionId);
  return {
    elapsedMs: Number((after - before).toFixed(1)),
    graphCanvasMutations: afterMutations - beforeMutations,
  };
}

async function measureClickToElement(
  sessionId,
  clickSelector,
  targetSelector,
  index = 0,
) {
  const beforeMutations = await graphMutationCount(sessionId);
  const start = await executeScript(sessionId, "return performance.now();");
  await click(sessionId, clickSelector, index);
  await waitForElement(sessionId, targetSelector);
  await delay(32);
  const end = await executeScript(sessionId, "return performance.now();");
  const afterMutations = await graphMutationCount(sessionId);
  return {
    elapsedMs: Number((end - start).toFixed(1)),
    graphCanvasMutations: afterMutations - beforeMutations,
  };
}

async function dragGraph(sessionId) {
  const start = await executeScript(sessionId, "return performance.now();");
  await driverRequest("POST", `/session/${sessionId}/actions`, {
    actions: [
      {
        type: "pointer",
        id: "mouse",
        parameters: { pointerType: "mouse" },
        actions: [
          {
            type: "pointerMove",
            duration: 0,
            origin: "viewport",
            x: 1_140,
            y: 620,
          },
          { type: "pointerDown", button: 0 },
          {
            type: "pointerMove",
            duration: 120,
            origin: "viewport",
            x: 1_230,
            y: 670,
          },
          { type: "pointerUp", button: 0 },
        ],
      },
    ],
  });
  await driverRequest("DELETE", `/session/${sessionId}/actions`);
  await delay(180);
  const end = await executeScript(sessionId, "return performance.now();");
  return Number((end - start).toFixed(1));
}

async function runLargeGraphProfile(sessionId) {
  await driverRequest("POST", `/session/${sessionId}/window/rect`, {
    width: 1440,
    height: 900,
  });
  await driverRequest("POST", `/session/${sessionId}/url`, {
    url: `${appUrl}/?graphScale=large`,
  });
  await waitForElement(
    sessionId,
    ".workspace-view-layer--graph.is-active .graph-canvas canvas",
  );
  await delay(1_200);
  await assertBrowserQaIsolation(driverRequest, sessionId);

  await executeScript(
    sessionId,
    `
      window.__graphitePerformance = { graphCanvasMutations: 0 };
      const graphContainer = document.querySelector(".graph-canvas");
      new MutationObserver((records) => {
        for (const record of records) {
          const changedCanvases = [...record.addedNodes, ...record.removedNodes]
            .filter((node) => node instanceof HTMLCanvasElement).length;
          window.__graphitePerformance.graphCanvasMutations += changedCanvases;
        }
      }).observe(graphContainer, { childList: true });
    `,
  );

  const metrics = {
    projectionLabel: await executeScript(
      sessionId,
      'return document.querySelector(".graph-title-block small")?.textContent?.replace(/\\s+/g, " ").trim();',
    ),
    canvasCount: await executeScript(
      sessionId,
      'return document.querySelectorAll(".graph-canvas canvas").length;',
    ),
    semanticScaleBefore: await executeScript(
      sessionId,
      'return document.querySelector(".semantic-zoom strong")?.textContent;',
    ),
  };
  metrics.layerToggle = await measureClick(
    sessionId,
    ".relation-control > button",
    0,
  );
  for (let index = 0; index < 4; index += 1) {
    await click(sessionId, ".graph-tools .icon-button", 0);
    await delay(200);
  }
  metrics.semanticScaleAfterZoom = await executeScript(
    sessionId,
    'return document.querySelector(".semantic-zoom strong")?.textContent;',
  );
  metrics.panElapsedMs = await dragGraph(sessionId);
  metrics.totalGraphCanvasMutations = await graphMutationCount(sessionId);
  await click(sessionId, ".graph-tools .icon-button", 2);
  await delay(380);
  metrics.semanticScaleAfterReset = await executeScript(
    sessionId,
    'return document.querySelector(".semantic-zoom strong")?.textContent;',
  );

  if (
    metrics.projectionLabel !== "Showing 1,200 of 10,000 research entities" ||
    metrics.canvasCount < 5 ||
    metrics.layerToggle.graphCanvasMutations !== 0 ||
    metrics.totalGraphCanvasMutations !== 0
  ) {
    throw new Error(`Large graph profile failed: ${JSON.stringify(metrics)}`);
  }
  return metrics;
}

async function runProfile(sessionId) {
  await driverRequest("POST", `/session/${sessionId}/window/rect`, {
    width: 1440,
    height: 900,
  });
  await driverRequest("POST", `/session/${sessionId}/url`, {
    url: appUrl,
  });
  await waitForElement(
    sessionId,
    ".workspace-view-layer--graph.is-active .graph-canvas canvas",
  );
  await delay(700);
  await assertBrowserQaIsolation(driverRequest, sessionId);

  await executeScript(
    sessionId,
    `
      window.__graphitePerformance = { graphCanvasMutations: 0 };
      const graphContainer = document.querySelector(".graph-canvas");
      new MutationObserver((records) => {
        for (const record of records) {
          const changedCanvases = [...record.addedNodes, ...record.removedNodes]
            .filter((node) => node instanceof HTMLCanvasElement).length;
          window.__graphitePerformance.graphCanvasMutations += changedCanvases;
        }
      }).observe(graphContainer, { childList: true });
    `,
  );

  const metrics = {};
  metrics.layerToggle = await measureClick(
    sessionId,
    ".relation-control > button",
    0,
  );

  await click(sessionId, ".search-trigger");
  await click(sessionId, ".search-citation-filters button", 2);
  await waitForElement(sessionId, ".search-result-open");
  metrics.openInspector = await measureClickToElement(
    sessionId,
    ".search-result-open",
    ".paper-inspector",
  );
  await waitForAbsence(sessionId, ".search-backdrop");
  metrics.closeInspector = await measureClick(
    sessionId,
    ".inspector-header button",
  );
  await waitForAbsence(sessionId, ".paper-inspector");

  for (let index = 0; index < 6; index += 1) {
    await click(sessionId, ".graph-tools .icon-button", 0);
    await delay(200);
  }
  await delay(250);
  metrics.semanticScaleBeforeViewSwitch = await executeScript(
    sessionId,
    'return document.querySelector(".semantic-zoom strong")?.textContent;',
  );

  metrics.openAnalytics = await measureClickToElement(
    sessionId,
    ".view-switcher button",
    ".workspace-view-layer--analytics.is-active .analytics-view",
    1,
  );
  metrics.returnToGraph = await measureClickToElement(
    sessionId,
    ".view-switcher button",
    ".workspace-view-layer--graph.is-active .graph-canvas canvas",
    0,
  );
  await delay(450);
  metrics.semanticScaleAfterViewSwitch = await executeScript(
    sessionId,
    'return document.querySelector(".semantic-zoom strong")?.textContent;',
  );

  metrics.openAgent = await measureClickToElement(
    sessionId,
    ".agent-identity",
    ".agent-workspace",
  );
  metrics.closeAgent = await measureClick(
    sessionId,
    ".agent-workspace-header button",
  );
  metrics.openSettings = await measureClickToElement(
    sessionId,
    ".settings-control .icon-button",
    ".settings-popover",
  );

  metrics.totalGraphCanvasMutations = await graphMutationCount(sessionId);
  return metrics;
}

const runtime = await prepareQaRuntime("performance");
appUrl = runtime.appUrl;
driverRequest = runtime.request;
outputDirectory = runtime.outputDirectory;

let sessionId;
try {
  const session = await runtime.createFirefoxSession();
  sessionId = session.sessionId;
  const metrics = largeGraphMode
    ? await runLargeGraphProfile(sessionId)
    : await runProfile(sessionId);
  const outputPath = resolve(outputDirectory, `${profileLabel}.json`);
  await writeFile(outputPath, `${JSON.stringify(metrics, null, 2)}\n`);
  if (largeGraphMode) {
    const encoded = await driverRequest(
      "GET",
      `/session/${sessionId}/screenshot`,
    );
    await writeFile(
      resolve(outputDirectory, `${profileLabel}.png`),
      Buffer.from(encoded, "base64"),
    );
  }
  process.stdout.write(`${outputPath}\n`);
} finally {
  if (sessionId) {
    await driverRequest("DELETE", `/session/${sessionId}`).catch(() => {});
  }
  await runtime.stop();
}
