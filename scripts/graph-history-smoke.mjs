import assert from "node:assert/strict";
import { writeFile, rm } from "node:fs/promises";
import { resolve, relative } from "node:path";
import process from "node:process";
import { fileURLToPath, URL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { prepareQaRuntime } from "./qa-runtime.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const runtime = await prepareQaRuntime("graph-history");
const page = resolve(runtime.outputDirectory, "history.html");
let sessionId;
await writeFile(
  page,
  `<!doctype html><html><body style="margin:0"><div id="root"></div>
<script type="module">
import {createElement as h, useEffect, useRef} from 'react';
import {createRoot} from 'react-dom/client';
import {useWorkspace} from '/src/app/useWorkspace.ts';
import {GraphCanvas} from '/src/features/graph/GraphCanvas.tsx';
import '/src/design-system/tokens.css';
import '/src/design-system/global.css';
function Harness(){
 const w=useWorkspace();
 const focusSerial=useRef(0);
 useEffect(()=>{window.qa=w;});
 if(!w.data) return null;
 return h('div',{style:{height:'100vh',width:'100vw'}},h(GraphCanvas,{
 graph:w.data.graph,topics:w.data.snapshot.topics,mode:w.graphMode,
 grouping:w.graphGrouping,restoreSerial:w.restoreSerial,
 selectedIds:w.selection.paperIds,highlightedIds:w.highlightedPaperIds,
 layers:w.layers,theme:'dark',controlsRef:w.graphControls,
 sessionRef:w.graphSession,focusSerialRef:focusSerial,focusRequest:w.focusRequest,
 onSelectNode:w.selectGraphNode,onSelectEdge:w.inspectRelationship,
 onHoverNode:()=>{},onClearSelection:w.clearSelection,onFailure:()=>{throw Error('renderer failed')}
 }));
}
createRoot(document.getElementById('root')).render(h(Harness));
</script></body></html>`,
);
const evaluate = (script) =>
  runtime.request("POST", `/session/${sessionId}/execute/sync`, {
    script,
    args: [],
  });
async function waitFor(script) {
  for (let n = 0; n < 100; n++) {
    if (await evaluate(script)) return;
    await delay(100);
  }
  throw Error(`Timed out: ${script}`);
}
async function camera() {
  return evaluate(
    "qa.graphControls.current.capture(); return qa.graphSession.current.cameras[qa.graphMode];",
  );
}
try {
  ({ sessionId } = await runtime.createFirefoxSession());
  await runtime.request("POST", `/session/${sessionId}/window/rect`, {
    width: 1440,
    height: 900,
  });
  await runtime.request("POST", `/session/${sessionId}/url`, {
    url: `${runtime.appUrl}/${relative(root, page)}`,
  });
  await waitFor("return window.qa?.data && qa.graphControls.current;");
  await delay(2500);
  for (const mode of ["3d", "2d"]) {
    await evaluate(`qa.setGraphMode('${mode}');`);
    await delay(800);
    await evaluate("qa.clearSelection();");
    await delay(100);
    const before = await camera();
    await evaluate("qa.selectPaper(qa.data.snapshot.items[0].id);");
    await delay(100);
    await evaluate("qa.selectPaper(qa.data.snapshot.items[1].id,true);");
    await delay(100);
    assert.equal(await evaluate("return qa.selectedPapers.length;"), 2);
    await evaluate("qa.goBack();");
    await delay(500);
    assert.equal(await evaluate("return qa.selectedPapers.length;"), 1);
    assert.deepEqual(await camera(), before);
    await evaluate("qa.selectPaper(qa.data.snapshot.items[1].id,true);");
    await delay(100);
    const selectedCamera = await camera();
    // A real pointer click in empty canvas space must invoke the clear action.
    const rect = await evaluate(
      'const r=document.querySelector("canvas").getBoundingClientRect();return {x:Math.round(r.left+12),y:Math.round(r.top+12)};',
    );
    await runtime.request("POST", `/session/${sessionId}/actions`, {
      actions: [
        {
          type: "pointer",
          id: "mouse",
          parameters: { pointerType: "mouse" },
          actions: [
            { type: "pointerMove", duration: 0, origin: "viewport", ...rect },
            { type: "pointerDown", button: 0 },
            { type: "pointerUp", button: 0 },
          ],
        },
      ],
    });
    await waitFor("return qa.selectedPapers.length===0;");
    assert.equal(await evaluate("return qa.activePaper;"), null);
    assert.deepEqual(await camera(), selectedCamera);
    const depth = await evaluate("return qa.historyDepth;");
    await evaluate("qa.clearSelection();");
    await delay(100);
    assert.equal(await evaluate("return qa.historyDepth;"), depth);
    await evaluate("qa.goBack();");
    await delay(500);
    assert.equal(await evaluate("return qa.selectedPapers.length;"), 2);
    assert.deepEqual(await camera(), selectedCamera);
    await evaluate("qa.clearSelection();");
    await delay(100);
    await evaluate("qa.undoSelection();");
    await delay(100);
    assert.equal(await evaluate("return qa.selectedPapers.length;"), 2);
    assert.equal(await evaluate("return qa.activePaper;"), null);
    assert.deepEqual(await camera(), selectedCamera);
    // Background dragging must preserve selection.
    await runtime.request("POST", `/session/${sessionId}/actions`, {
      actions: [
        {
          type: "pointer",
          id: "mouse",
          parameters: { pointerType: "mouse" },
          actions: [
            { type: "pointerMove", duration: 0, origin: "viewport", ...rect },
            { type: "pointerDown", button: 0 },
            {
              type: "pointerMove",
              duration: 300,
              origin: "viewport",
              x: rect.x + 60,
              y: rect.y + 40,
            },
            { type: "pointerUp", button: 0 },
          ],
        },
      ],
    });
    await delay(200);
    assert.equal(await evaluate("return qa.selectedPapers.length;"), 2);
    process.stdout.write(
      `History, clear, Undo, camera and drag passed in ${mode}.\n`,
    );
  }
  for (const mode of ["3d", "2d"]) {
    await evaluate(`qa.setGraphMode('${mode}');`);
    await delay(800);
    let previousSerial = 0;
    for (let attempt = 0; attempt < 3; attempt++) {
      await evaluate("qa.clearSelection(); qa.graphControls.current.fit();");
      await delay(800);
      const before = await camera();
      const depth = await evaluate("return qa.historyDepth;");
      await evaluate(
        "qa.findPaper(qa.data.snapshot.items[0].id, {select:true});",
      );
      await waitFor(
        "return qa.selectedPapers.length===1 && qa.activePaper?.id===qa.selectedPapers[0].id;",
      );
      const serial = await evaluate("return qa.focusRequest.serial;");
      assert.ok(serial > previousSerial);
      previousSerial = serial;
      await delay(900);
      assert.notDeepEqual(await camera(), before);
      assert.equal(await evaluate("return qa.historyDepth;"), depth + 1);
      await evaluate("qa.goBack();");
      await delay(500);
      assert.equal(await evaluate("return qa.selectedPapers.length;"), 0);
      assert.deepEqual(await camera(), before);
    }
    process.stdout.write(
      `Search focus after clear and Back passed in ${mode}.\n`,
    );
  }
  await runtime.request("POST", `/session/${sessionId}/url`, {
    url: runtime.appUrl,
  });
  await waitFor('return document.querySelector(".search-trigger");');
  await evaluate('document.querySelector(".search-trigger").click();');
  await waitFor('return document.querySelector(".search-input-row input");');
  await evaluate(`const input=document.querySelector('.search-input-row input');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'earthquake');
    input.dispatchEvent(new Event('input',{bubbles:true}));`);
  await waitFor(
    'return document.querySelectorAll(".result-checkbox").length>=2;',
  );
  await evaluate('document.querySelectorAll(".result-checkbox")[0].click();');
  await delay(100);
  await evaluate('document.querySelectorAll(".result-checkbox")[1].click();');
  await delay(100);
  await evaluate(
    `document.querySelector('[aria-label="Close search"]').click();`,
  );
  await evaluate('document.getElementById("selection-trigger").click();');
  await waitFor('return document.querySelector(".selection-paper-title");');
  await evaluate('document.querySelector(".selection-paper-title").click();');
  await waitFor('return document.querySelector(".paper-inspector");');
  await evaluate('document.getElementById("selection-trigger").click();');
  await waitFor('return document.querySelector(".selection-edit-tools");');
  await evaluate(
    'document.querySelector(".selection-edit-tools button").click();',
  );
  await waitFor(
    'return !document.querySelector(".selection-edit-tools") && !document.querySelector(".paper-inspector");',
  );
  assert.match(
    await evaluate(
      'return document.getElementById("selection-trigger").textContent;',
    ),
    /0 selected/,
  );
  await evaluate('document.getElementById("selection-trigger").click();');
  await waitFor('return document.querySelector(".selection-edit-tools");');
  await evaluate(
    'document.querySelectorAll(".selection-edit-tools button")[1].click();',
  );
  await delay(100);
  assert.match(
    await evaluate(
      'return document.getElementById("selection-trigger").textContent;',
    ),
    /2 selected/,
  );
  assert.equal(
    await evaluate('return !!document.querySelector(".paper-inspector");'),
    false,
  );
  process.stdout.write(
    "App selection panel closes on Clear and Undo restores papers without focus.\n",
  );
  await evaluate('document.querySelector(".search-trigger").click();');
  await waitFor('return document.querySelector(".search-input-row input");');
  await evaluate(`const input=document.querySelector('.search-input-row input');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(input,'earthquake');
    input.dispatchEvent(new Event('input',{bubbles:true}));`);
  await waitFor('return document.querySelector(".search-result-open");');
  await evaluate('document.querySelector(".search-result-open").click();');
  await waitFor(
    'return document.querySelector(".paper-inspector") && document.getElementById("selection-trigger").textContent.includes("1 selected");',
  );
  process.stdout.write(
    "Search title replaces the checkbox selection and opens the paper.\n",
  );
} finally {
  if (sessionId) await runtime.request("DELETE", `/session/${sessionId}`);
  await runtime.stop();
  await rm(page, { force: true });
}
