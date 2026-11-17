import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import process from "node:process";
import { setTimeout as sleep } from "node:timers/promises";

import { assertBrowserQaIsolation, prepareQaRuntime } from "./qa-runtime.mjs";

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
  for (let attempt = 0; attempt < 40; attempt += 1) {
    const elements = await findElements(sessionId, selector);
    if (elements[index]) return elementId(elements[index]);
    await delay(150);
  }
  throw new Error(`Timed out waiting for ${selector}.`);
}

async function waitForAbsence(sessionId, selector) {
  for (let attempt = 0; attempt < 40; attempt += 1) {
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

async function typeText(sessionId, selector, text) {
  const id = await waitForElement(sessionId, selector);
  await driverRequest("POST", `/session/${sessionId}/element/${id}/value`, {
    text,
    value: [...text],
  });
}

async function clearText(sessionId, selector) {
  await click(sessionId, selector);
  await driverRequest("POST", `/session/${sessionId}/actions`, {
    actions: [
      {
        type: "key",
        id: "clear-field",
        actions: [
          { type: "keyDown", value: "\uE009" },
          { type: "keyDown", value: "a" },
          { type: "keyUp", value: "a" },
          { type: "keyUp", value: "\uE009" },
          { type: "keyDown", value: "\uE003" },
          { type: "keyUp", value: "\uE003" },
        ],
      },
    ],
  });
}

async function scrollIntoView(sessionId, selector, index = 0) {
  const id = await waitForElement(sessionId, selector, index);
  await driverRequest("POST", `/session/${sessionId}/execute/sync`, {
    script:
      "arguments[0].scrollIntoView({block: 'center', inline: 'nearest'});",
    args: [{ "element-6066-11e4-a52e-4f735466cecf": id }],
  });
}

async function executeScript(sessionId, script, args = []) {
  return driverRequest("POST", `/session/${sessionId}/execute/sync`, {
    script,
    args,
  });
}

async function capture(sessionId, filename) {
  const encoded = await driverRequest(
    "GET",
    `/session/${sessionId}/screenshot`,
  );
  await writeFile(
    resolve(outputDirectory, filename),
    Buffer.from(encoded, "base64"),
  );
}

async function movePointer(sessionId, x, y) {
  await driverRequest("POST", `/session/${sessionId}/actions`, {
    actions: [
      {
        type: "pointer",
        id: "mouse",
        parameters: { pointerType: "mouse" },
        actions: [
          {
            type: "pointerMove",
            duration: 80,
            origin: "viewport",
            x,
            y,
          },
        ],
      },
    ],
  });
  await driverRequest("DELETE", `/session/${sessionId}/actions`);
}

async function dragHorizontally(sessionId, selector, distance) {
  const id = await waitForElement(sessionId, selector);
  const rect = await driverRequest(
    "GET",
    `/session/${sessionId}/element/${id}/rect`,
  );
  const startX = Math.round(rect.x + rect.width / 2);
  const startY = Math.round(rect.y + rect.height / 2);
  await driverRequest("POST", `/session/${sessionId}/actions`, {
    actions: [
      {
        type: "pointer",
        id: "resize-mouse",
        parameters: { pointerType: "mouse" },
        actions: [
          {
            type: "pointerMove",
            duration: 0,
            origin: "viewport",
            x: startX,
            y: startY,
          },
          { type: "pointerDown", button: 0 },
          {
            type: "pointerMove",
            duration: 220,
            origin: "viewport",
            x: startX + distance,
            y: startY,
          },
          { type: "pointerUp", button: 0 },
        ],
      },
    ],
  });
  await driverRequest("DELETE", `/session/${sessionId}/actions`);
}

async function inspectGraphContrast(sessionId, expectedTheme) {
  const state = await executeScript(
    sessionId,
    `
      const styles = getComputedStyle(document.documentElement);
      const probe = document.createElement("span");
      probe.hidden = true;
      document.body.appendChild(probe);

      const color = (token) => {
        probe.style.color = styles.getPropertyValue(token).trim();
        return getComputedStyle(probe).color;
      };
      const channels = (value) =>
        value.match(/[\\d.]+/g).slice(0, 3).map(Number);
      const luminance = (value) => {
        const [red, green, blue] = channels(value).map((channel) => {
          const normalized = channel / 255;
          return normalized <= 0.04045
            ? normalized / 12.92
            : Math.pow((normalized + 0.055) / 1.055, 2.4);
        });
        return red * 0.2126 + green * 0.7152 + blue * 0.0722;
      };
      const ratio = (first, second) => {
        const high = Math.max(luminance(first), luminance(second));
        const low = Math.min(luminance(first), luminance(second));
        return Number(((high + 0.05) / (low + 0.05)).toFixed(2));
      };

      const background = color("--color-background-deep");
      const values = {
        normalLabel: color("--color-text-primary"),
        hoverLabelBackground: color("--color-surface-raised"),
        hoverLabelText: color("--color-text-primary"),
        hoverNode: color("--graph-node-hover"),
        selectedNode: color("--graph-node-selected"),
        externalNode: color("--graph-node-external"),
      };
      probe.remove();
      return {
        theme: document.documentElement.dataset.theme,
        values,
        ratios: {
          normalLabel: ratio(values.normalLabel, background),
          hoverLabel: ratio(
            values.hoverLabelText,
            values.hoverLabelBackground,
          ),
          hoverNode: ratio(values.hoverNode, background),
          selectedNode: ratio(values.selectedNode, background),
          externalNode: ratio(values.externalNode, background),
        },
      };
    `,
  );

  const invalid =
    state.theme !== expectedTheme ||
    state.ratios.normalLabel < 4.5 ||
    state.ratios.hoverLabel < 4.5 ||
    state.ratios.hoverNode < 3 ||
    state.ratios.selectedNode < 3 ||
    state.ratios.externalNode < 3;
  if (invalid) {
    throw new Error(
      `Graph contrast failed in ${expectedTheme}: ${JSON.stringify(state)}.`,
    );
  }
  return state;
}

async function inspectTheme(sessionId, expectedTheme) {
  const state = await executeScript(
    sessionId,
    `
      const root = document.documentElement;
      const resources = performance.getEntriesByType("resource").map(
        (entry) => entry.name,
      );
      return {
        theme: root.dataset.theme,
        storedTheme: localStorage.getItem(root.dataset.themeStorageKey),
        themeColor: document.querySelector('meta[name="theme-color"]')?.content,
        logoSource: document.querySelector(".brand-mark")?.src ?? null,
        fontFamily: getComputedStyle(document.body).fontFamily,
        interLoaded: document.fonts.check('12px "Inter Variable"'),
        externalResources: resources.filter((resource) => {
          try {
            const url = new URL(resource);
            return url.origin !== window.location.origin;
          } catch {
            return false;
          }
        }),
      };
    `,
  );
  const expectedLogoPath =
    expectedTheme === "dark"
      ? "/graphics_theme/Dark_Theme/logo/graphite_logo.svg"
      : "/graphics_theme/Light_Theme/logo/graphite_logo.svg";
  const expectedLogoStroke = expectedTheme === "dark" ? "#F4F3EF" : "#292B2E";
  const expectedThemeColor = expectedTheme === "dark" ? "#292b2e" : "#f4f3ef";
  const decodedLogoSource = state.logoSource
    ? decodeURIComponent(state.logoSource)
    : "";

  if (state.theme !== expectedTheme) {
    throw new Error(
      `Expected ${expectedTheme} theme, received ${state.theme}.`,
    );
  }
  if (state.themeColor.toLowerCase() !== expectedThemeColor) {
    throw new Error(
      `Expected ${expectedTheme} browser theme color ${expectedThemeColor}, received ${state.themeColor}.`,
    );
  }
  if (
    !state.logoSource?.includes(expectedLogoPath) &&
    !decodedLogoSource.includes(`stroke='${expectedLogoStroke}'`) &&
    !decodedLogoSource.includes(`stroke="${expectedLogoStroke}"`)
  ) {
    throw new Error(
      `Expected ${expectedTheme} logo ${expectedLogoPath}, received ${state.logoSource}.`,
    );
  }
  if (!/^['"]?Inter Variable/.test(state.fontFamily) || !state.interLoaded) {
    throw new Error(`Inter is not active: ${JSON.stringify(state)}.`);
  }
  if (state.externalResources.length > 0) {
    throw new Error(
      `Unexpected external runtime resources: ${state.externalResources.join(", ")}.`,
    );
  }
  return state;
}

async function inspectSearchSurface(sessionId, expectedTheme, expectedState) {
  // Wait for the bounded entrance before checking final panel geometry.
  await delay(450);
  const state = await executeScript(
    sessionId,
    `
      const backdrop = document.querySelector(".search-backdrop");
      const surface = document.querySelector(".search-surface");
      const input = document.querySelector(".search-input-row input");
      const inputRow = document.querySelector(".search-input-row");
      const surfaceRect = surface.getBoundingClientRect();
      const backdropRect = backdrop.getBoundingClientRect();
      return {
        theme: document.documentElement.dataset.theme,
        role: surface.getAttribute("role"),
        modal: surface.getAttribute("aria-modal"),
        inputFocused: document.activeElement === input,
        focusWithin: inputRow.matches(":focus-within"),
        surface: {
          left: surfaceRect.left,
          right: surfaceRect.right,
          top: surfaceRect.top,
          width: surfaceRect.width,
          height: surfaceRect.height,
        },
        backdrop: {
          width: backdropRect.width,
          height: backdropRect.height,
        },
        viewport: {
          width: window.innerWidth,
          height: window.innerHeight,
        },
        empty: Boolean(document.querySelector(".search-empty")),
        noResults: Boolean(document.querySelector(".no-results")),
        resultCount: document.querySelectorAll(".search-result-open").length,
        activeResultCount: document.querySelectorAll(".search-result-open.is-active").length,
        retrievalModeCount: document.querySelectorAll(".search-retrieval-modes button").length,
        citationFilterCount: document.querySelectorAll(".search-citation-filters button").length,
        footerVisible: Boolean(document.querySelector(".search-footer")),
      };
    `,
  );

  const stateMismatch =
    (expectedState === "empty" && !state.empty) ||
    (expectedState === "no-results" && !state.noResults) ||
    (expectedState === "results" &&
      (state.resultCount === 0 || state.activeResultCount !== 1));
  const invalid =
    state.theme !== expectedTheme ||
    state.role !== "dialog" ||
    state.modal !== "true" ||
    !state.inputFocused ||
    !state.focusWithin ||
    state.surface.left < 49 ||
    state.surface.right > state.viewport.width - 49 ||
    state.surface.top < 77 ||
    state.surface.width > 701 ||
    state.backdrop.width !== state.viewport.width ||
    state.backdrop.height !== state.viewport.height ||
    state.retrievalModeCount !== 3 ||
    state.citationFilterCount !== 5 ||
    !state.footerVisible ||
    stateMismatch;
  if (invalid) {
    throw new Error(
      `Search ${expectedState} baseline failed in ${expectedTheme}: ${JSON.stringify(state)}.`,
    );
  }
  return state;
}

async function chooseTheme(sessionId, theme) {
  await executeScript(
    sessionId,
    `
    document.querySelector('[aria-label="Switch to ${theme} mode"]')?.click();
  `,
  );
  await delay(240);
}

async function inspectPaperTypeColor(sessionId, expectedTheme) {
  const state = await executeScript(
    sessionId,
    `
      const label = document.querySelector(".paper-kicker span:first-child");
      return {
        color: label ? getComputedStyle(label).color : null,
        inlineStyle: label?.getAttribute("style") ?? null,
      };
    `,
  );
  const expectedColor =
    expectedTheme === "dark" ? "rgb(209, 176, 115)" : "rgb(128, 99, 53)";
  if (state.color !== expectedColor || state.inlineStyle !== null) {
    throw new Error(
      `Paper type does not use the ${expectedTheme} palette contract: ${JSON.stringify(state)}.`,
    );
  }
  return state;
}

async function pressTab(sessionId) {
  await driverRequest("POST", `/session/${sessionId}/actions`, {
    actions: [
      {
        type: "key",
        id: "keyboard",
        actions: [
          { type: "keyDown", value: "\uE004" },
          { type: "keyUp", value: "\uE004" },
        ],
      },
    ],
  });
  await driverRequest("DELETE", `/session/${sessionId}/actions`);
}

async function reloadApp(sessionId) {
  await driverRequest("POST", `/session/${sessionId}/url`, {
    url: appUrl,
  });
  await waitForElement(
    sessionId,
    ".workspace-view-layer--graph.is-active .graph-canvas canvas",
  );
  await delay(700);
}

async function runJourney(sessionId) {
  const verification = {};
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
  await delay(900);
  verification.isolation = await assertBrowserQaIsolation(
    driverRequest,
    sessionId,
  );

  await executeScript(
    sessionId,
    "localStorage.removeItem(document.documentElement.dataset.themeStorageKey);",
  );
  await reloadApp(sessionId);
  verification.systemPreference = await executeScript(
    sessionId,
    `return {
      theme: document.documentElement.dataset.theme,
      prefersDark: matchMedia("(prefers-color-scheme: dark)").matches,
      storedTheme: localStorage.getItem(
        document.documentElement.dataset.themeStorageKey,
      ),
    };`,
  );
  if (
    verification.systemPreference.theme !==
    (verification.systemPreference.prefersDark ? "dark" : "light")
  ) {
    throw new Error(
      `Initial theme did not follow the system preference: ${JSON.stringify(verification.systemPreference)}.`,
    );
  }

  await chooseTheme(sessionId, "dark");
  verification.darkSelected = await inspectTheme(sessionId, "dark");
  await capture(sessionId, "01-dark-settings.png");
  await capture(sessionId, "01-dark-graph.png");
  await click(sessionId, ".graph-topic-controls > button", 0);
  await waitForElement(sessionId, ".topic-map-popover:popover-open");
  verification.unpreparedTopics = await executeScript(
    sessionId,
    `return {
    message: document.querySelector('.topic-map-popover').textContent,
    disabled: document.querySelectorAll('.graph-topic-controls > button')[1].disabled
  };`,
  );
  assert.equal(verification.unpreparedTopics.disabled, true);
  assert.ok(
    verification.unpreparedTopics.message.includes(
      "Semantic indexing is required",
    ),
  );
  await capture(sessionId, "01-topic-preparation-required.png");
  await click(sessionId, ".topic-map-popover > button");
  await waitForElement(sessionId, ".source-popover");
  await click(sessionId, ".source-trigger");
  await executeScript(
    sessionId,
    "document.querySelector('.topic-map-popover').hidePopover();",
  );

  verification.darkGraphContrast = await inspectGraphContrast(
    sessionId,
    "dark",
  );
  await movePointer(sessionId, 611, 409);
  await delay(180);
  await capture(sessionId, "01-dark-node-hover.png");
  await reloadApp(sessionId);
  verification.darkReloaded = await inspectTheme(sessionId, "dark");

  await click(sessionId, ".source-trigger");
  await waitForElement(sessionId, ".source-popover");
  await delay(220);
  await capture(sessionId, "01-dark-data-source.png");
  await click(sessionId, ".source-trigger");

  await chooseTheme(sessionId, "light");
  verification.lightSelected = await inspectTheme(sessionId, "light");
  await capture(sessionId, "01-light-settings.png");
  await capture(sessionId, "01-light-graph.png");
  verification.lightGraphContrast = await inspectGraphContrast(
    sessionId,
    "light",
  );
  await movePointer(sessionId, 611, 409);
  await delay(180);
  await capture(sessionId, "01-light-node-hover.png");
  await reloadApp(sessionId);
  verification.lightReloaded = await inspectTheme(sessionId, "light");
  await pressTab(sessionId);
  verification.keyboardFocus = await executeScript(
    sessionId,
    `
      const active = document.activeElement;
      const style = getComputedStyle(active);
      const rect = active.getBoundingClientRect();
      return {
        className: active.className,
        focusVisible: active.matches(":focus-visible"),
        outlineStyle: style.outlineStyle,
        outlineWidth: style.outlineWidth,
        top: rect.top,
      };
    `,
  );
  if (
    verification.keyboardFocus.className !== "skip-link" ||
    !verification.keyboardFocus.focusVisible ||
    verification.keyboardFocus.outlineStyle === "none" ||
    Number.parseFloat(verification.keyboardFocus.outlineWidth) < 2 ||
    verification.keyboardFocus.top < 0
  ) {
    throw new Error(
      `Keyboard focus is not clearly visible: ${JSON.stringify(verification.keyboardFocus)}.`,
    );
  }
  await capture(sessionId, "01-light-keyboard-focus.png");

  await click(sessionId, ".search-trigger");
  await waitForElement(sessionId, ".search-empty");
  await delay(240);
  verification.searchEmptyLight = await inspectSearchSurface(
    sessionId,
    "light",
    "empty",
  );
  await capture(sessionId, "02-search-empty-light.png");
  await typeText(
    sessionId,
    ".search-input-row input",
    "no-match-graphite-visual-fixture",
  );
  await waitForElement(sessionId, ".no-results");
  await delay(180);
  verification.searchNoResultsLight = await inspectSearchSurface(
    sessionId,
    "light",
    "no-results",
  );
  await capture(sessionId, "02-search-no-results-light.png");
  await clearText(sessionId, ".search-input-row input");
  await click(sessionId, ".search-citation-filters button", 2);
  await waitForElement(sessionId, ".no-results");
  assert.equal(
    (await findElements(sessionId, ".search-result-open")).length,
    0,
  );
  await capture(sessionId, "02-unindexed-citation-filter.png");
  await click(sessionId, ".search-citation-filters button", 0);
  await typeText(sessionId, ".search-input-row input", "Bayesian");
  await waitForElement(sessionId, ".search-result-open");
  verification.searchResultsLight = await inspectSearchSurface(
    sessionId,
    "light",
    "results",
  );
  await capture(sessionId, "02-real-metadata-search.png");
  await click(sessionId, ".search-result-open");
  await waitForElement(sessionId, ".paper-inspector");
  await delay(350);
  verification.lightPaperType = await inspectPaperTypeColor(sessionId, "light");
  assert.equal(
    await executeScript(
      sessionId,
      "return document.querySelector('.paper-inspector h2').textContent.includes('Bayesian');",
    ),
    true,
  );
  await capture(sessionId, "02-real-paper-inspector.png");
  await chooseTheme(sessionId, "dark");
  verification.darkPaperType = await inspectPaperTypeColor(sessionId, "dark");
  await capture(sessionId, "02-real-paper-inspector-dark.png");
  await click(sessionId, ".search-trigger");
  await waitForElement(sessionId, ".search-result-open");
  await delay(200);
  verification.searchResultsDark = await inspectSearchSurface(
    sessionId,
    "dark",
    "results",
  );
  verification.semanticDisabled = await executeScript(
    sessionId,
    "return [...document.querySelectorAll('.search-retrieval-modes button')].slice(1).every(button => button.disabled);",
  );
  assert.equal(verification.semanticDisabled, true);
  await capture(sessionId, "02-unindexed-semantic-search.png");
  await click(sessionId, ".search-input-row > button");
  await waitForAbsence(sessionId, ".search-surface");

  await scrollIntoView(sessionId, ".primary-action");
  await click(sessionId, ".primary-action");
  await waitForElement(sessionId, ".reader-view");
  await delay(250);
  assert.equal(
    await executeScript(
      sessionId,
      "return document.querySelector('.reader-view').textContent.includes('desktop app');",
    ),
    true,
  );
  assert.equal(
    (await findElements(sessionId, ".pdf-page-stage canvas")).length,
    0,
  );
  await capture(sessionId, "03-pdf-desktop-required.png");
  await click(sessionId, ".back-button");
  await waitForElement(sessionId, ".graph-canvas canvas");
  await click(sessionId, '[aria-label="Close paper inspector"]');

  await click(sessionId, ".agent-launcher");
  await waitForElement(sessionId, ".agent-preparation");
  assert.equal(
    await executeScript(
      sessionId,
      "return document.querySelector('.agent-preparation').textContent.includes('desktop app');",
    ),
    true,
  );
  await click(sessionId, ".agent-setup-trigger");
  await waitForElement(sessionId, ".agent-setup-panel");
  await delay(300);
  await capture(sessionId, "04-browser-agent-capability.png");
  assert.equal((await findElements(sessionId, ".agent-answer-text")).length, 0);
  await click(sessionId, ".agent-identity");
  await waitForAbsence(sessionId, ".agent-bar");

  await click(sessionId, ".library-rail nav .rail-item");
  await click(sessionId, ".view-switcher button", 1);
  await waitForElement(sessionId, ".analytics-view");
  await delay(250);
  verification.realYearCounts = await executeScript(
    sessionId,
    `
    return [...document.querySelectorAll('.year-chart > button')]
      .map(button => button.getAttribute('aria-label'));
  `,
  );
  assert.ok(verification.realYearCounts.length > 1);
  await capture(sessionId, "05-real-analytics-dark.png");
  await chooseTheme(sessionId, "light");
  await capture(sessionId, "05-real-analytics-light.png");
  await click(sessionId, ".year-chart > button", 0);
  await waitForElement(sessionId, ".year-paper-list button");
  await capture(sessionId, "05-real-year-papers.png");
  await click(sessionId, ".year-paper-list button");
  await waitForElement(sessionId, ".paper-inspector");
  await click(sessionId, '[aria-label="Close paper inspector"]');

  const beforeWidth = await executeScript(
    sessionId,
    "return document.querySelector('.library-rail').getBoundingClientRect().width;",
  );
  await dragHorizontally(
    sessionId,
    '[aria-label="Resize library sidebar"]',
    30,
  );
  const afterWidth = await executeScript(
    sessionId,
    "return document.querySelector('.library-rail').getBoundingClientRect().width;",
  );
  assert.ok(afterWidth > beforeWidth);
  verification.libraryResize = { beforeWidth, afterWidth };
  await click(sessionId, '[aria-label="Collapse library navigation"]');
  await waitForElement(sessionId, ".library-rail.is-collapsed");
  await delay(200);
  await capture(sessionId, "06-collapsed-navigation.png");
  await click(sessionId, '[aria-label="Expand library navigation"]');

  await driverRequest("POST", `/session/${sessionId}/window/rect`, {
    width: 1024,
    height: 700,
  });
  await delay(400);
  await click(sessionId, ".search-trigger");
  await waitForElement(sessionId, ".search-surface");
  await click(sessionId, ".search-retrieval-modes button", 0);
  await clearText(sessionId, ".search-input-row input");
  await typeText(sessionId, ".search-input-row input", "Bayesian");
  await waitForElement(sessionId, ".search-result-open");
  verification.compactSearch = await inspectSearchSurface(
    sessionId,
    "light",
    "results",
  );
  await capture(sessionId, "07-compact-real-search.png");
  await click(sessionId, ".search-input-row > button");
  await waitForAbsence(sessionId, ".search-backdrop");
  await click(sessionId, ".agent-launcher");
  await waitForElement(sessionId, ".agent-bar");
  await delay(350);
  verification.compactComposer = await executeScript(
    sessionId,
    `
    const box = document.querySelector('.agent-bar').getBoundingClientRect();
    return { left: box.left, right: box.right, bottom: box.bottom,
      width: innerWidth, height: innerHeight };
  `,
  );
  assert.ok(verification.compactComposer.left >= 0);
  assert.ok(
    verification.compactComposer.right <= verification.compactComposer.width,
  );
  assert.ok(
    verification.compactComposer.bottom <= verification.compactComposer.height,
  );
  await capture(sessionId, "07-compact-agent.png");
  await click(sessionId, ".agent-identity");
  await chooseTheme(sessionId, "dark");
  await capture(sessionId, "07-compact-graph-dark.png");
  verification.coverage = {
    data: "Current real-paper metadata adapter; no synthetic research evidence injected.",
    checked: [
      "light/dark theme persistence",
      "graph contrast",
      "keyboard focus",
      "metadata search and empty results",
      "unindexed citation and semantic results",
      "paper inspector",
      "PDF desktop requirement",
      "agent desktop requirement",
      "publication-year navigation",
      "sidebar resize/collapse",
      "compact layout",
    ],
    outsideBrowserPreview: [
      "PDF rendering and editing: qa:pdf-editor with a verified real PDF",
      "live citation extraction, semantic retrieval and model answers: native integration suites",
      "recorded response citation navigation: real-demo-preview --rag-replay",
    ],
  };
  await writeFile(
    resolve(outputDirectory, "verification.json"),
    `${JSON.stringify(verification, null, 2)}\n`,
  );
}

const runtime = await prepareQaRuntime("visual");
appUrl = runtime.appUrl;
driverRequest = runtime.request;
outputDirectory = runtime.outputDirectory;

let sessionId;
try {
  const session = await runtime.createFirefoxSession();
  sessionId = session.sessionId;
  await runJourney(sessionId);
  process.stdout.write(`${outputDirectory}\n`);
} catch (error) {
  if (sessionId) await capture(sessionId, "failure.png");
  throw error;
} finally {
  if (sessionId) {
    await driverRequest("DELETE", `/session/${sessionId}`).catch(() => {});
  }
  await runtime.stop();
}
