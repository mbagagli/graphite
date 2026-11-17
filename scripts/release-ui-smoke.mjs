import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { readFile, writeFile } from "node:fs/promises";
import { relative, resolve } from "node:path";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath, URL } from "node:url";

import { assertBrowserQaIsolation, prepareQaRuntime } from "./qa-runtime.mjs";

const { version } = JSON.parse(
  await readFile(new URL("../package.json", import.meta.url), "utf8"),
);
const runtime = await prepareQaRuntime("release-ui");
let sessionId;

async function evaluate(script, args = []) {
  return runtime.request("POST", `/session/${sessionId}/execute/sync`, {
    script,
    args,
  });
}

async function waitFor(script) {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    const value = await evaluate(script);
    if (value) return value;
    await delay(100);
  }
  throw new Error("Release UI did not reach the expected state.");
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
        id: "release-keyboard",
        actions: [
          { type: "keyDown", value },
          { type: "keyUp", value },
        ],
      },
    ],
  });
}

async function capture(name) {
  const encoded = await runtime.request(
    "GET",
    `/session/${sessionId}/screenshot`,
  );
  await writeFile(
    resolve(runtime.outputDirectory, `${name}.png`),
    Buffer.from(encoded, "base64"),
  );
}

async function closeAndCheckFocus(selector) {
  await key("\uE00C");
  await waitFor("return !document.querySelector('dialog[open]');");
  assert.equal(
    await evaluate(
      "return document.activeElement === document.querySelector(arguments[0]);",
      [selector],
    ),
    true,
    "Closing a dialog must restore the opener's keyboard focus.",
  );
}

async function checkYearScrollbar() {
  await waitFor("return document.querySelector('.horizontal-scrollbar');");
  await waitFor(`const chart=document.querySelector('.year-chart');
    const bar=document.querySelector('.horizontal-scrollbar');
    return Number(bar.getAttribute('aria-valuemax')) === Math.round(chart.scrollWidth-chart.clientWidth);`);
  await evaluate("document.querySelector('.horizontal-scrollbar').focus();");
  await key("\uE010");
  await waitFor(`const chart=document.querySelector('.year-chart');
    return Math.abs(chart.scrollLeft-(chart.scrollWidth-chart.clientWidth))<=1;`);
  assert.equal(
    await evaluate(`
    const chart = document.querySelector('.year-chart');
    const last = chart.lastElementChild.getBoundingClientRect();
    return last.right <= chart.getBoundingClientRect().right + 1;
  `),
    true,
  );
  await key("\uE011");
  await waitFor(
    "return document.querySelector('.year-chart').scrollLeft === 0;",
  );
  await runtime.request("POST", `/session/${sessionId}/actions`, {
    actions: [
      {
        type: "pointer",
        id: "year-pointer",
        parameters: { pointerType: "mouse" },
        actions: [
          {
            type: "pointerMove",
            origin: "viewport",
            x: 30,
            y: 25,
            duration: 0,
          },
        ],
      },
    ],
  });
  assert.equal(
    await evaluate(`
    const chart = document.querySelector('.year-chart');
    const slider = document.querySelector('.horizontal-scrollbar');
    return !slider.matches(':hover') && slider.getBoundingClientRect().height >= 14 &&
      getComputedStyle(slider).visibility === 'visible' &&
      chart.querySelectorAll('button > small').length === chart.children.length;
  `),
    true,
  );
}

async function runJourney() {
  const verification = { version, viewports: [] };
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

  assert.equal(
    await evaluate(
      "return document.querySelector('.topic-map-popover').matches(':popover-open');",
    ),
    false,
  );
  await click(".graph-topic-controls > button");
  await waitFor(
    "return document.querySelector('.topic-map-popover').matches(':popover-open');",
  );
  const topicCount = await evaluate(`
    return [...document.querySelectorAll('.topic-map-popover li small')]
      .reduce((sum, count) => sum + Number(count.textContent), 0);
  `);
  assert.equal(topicCount, 0);
  assert.equal(
    await evaluate(
      "return document.querySelectorAll('.graph-topic-controls > button')[1].disabled;",
    ),
    true,
  );
  await capture("topic-map-open");
  await key("\uE00C");
  assert.equal(
    await evaluate(
      "return document.querySelector('.topic-map-popover').matches(':popover-open');",
    ),
    false,
  );
  await delay(2000);
  await click('[aria-label="Fit graph"]');
  await delay(1000);
  await capture("workspace-desktop");
  verification.topicControls = true;
  process.stdout.write(
    "Topic disclosure and preparation prerequisite verified.\n",
  );

  for (const selector of [
    "#library-collections .rail-list button",
    'nav[aria-label="Library navigation"] .rail-item',
  ]) {
    await click(selector);
    await waitFor(`
      const label = [...document.querySelectorAll(
        '.graph-control-popover label',
      )].find((item) => item.textContent.includes('Shared tags'));
      return label && !label.querySelector('input')?.checked &&
        !document.querySelector('.graph-preparing');
    `);
  }
  verification.demoConnectionsSurviveLibraryNavigation = true;

  await click(".source-trigger");
  await waitFor("return document.querySelector('#source-status-popover');");
  assert.match(
    await evaluate(
      "return document.querySelector('.source-popover').textContent;",
    ),
    /60 real seismology papers/,
  );
  await key("\uE00C");
  verification.zoteroSourceAccessible = true;
  process.stdout.write("Source control verified.\n");

  await click(".top-help-button", 1);
  await waitFor("return document.querySelector('dialog[open] #about-title');");
  assert.equal(
    await evaluate(
      "return document.querySelector('.about-details dd').textContent;",
    ),
    version,
  );
  assert.equal(
    await evaluate(
      "return document.querySelectorAll('.about-details dd')[1].textContent;",
    ),
    "Graphite - Discover Knowledge",
  );
  await capture("about-desktop");
  await closeAndCheckFocus(".top-help-button + .top-help-button");
  verification.aboutVersionMatches = true;
  process.stdout.write("About version verified.\n");

  for (const dimensions of [
    { width: 1440, height: 900, label: "desktop" },
    { width: 1024, height: 700, label: "compact" },
  ]) {
    await runtime.request("POST", `/session/${sessionId}/window/rect`, {
      width: dimensions.width,
      height: dimensions.height,
    });
    await click(".top-help-button");
    await waitFor("return document.querySelector('dialog.help-dialog[open]');");
    const titles = ["START-UP GUIDE", "HOW-TO", "RAG-SYSTEM", "TIPS"];
    for (let index = 0; index < titles.length; index += 1) {
      await click(".help-tabs [role=tab]", index);
      assert.equal(
        await evaluate(
          "return document.querySelector('[role=tab][aria-selected=true]').textContent;",
        ),
        titles[index],
      );
      assert.ok(
        await evaluate(
          "return document.querySelector('[role=tabpanel]').textContent.length > 300;",
        ),
      );
    }
    await key("\uE011");
    await key("\uE014");
    assert.equal(
      await evaluate("return document.activeElement.textContent;"),
      "HOW-TO",
    );
    await key("\uE010");
    assert.equal(
      await evaluate("return document.activeElement.textContent;"),
      "TIPS",
    );
    await capture(`help-${dimensions.label}`);
    assert.equal(
      await evaluate(`
        const dialog = document.querySelector('dialog[open]');
        const bounds = dialog.getBoundingClientRect();
        return bounds.left >= 0 && bounds.right <= innerWidth &&
          bounds.top >= 0 && bounds.bottom <= innerHeight;
      `),
      true,
    );
    await closeAndCheckFocus(".top-help-button");

    await click(".view-switcher button", 1);
    await waitFor("return document.querySelector('.year-chart > button');");
    const overflow = await evaluate(
      "const chart=document.querySelector('.year-chart'); return chart.scrollWidth > chart.clientWidth;",
    );
    if (overflow) await checkYearScrollbar();
    await capture(`year-navigation-${dimensions.label}`);
    const yearLabel = await evaluate(`
      const button = document.querySelector('.year-chart > button');
      button.focus();
      return button.getAttribute('aria-label');
    `);
    await key("\uE007");
    await waitFor(
      "return document.querySelector('.year-papers-dialog[open]');",
    );
    const expectedCount = Number(yearLabel.match(/: (\d+) papers?/)[1]);
    assert.equal(
      await evaluate(
        "return document.querySelectorAll('.year-paper-list li').length;",
      ),
      expectedCount,
    );
    await capture(`year-${dimensions.label}`);
    await closeAndCheckFocus(".year-chart > button");
    verification.viewports.push({ ...dimensions, yearLabel, expectedCount });
    process.stdout.write(
      `Help and year browsing verified: ${dimensions.label}.\n`,
    );
  }

  for (const mode of ["3d", "2d"]) {
    if (mode === "2d") {
      await click('[aria-label="Graph dimensions"] button', 1);
      await delay(700);
      await click(".view-switcher button", 1);
    }
    await click(".year-chart > button");
    await waitFor("return document.querySelector('.year-paper-list button');");
    const paperTitle = await evaluate(
      "return document.querySelector('.year-paper-list strong').textContent;",
    );
    await click(".year-paper-list button");
    await waitFor(
      "return document.querySelector('.paper-inspector') && document.querySelector('.graph-canvas canvas') && !document.querySelector('.graph-preparing');",
    );
    await delay(700);
    assert.ok(
      (
        await evaluate(
          "return document.querySelector('.paper-inspector').textContent;",
        )
      ).includes(paperTitle),
    );
    assert.ok(
      (
        await evaluate(
          "return document.querySelector('.graph-selection strong').textContent;",
        )
      ).includes(paperTitle),
    );
    await capture(`year-paper-focused-${mode}`);

    // Hit the actual paper at the standard focus point after clearing selection.
    const point = await evaluate(`
      const box = document.querySelector('.graph-canvas').getBoundingClientRect();
      return { x: Math.round(box.left + box.width * 0.35),
        y: Math.round(box.top + box.height * 0.5) };
    `);
    await click('[aria-label="Close paper inspector"]');
    await waitFor("return !document.querySelector('.paper-inspector');");
    await runtime.request("POST", `/session/${sessionId}/actions`, {
      actions: [
        {
          type: "pointer",
          id: "year-pointer",
          parameters: { pointerType: "mouse" },
          actions: [
            {
              type: "pointerMove",
              origin: "viewport",
              x: 30,
              y: 25,
              duration: 0,
            },
            { type: "pause", duration: 100 },
            {
              type: "pointerMove",
              origin: "viewport",
              ...point,
              duration: 100,
            },
            { type: "pause", duration: 250 },
            { type: "pointerDown", button: 0 },
            { type: "pause", duration: 50 },
            { type: "pointerUp", button: 0 },
          ],
        },
      ],
    });
    await waitFor("return document.querySelector('.paper-inspector');");
    assert.equal(
      await evaluate(
        "return document.querySelector('.paper-identity h2').textContent;",
      ),
      paperTitle,
      "Analytics must frame the selected graph node.",
    );
    verification[`paperFocus${mode}`] = true;
  }
  verification.paperNavigation = true;
  process.stdout.write("Analytics node focus verified in 3D and 2D.\n");

  const fixturePath = resolve(runtime.outputDirectory, "long-list.html");
  const fixtureRoute = relative(
    fileURLToPath(new URL("../", import.meta.url)),
    fixturePath,
  );
  assert.ok(
    !fixtureRoute.startsWith(".."),
    "QA artifacts must be inside the checkout.",
  );
  await writeFile(
    fixturePath,
    `<!doctype html>
<html lang="en" data-theme="dark">
<head><meta charset="utf-8"><title>Publication year scroll fixture</title></head>
<body><div id="fixture"></div><script type="module">
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource-variable/inter/wght.css";
import "/src/design-system/tokens.css";
import "/src/design-system/global.css";
import { YearPapersDialog } from "/src/features/analytics/YearPapersDialog.tsx";
import { demoItems } from "/src/adapters/demo/demoData.ts";
const root = createRoot(document.getElementById("fixture"));
const papers = Array.from({ length: 1200 }, (_, index) => ({
  ...demoItems[0], id: "qa-" + index,
  title: "Long-list fixture " + String(index).padStart(4, "0"), year: 2020,
}));
root.render(createElement(YearPapersDialog, {
  year: 2020, papers, onClose: () => root.unmount(), onSelectPaper: () => {},
}));
</script></body></html>`,
  );
  await runtime.request("POST", `/session/${sessionId}/url`, {
    url: new URL(fixtureRoute, `${runtime.appUrl}/`).href,
  });
  await waitFor(
    "return document.querySelectorAll('.year-paper-list li').length === 1200;",
  );
  process.stdout.write("Long-list fixture mounted.\n");
  verification.longList = await evaluate(`
    const area = document.querySelector('.year-papers-dialog .modal-dialog-body');
    area.scrollTop = area.scrollHeight;
    const last = document.querySelector('.year-paper-list li:last-child');
    const bounds = last.getBoundingClientRect();
    const viewport = area.getBoundingClientRect();
    return {
      count: document.querySelectorAll('.year-paper-list li').length,
      scrollable: area.scrollHeight > area.clientHeight,
      lastVisible: bounds.bottom <= viewport.bottom && bounds.top >= viewport.top,
    };
  `);
  assert.deepEqual(verification.longList, {
    count: 1200,
    scrollable: true,
    lastVisible: true,
  });
  await capture("long-list-compact");
  await key("\uE00C");
  const scrollbarPath = resolve(runtime.outputDirectory, "year-scrollbar.html");
  await writeFile(
    scrollbarPath,
    `<!doctype html>
<html lang="en" data-theme="dark"><head><meta charset="utf-8"><title>Year scrollbar interaction fixture</title></head>
<body><div id="fixture"></div><script type="module">
import {createElement} from "react";
import {createRoot} from "react-dom/client";
import "/src/design-system/tokens.css";
import "/src/design-system/global.css";
import {AnalyticsView} from "/src/features/analytics/AnalyticsView.tsx";
import {browserWorkspace} from "/src/adapters/demo/browserDiagnostics.ts";
import {demoItems} from "/src/adapters/demo/demoData.ts";
const items = Array.from({length:40}, (_,index) => ({...demoItems[0],id:"scroll-"+index,year:1980+index}));
createRoot(document.getElementById("fixture")).render(createElement(AnalyticsView, {
analytics:browserWorkspace.analytics,topics:browserWorkspace.snapshot.topics,items,
onSelectPaper:()=>{},provenance:"real-demo",scopeLabel:"Year scrolling test",totalItemCount:40}));
</script></body></html>`,
  );
  await runtime.request("POST", `/session/${sessionId}/url`, {
    url: new URL(
      relative(fileURLToPath(new URL("../", import.meta.url)), scrollbarPath),
      `${runtime.appUrl}/`,
    ).href,
  });
  for (const dimensions of [
    { width: 1440, height: 900, label: "desktop" },
    { width: 1024, height: 700, label: "compact" },
  ]) {
    await runtime.request("POST", `/session/${sessionId}/window/rect`, {
      width: dimensions.width,
      height: dimensions.height,
    });
    await checkYearScrollbar();
    await capture(`year-scrollbar-${dimensions.label}`);
  }
  verification.persistentYearNavigation = true;
  return verification;
}

try {
  ({ sessionId } = await runtime.createFirefoxSession());
  const verification = await runJourney();
  await writeFile(
    resolve(runtime.outputDirectory, "verification.json"),
    `${JSON.stringify(verification, null, 2)}\n`,
  );
} catch (error) {
  if (sessionId) await capture("failure");
  throw error;
} finally {
  if (sessionId) {
    await runtime.request("DELETE", `/session/${sessionId}`).catch(() => {});
  }
  await runtime.stop();
}
process.stdout.write(
  "Release UI smoke passed; evidence: artifacts/qa/release-ui\n",
);
