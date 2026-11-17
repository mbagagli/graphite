import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { readFile, rm, writeFile } from "node:fs/promises";
import { relative, resolve } from "node:path";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath, URL } from "node:url";
import { prepareQaRuntime } from "./qa-runtime.mjs";

// This checks real UI/coordinator code with public metadata and command stubs.
// Native ZIP validation, PDF extraction and model execution are separate checks.
const runtime = await prepareQaRuntime("library-preparation");
const root = fileURLToPath(new URL("../", import.meta.url));
const fixture = resolve(runtime.outputDirectory, "preparation-browser.html");
const route = relative(root, fixture);
assert.ok(
  !route.startsWith(".."),
  "Keep the temporary harness in the checkout.",
);
const archive = JSON.parse(
  await readFile(new URL("../demo/distribution.json", import.meta.url), "utf8"),
);
let sessionId;

async function evaluate(script, args = []) {
  return runtime.request("POST", `/session/${sessionId}/execute/sync`, {
    script,
    args,
  });
}

async function waitFor(script) {
  for (let attempt = 0; attempt < 100; attempt++) {
    const result = await evaluate(script);
    if (result) return result;
    await delay(75);
  }
  throw new Error(`Preparation UI condition timed out: ${script}`);
}

async function click(selector) {
  const element = await waitFor(
    `return document.querySelector(${JSON.stringify(selector)});`,
  );
  await runtime.request(
    "POST",
    `/session/${sessionId}/element/${element["element-6066-11e4-a52e-4f735466cecf"]}/click`,
    {},
  );
}

async function clickButton(text) {
  const element = await waitFor(`return [...document.querySelectorAll('button')]
    .find(button => button.textContent.includes(${JSON.stringify(text)}));`);
  await runtime.request(
    "POST",
    `/session/${sessionId}/element/${element["element-6066-11e4-a52e-4f735466cecf"]}/click`,
    {},
  );
}

async function key(value) {
  await runtime.request("POST", `/session/${sessionId}/actions`, {
    actions: [
      {
        type: "key",
        id: "preparation-keyboard",
        actions: [
          { type: "keyDown", value },
          { type: "keyUp", value },
        ],
      },
    ],
  });
}

async function capture(name) {
  await delay(150);
  const png = await runtime.request("GET", `/session/${sessionId}/screenshot`);
  await writeFile(
    resolve(runtime.outputDirectory, `${name}.png`),
    Buffer.from(png, "base64"),
  );
}

async function bounds() {
  const state = await evaluate(`
    const popup = document.querySelector('.source-popover');
    const control = document.querySelector('.library-preparation');
    const box = popup.getBoundingClientRect();
    return {width:innerWidth,height:innerHeight,left:box.left,right:box.right,
      top:box.top,bottom:box.bottom,
      noOverflow:control.scrollWidth <= control.clientWidth + 1};
  `);
  assert.ok(
    state.noOverflow &&
      state.left >= 0 &&
      state.top >= 0 &&
      state.right <= state.width + 1 &&
      state.bottom <= state.height + 1,
    JSON.stringify(state),
  );
  return state;
}

async function openScenario(query = "") {
  const url = new URL(route, `${runtime.appUrl}/`);
  url.search = query;
  await runtime.request("POST", `/session/${sessionId}/url`, { url: url.href });
  await waitFor("return window.__preparationQa?.ready;");
  await click(".source-trigger");
  await waitFor(
    "return document.querySelector('.source-popover:popover-open');",
  );
}

async function runViewport(viewport) {
  await runtime.request("POST", `/session/${sessionId}/window/rect`, viewport);
  await openScenario();
  assert.equal(
    await evaluate(
      "return document.querySelectorAll('.library-preparation button').length;",
    ),
    1,
  );
  for (let step = 0; step < 6; step++) {
    await key("\uE004");
    if (
      await evaluate(
        "return document.activeElement.matches('.library-prepare-button');",
      )
    )
      break;
  }
  assert.equal(
    await evaluate(
      "return document.activeElement.matches('.library-prepare-button:focus-visible');",
    ),
    true,
  );
  await key("\uE007");
  await waitFor("return document.querySelector('.library-archive-choice');");
  assert.match(
    await evaluate(
      "return document.querySelector('.library-archive-choice').textContent;",
    ),
    /not published|not configured|unavailable/i,
  );
  await capture(`zip-choice-${viewport.width}`);
  await clickButton("Already downloaded?");
  await waitFor(
    "return window.__preparationQa.progress?.status === 'cancelled';",
  );
  assert.equal(
    await evaluate(
      "return window.__preparationQa.calls.includes('index_semantic_corpus');",
    ),
    false,
  );
  await evaluate("window.__preparationQa.pickerAccepted = true;");
  await clickButton("Already downloaded?");
  await waitFor("return document.querySelector('progress')?.value > 0;");
  assert.equal(
    await evaluate(
      "return document.querySelector('.source-popover footer button').disabled;",
    ),
    true,
  );
  const before = await evaluate(
    "return document.querySelector('progress').value;",
  );
  await key("\uE00C");
  await waitFor("return !document.querySelector('.source-popover');");
  assert.equal(
    await evaluate("return window.__preparationQa.progress.status;"),
    "running",
  );
  assert.equal(
    await evaluate("return document.activeElement.matches('.source-trigger');"),
    true,
  );
  await key("\uE007");
  await waitFor(
    "return document.querySelector('.source-popover:popover-open');",
  );
  assert.equal(
    await evaluate("return document.querySelector('progress').value;"),
    before,
  );
  const measured = await bounds();
  await capture(`progress-reopened-${viewport.width}`);
  await evaluate("window.__preparationQa.release();");
  await waitFor(
    "return window.__preparationQa.progress?.status === 'needs-model';",
  );
  assert.match(
    await evaluate(
      "return document.querySelector('.library-preparation').textContent;",
    ),
    /Embedding model needed/,
  );
  assert.equal(
    await evaluate(
      "return window.__preparationQa.calls.includes('install_semantic_model');",
    ),
    false,
  );
  await capture(`embedding-needed-${viewport.width}`);
  await clickButton("Install embedding model");
  await waitFor("return window.__preparationQa.data.semantic.modelInstalled;");
  await click(".library-prepare-button");
  await waitFor(
    "return window.__preparationQa.progress?.status === 'complete';",
  );
  assert.ok(
    await evaluate(
      "return window.__preparationQa.calls.filter(x => x === 'index_semantic_corpus').length >= 2;",
    ),
  );
  await capture(`prepared-${viewport.width}`);

  await openScenario("mode=zotero&partial=1");
  await click(".library-prepare-button");
  await waitFor(
    "return window.__preparationQa.progress?.status === 'partial';",
  );
  await click(".library-preparation-issues summary");
  assert.equal(
    await evaluate(
      "return document.querySelectorAll('.library-preparation-issues li').length;",
    ),
    1,
  );
  assert.match(
    await evaluate(
      "return document.querySelector('.library-preparation-issues').textContent;",
    ),
    /No usable PDF or text/,
  );
  assert.equal(
    await evaluate(
      "return window.__preparationQa.calls.includes('download_demo_pdfs');",
    ),
    false,
  );
  await bounds();
  await capture(`zotero-partial-${viewport.width}`);

  await openScenario("online=1");
  await click(".library-prepare-button");
  await click(".library-prepare-button");
  await waitFor("return document.querySelector('progress')?.value > 0;");
  assert.match(
    await evaluate(
      "return document.querySelector('.library-preparation-progress').textContent;",
    ),
    /MB/,
  );
  await capture(`download-${viewport.width}`);
  await clickButton("Cancel");
  await waitFor(
    "return window.__preparationQa.progress?.status === 'cancelled';",
  );
  assert.equal(
    await evaluate(
      "return window.__preparationQa.calls.includes('cancel_demo_preparation');",
    ),
    true,
  );

  const isolation = await evaluate(`return {
    hostname:location.hostname,
    stubbed:window.__preparationQa.stubbed,
    outside:performance.getEntriesByType('resource').map(x => x.name)
      .filter(x => new URL(x,location.href).origin !== location.origin),
    unexpected:window.__preparationQa.unexpected,
  };`);
  assert.equal(isolation.hostname, "127.0.0.1");
  assert.equal(isolation.stubbed, true);
  assert.deepEqual(isolation.outside, []);
  assert.deepEqual(isolation.unexpected, []);
  return {
    ...viewport,
    measured,
    keyboardAccess: true,
    pickerCancelled: true,
    progressSurvivesClose: true,
    missingModelResume: true,
    partialDetails: true,
    downloadCancellation: true,
    noImplicitModelDownload: true,
    isolation,
  };
}

const harness = `<!doctype html>
<html lang="en" data-theme="dark" data-theme-storage-key="graphite-preparation-qa-theme">
<head><meta charset="utf-8"><title>Library preparation browser check</title></head>
<body><div id="root"></div><script type="module">
const options = new URLSearchParams(location.search);
const mode = options.get('mode') === 'zotero' ? 'zotero' : 'demo';
localStorage.setItem('graphite-source-mode', mode);
localStorage.setItem('graphite-preparation-qa-theme', 'dark');
const qa = window.__preparationQa = {stubbed:true,ready:false,calls:[],unexpected:[],
  pickerAccepted:false,cached:false,release:null,progress:null,data:null};
const callbacks = new Map();
const listeners = new Map();
let nextCallback = 1;
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener: (_event,id) => listeners.delete(id),
};
window.__TAURI_INTERNALS__ = {
  transformCallback(callback) { const id=nextCallback++;callbacks.set(id,callback);return id; },
  unregisterCallback(id) { callbacks.delete(id); },
  async invoke(command,args={}) {
    qa.calls.push(command);
    if (command === 'plugin:event|listen') {
      listeners.set(args.handler,args);return args.handler;
    }
    if (command === 'plugin:event|unlisten') return;
    if (command === 'get_demo_archive_info') return {
      ...${JSON.stringify(archive)},
      url: options.has('online') ? 'https://zenodo.org/records/1/files/browser-qa-only.zip' : null,
    };
    if (command === 'get_demo_status') return {pdfCount:qa.cached?60:0,totalPdfs:60,totalBytes:411742026,documents:[],busy:false,progress:null};
    if (command === 'cancel_demo_preparation') {qa.release?.(true);return;}
    if (command === 'cancel_semantic_indexing') return;
    if (command === 'download_demo_pdfs' || command === 'import_demo_pdfs') {
      if (command === 'import_demo_pdfs' && !qa.pickerAccepted) return null;
      if (qa.cached) return {};
      const bytes = ${archive.bytes};
      for (const [id,event] of listeners) callbacks.get(id)?.({event:event.event,id,
        payload:{phase:command==='download_demo_pdfs'?'downloading-archive':'Importing verified PDFs',
          completed:30,total:60,bytes:Math.floor(bytes/4),totalBytes:bytes,paperId:null}});
      await new Promise((accept,reject) => {qa.release = (cancelled=false) => {
        qa.release=null;
        if (cancelled) reject(new Error('Browser command stub cancelled'));
        else {qa.cached=true;accept();}
      };});
      return {};
    }
    if (['activate_demo_library','activate_zotero_library','sync_zotero','sync_document_corpus'].includes(command)) return structuredClone(qa.data);
    if (command === 'get_library_preparation_documents') {
      return qa.data.snapshot.items.map((item,index) => ({itemId:item.id,title:item.title,
        documentId:options.has('partial')&&index===0?null:'qa-document-'+index,
        hasUsableText:!options.has('partial')||index!==0,mediaType:'application/pdf'}));
    }
    if (command === 'install_semantic_model') {
      qa.data.semantic.modelInstalled=true;return structuredClone(qa.data);
    }
    if (command === 'index_semantic_corpus') {
      await new Promise(resolve=>setTimeout(resolve,35));
      const s=qa.data.semantic;
      s.indexedPapers=Math.min(s.eligiblePapers,s.indexedPapers+24);
      s.chunkedDocuments=Math.min(s.eligibleDocuments,s.chunkedDocuments+6);
      s.indexedChunks=s.chunkedDocuments;
      return structuredClone(qa.data);
    }
    qa.unexpected.push(command);throw new Error('Unexpected browser command: '+command);
  },
};
const {browserWorkspace}=await import('/src/adapters/demo/browserDiagnostics.ts');
qa.data=structuredClone(browserWorkspace);
qa.data.source.mode=mode;
qa.data.source.health=mode==='demo'?'demo':'ready';
qa.data.source.activeLibraryId=mode==='demo'?qa.data.snapshot.activeLibraryId:'qa-zotero-public-demo';
qa.data.snapshot.activeLibraryId=qa.data.source.activeLibraryId;
qa.data.snapshot.libraries[0].id=qa.data.source.activeLibraryId;
Object.assign(qa.data.corpus,{fullTextCount:options.has('partial')?59:60,lastError:null});
Object.assign(qa.data.citations,{queuedDocuments:0,queuedResolutions:0,lastError:null});
Object.assign(qa.data.semantic,{modelInstalled:mode==='zotero'||options.has('online'),
  eligiblePapers:60,indexedPapers:0,eligibleDocuments:options.has('partial')?59:60,
  chunkedDocuments:0,indexedChunks:0,stalePapers:0,staleChunks:0,lastError:null});
const {createElement:h,useState}=await import('react');
const {createRoot}=await import('react-dom/client');
const {ThemeProvider}=await import('/src/design-system/ThemeProvider.tsx');
const {TopBar}=await import('/src/app/TopBar.tsx');
const {useLibraryPreparation}=await import('/src/app/useLibraryPreparation.ts');
const {services}=await import('/src/services/runtime.ts');
await import('/src/design-system/tokens.css');
await import('/src/design-system/global.css');
await import('/src/features/agent/agent.css');
await import('@fontsource-variable/inter/wght.css');
function Harness() {
  const [data,setData]=useState(qa.data);
  const [panel,setPanel]=useState(null);
  const preparation=useLibraryPreparation(data,setData);
  qa.progress=preparation.progress;qa.ready=true;
  const noop=()=>{};
  return h('div',{className:'app-shell'},h(TopBar,{
    openPanel:panel,onPanelChange:setPanel,view:'graph',selectedCount:0,
    onViewChange:noop,onSearchOpen:noop,source:data.source,corpus:data.corpus,
    extraction:data.extraction,citations:data.citations,semantic:data.semantic,
    libraries:data.snapshot.libraries,syncing:false,citationIndexing:false,
    semanticIndexing:false,syncError:null,onSync:noop,onRebuild:noop,
    onRebuildCorpus:noop,onRebuildCitations:noop,onCancelSemantic:noop,
    onRebuildSemantic:noop,onLibraryChange:noop,onModeChange:noop,preparation,
    onInstallSemanticModel:async()=>setData(await services.semantic.installModel()),
  }),h('main',{style:{gridColumn:'1 / -1',padding:24}},
    h('h1',{style:{fontSize:20}},'Library preparation browser check'),
    h('p',{},'Real 60-paper metadata; simulated native command responses.'),
    h('p',{},'This checks the setup interface, not PDF validation or model inference.'),
    h('ul',{},data.snapshot.items.slice(0,6).map(item=>h('li',{key:item.id},item.title)))));
}
createRoot(document.getElementById('root')).render(h(ThemeProvider,{},h(Harness)));
</script></body></html>`;

try {
  await writeFile(fixture, harness);
  ({ sessionId } = await runtime.createFirefoxSession());
  const viewports = [];
  for (const viewport of [
    { width: 1440, height: 900 },
    { width: 1024, height: 700 },
  ]) {
    viewports.push(await runViewport(viewport));
  }
  await writeFile(
    resolve(runtime.outputDirectory, "verification.json"),
    JSON.stringify(
      {
        kind: "browser-command-stubs",
        actualDemoMetadata: true,
        nativeZipVerified: false,
        pdfExtractionVerified: false,
        modelInferenceVerified: false,
        viewports,
      },
      null,
      2,
    ) + "\n",
  );
} catch (error) {
  if (sessionId) await capture("failure").catch(() => {});
  throw error;
} finally {
  if (sessionId)
    await runtime.request("DELETE", `/session/${sessionId}`).catch(() => {});
  await runtime.stop();
  await rm(fixture, { force: true });
}
process.stdout.write(
  "Library preparation browser checks passed. Native validation is separate.\n",
);
