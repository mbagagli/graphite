import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { writeFile, rm } from "node:fs/promises";
import { relative, resolve } from "node:path";
import { fileURLToPath, URL } from "node:url";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";
import { prepareQaRuntime } from "./qa-runtime.mjs";

const runtime = await prepareQaRuntime("graph-picking");
const root = fileURLToPath(new URL("../", import.meta.url));
const fixture = resolve(runtime.outputDirectory, "picking-browser.html");
const route = relative(root, fixture);
assert.ok(!route.startsWith(".."));
let sessionId;
const harness = `<!doctype html><html lang="en" data-theme="dark">
<head><meta charset="utf-8"><title>Graph picking regression</title>
<style>#root{height:calc(100vh - 40px);position:relative}</style></head>
<body><div id="root"></div><script type="module">
import {createElement as h} from 'react';
import {createRoot} from 'react-dom/client';
import {GraphCanvas} from '/src/features/graph/GraphCanvas.tsx';
import {projectionKey} from '/src/features/graph/graphLayout.ts';
import '/src/design-system/tokens.css';
import '/src/design-system/global.css';
const root = createRoot(document.getElementById('root'));
const qa = window.__pickQa = {errors:[], clicks:[], backgrounds:0, serial:0};
window.addEventListener('error', e => qa.errors.push(e.message));
window.addEventListener('unhandledrejection', e => qa.errors.push(String(e.reason)));
qa.render = (mode, depth, theme) => {
 document.documentElement.dataset.theme = theme;
 qa.clicks=[]; qa.backgrounds=0; qa.hovered=null;
 const ids = mode === '3d' ? ['back','front'] : ['front'];
 const graph = {nodes:ids.map(id=>({id,paperId:id,entityKind:'research-item',
   label:id,detail:null,topicId:'uncategorized',x:0,y:0,importance:1})),
   edges:[],availableRelations:[],totalNodeCount:ids.length,hasMore:false};
 const positions = new Map(ids.map(id=>[id,{x:0,y:0,z:id==='back'?-80:0}]));
 const camera = {position:{x:0,y:0,z:depth},target:{x:0,y:0,z:0}};
 qa.controls={current:null};
 qa.session={current:{startupPlayed:true,cameras:{'2d':camera,'3d':camera},
   layouts:{key:projectionKey(graph),positions:{'2d':positions,'3d':positions},preparationMs:0}}};
 root.render(h(GraphCanvas,{key:++qa.serial,graph,mode,grouping:'relations',
   topics:[],selectedIds:[],highlightedIds:[],layers:{},theme,
   controlsRef:qa.controls,sessionRef:qa.session,focusSerialRef:{current:0},
   onSelectNode:(node,additive)=>qa.clicks.push({id:node.id,additive}),
   onHoverNode:node=>qa.hovered=node?.id??null,
   onSelectEdge:()=>qa.errors.push('Unexpected edge'),
   onClearSelection:()=>qa.backgrounds++,onFailure:()=>qa.errors.push('Graph failed')}));
};
qa.render('3d',1800,'dark');
</script></body></html>`;

async function evaluate(script, args = []) {
  return runtime.request("POST", `/session/${sessionId}/execute/sync`, {
    script,
    args,
  });
}

async function actions(steps, modifier = false) {
  const pointer = {
    type: "pointer",
    id: "mouse",
    parameters: { pointerType: "mouse" },
    actions: steps,
  };
  await runtime.request("POST", `/session/${sessionId}/actions`, {
    actions: modifier
      ? [
          pointer,
          {
            type: "key",
            id: "keyboard",
            actions: [
              { type: "keyDown", value: "\uE008" },
              ...steps.slice(1, -1).map(() => ({ type: "pause", duration: 0 })),
              { type: "keyUp", value: "\uE008" },
            ],
          },
        ]
      : [pointer],
  });
}

async function point(offset, click = false, modifier = false) {
  const bounds = await evaluate(`
    const r=document.querySelector('canvas').getBoundingClientRect();
    return {x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2)};
  `);
  await actions(
    [
      { type: "pointerMove", duration: 0, x: bounds.x + offset, y: bounds.y },
      { type: "pause", duration: 150 },
      ...(click
        ? [
            { type: "pointerDown", button: 0 },
            { type: "pointerUp", button: 0 },
          ]
        : []),
    ],
    modifier,
  );
  if (click) {
    await delay(80);
    await evaluate("window.__pickQa.controls.current.navigate();");
  }
}

try {
  await writeFile(fixture, harness);
  ({ sessionId } = await runtime.createFirefoxSession());
  await runtime.request("POST", `/session/${sessionId}/window/rect`, {
    width: 1200,
    height: 850,
  });
  await runtime.request("POST", `/session/${sessionId}/url`, {
    url: `${runtime.appUrl}/${route}`,
  });
  for (let attempt = 0; attempt < 100; attempt++) {
    if (await evaluate("return Boolean(window.__pickQa?.controls.current);"))
      break;
    await delay(100);
  }
  for (const mode of ["3d", "2d"]) {
    for (const depth of [1800, 18000]) {
      for (const theme of ["dark", "light"]) {
        await evaluate("window.__pickQa.render(...arguments);", [
          mode,
          depth,
          theme,
        ]);
        await delay(400);
        await point(10);
        assert.equal(
          await evaluate("return window.__pickQa.hovered;"),
          "front",
        );
        await point(10, true, true);
        assert.deepEqual(await evaluate("return window.__pickQa.clicks;"), [
          { id: "front", additive: true },
        ]);
        await point(30, true);
        assert.equal(await evaluate("return window.__pickQa.backgrounds;"), 1);
        assert.equal(
          await evaluate("return window.__pickQa.clicks.length;"),
          1,
        );
      }
    }
  }
  await runtime.request("POST", `/session/${sessionId}/window/rect`, {
    width: 1024,
    height: 768,
  });
  await delay(300);
  await point(10, true);
  assert.deepEqual(await evaluate("return window.__pickQa.clicks.at(-1);"), {
    id: "front",
    additive: false,
  });
  await point(100);
  const before = await evaluate("return window.__pickQa.backgrounds;");
  await actions([
    { type: "pointerDown", button: 0 },
    { type: "pointerMove", origin: "pointer", x: 40, y: 0, duration: 200 },
    { type: "pointerUp", button: 0 },
  ]);
  assert.equal(await evaluate("return window.__pickQa.backgrounds;"), before);
  assert.deepEqual(await evaluate("return window.__pickQa.errors;"), []);
  await writeFile(
    resolve(runtime.outputDirectory, "compact-light.png"),
    Buffer.from(
      await runtime.request("GET", `/session/${sessionId}/screenshot`),
      "base64",
    ),
  );
  await writeFile(
    resolve(runtime.outputDirectory, "verification.json"),
    JSON.stringify(
      {
        kind: "isolated-graph-fixture",
        zoomLevels: [1800, 18000],
        modes: ["3d", "2d"],
        themes: ["dark", "light"],
        foremostNode: true,
        paddedHoverAndClick: true,
        additiveSelection: true,
        backgroundClick: true,
        dragDoesNotClick: true,
        resize: true,
        errors: [],
      },
      null,
      2,
    ),
  );
  process.stdout.write("Graph picking regression passed.\n");
} finally {
  if (sessionId)
    await runtime.request("DELETE", `/session/${sessionId}`).catch(() => {});
  await runtime.stop();
  await rm(fixture, { force: true });
}
