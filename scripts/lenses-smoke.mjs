import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { writeFile, rm } from "node:fs/promises";
import { relative, resolve } from "node:path";
import { fileURLToPath, URL } from "node:url";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";
import { prepareQaRuntime } from "./qa-runtime.mjs";

// Exercise the real App with command stubs; native restart has a Rust test.
const runtime = await prepareQaRuntime("lenses");
const root = fileURLToPath(new URL("../", import.meta.url));
const fixture = resolve(runtime.outputDirectory, "lenses-browser.html");
const route = relative(root, fixture);
assert.ok(!route.startsWith(".."), "Keep the harness in the checkout.");
let sessionId;
const harness = `<!doctype html><html lang="en" data-theme="dark"
 data-theme-storage-key="graphite-lens-qa-theme"><head><meta charset="utf-8">
<title>Graphite Lens workflow check</title></head><body><div id="root"></div>
<script type="module">
localStorage.setItem('graphite-lens-qa-theme','dark');
const qa = window.__lensQa = { calls: [], errors: [] };
window.addEventListener('error', e => qa.errors.push(e.message));
window.addEventListener('unhandledrejection', e => qa.errors.push(String(e.reason)));
const storageKey = 'graphite-lenses-test-only';
const read = () => JSON.parse(sessionStorage.getItem(storageKey) || '[]');
window.__TAURI_INTERNALS__ = {
 transformCallback: () => 1, unregisterCallback: () => {},
 async invoke(command, args = {}) {
  qa.calls.push({command,args});
  if(command === 'list_lenses') return read();
  if(command === 'save_lens') {
   const request = args.request;
   const lens = {...request,id:request.id || 'qa-lens',revision:(request.revision || 0)+1,
     saved:true,groups:[],unavailablePaperIds:[],paperTitles:{}};
   sessionStorage.setItem(storageKey,JSON.stringify([...read().filter(l=>l.id!==lens.id),lens]));
   return lens;
  }
  if(command === 'delete_lens') {
   if (qa.failNextDelete) {
    qa.failNextDelete = false;
    throw new Error('Test deletion interruption. Try again.');
   }
   sessionStorage.setItem(storageKey,JSON.stringify(read().filter(l=>l.id!==args.id))); return;
  }
  throw new Error('Unexpected command '+command);
 }
};
const {browserDemoServices} = await import('/src/adapters/demo/BrowserDemoServices.ts');
const {browserWorkspace} = await import('/src/adapters/demo/browserDiagnostics.ts');
const {services} = await import('/src/services/runtime.ts');
const nativeLenses = services.lenses;
const demoSearch = browserDemoServices.search;

Object.assign(services,browserDemoServices);
services.lenses = nativeLenses;
services.search = { async search(query, ...args) {
  if(query === 'qa-search-failure') {
    throw new Error('Test search interruption');
  }
  if(query === 'qa-search-slow') {
    return new Promise(resolve => { qa.finishSlowSearch = async () =>
      resolve(await demoSearch.search('noise', ...args)); });
  }
  return demoSearch.search(query, ...args);
}};
services.workspace = {...browserDemoServices.workspace,
 load:async()=>({...browserWorkspace,snapshot:{...browserWorkspace.snapshot,lenses:read()}})};
const paper = browserWorkspace.snapshot.items[0];
const summary = {id:'qa-history',title:'QA source navigation',turnCount:2,
  createdAt:'2026-01-01T00:00:00Z',updatedAt:'2026-01-01T00:00:00Z'};
const result = {runId:'qa-run',conversationId:summary.id,question:'Fixture question',
  intent:'current-paper-qa',answerMarkdown:'Fixture evidence [S1].',status:'complete',
  providerLabel:'Fixture',modelLabel:'Fixture',contextLabel:'Fixture',
  contextTokens:1,generationTokens:1,citations:[],limitations:[],actions:[],
  toolCalls:[],promptVersions:[],timings:{totalMs:1},sources:[{
    sourceId:'S1',kind:'document-passage',itemId:paper.id,itemTitle:paper.title,
    documentId:paper.attachments[0]?.documentId ?? 'qa-document',pageStart:1,
    excerpt:'Fixture evidence.',provider:'Fixture',retrievalModes:[],
    eligibleForScientificClaim:true}]};
services.agent = {...browserDemoServices.agent,
  conversations:async()=>[summary],
  conversation:async()=>({summary,turns:[],runs:[result]})};
const {createElement:h} = await import('react');
const {createRoot} = await import('react-dom/client');
const {App} = await import('/src/app/App.tsx');
const {ThemeProvider} = await import('/src/design-system/ThemeProvider.tsx');
await import('@fontsource-variable/inter/wght.css');
await import('/src/design-system/tokens.css');
await import('/src/design-system/global.css');
await import('/src/features/library/library.css');
await import('/src/features/search/search.css');
await import('/src/features/reader/reader.css');
await import('/src/features/agent/agent.css');
createRoot(document.getElementById('root')).render(h(ThemeProvider,{},h(App)));
</script></body></html>`;

async function evaluate(script, args = []) {
  return runtime.request("POST", `/session/${sessionId}/execute/sync`, {
    script,
    args,
  });
}
async function waitFor(script) {
  for (let attempt = 0; attempt < 100; attempt++) {
    const value = await evaluate(script);
    if (value) return value;
    await delay(100);
  }
  throw new Error(`Lens UI timed out: ${script}`);
}
async function click(text) {
  const element = await waitFor(
    `return [...document.querySelectorAll('button')].find(b=>b.textContent.trim()===${JSON.stringify(text)} && !b.disabled);`,
  );
  await runtime.request(
    "POST",
    `/session/${sessionId}/element/${element["element-6066-11e4-a52e-4f735466cecf"]}/click`,
    {},
  );
}
async function input(selector, text) {
  await evaluate(
    `const el=document.querySelector(arguments[0]);
    const proto=el instanceof HTMLTextAreaElement?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto,'value').set.call(el,arguments[1]);
    el.dispatchEvent(new Event('input',{bubbles:true}));`,
    [selector, text],
  );
}
async function capture(name) {
  await delay(250);
  await writeFile(
    resolve(runtime.outputDirectory, `${name}.png`),
    Buffer.from(
      await runtime.request("GET", `/session/${sessionId}/screenshot`),
      "base64",
    ),
  );
}

try {
  await writeFile(fixture, harness);
  ({ sessionId } = await runtime.createFirefoxSession());
  await runtime.request("POST", `/session/${sessionId}/window/rect`, {
    width: 1440,
    height: 1000,
  });
  await runtime.request("POST", `/session/${sessionId}/url`, {
    url: `http://127.0.0.1:1420/${route}`,
  });
  await waitFor("return document.querySelector('.source-trigger');");
  await evaluate("document.querySelector('.source-trigger').click();");
  await waitFor("return document.querySelector('.source-switch button');");
  assert.ok(
    await evaluate(`
    const switcher=document.querySelector('.source-switch button');
    const preparation=document.querySelector('.library-preparation');
    const box=switcher.getBoundingClientRect();
    return switcher.textContent==='Use Zotero' && box.top>=0 &&
      box.bottom<=preparation.getBoundingClientRect().top;
  `),
  );
  await capture("source-top-dark");
  await evaluate("document.documentElement.dataset.theme='light';");
  await capture("source-top-light");
  await evaluate(
    "document.documentElement.dataset.theme='dark'; document.querySelector('.source-popover-close').click();",
  );
  await click("Save selection as Lens");
  await input(".lens-editor input", "Network transfer");
  await input(".lens-editor textarea", "Which methods generalize?");
  await capture("create-desktop");
  await click("Save Lens");
  await waitFor(
    "return !document.querySelector('dialog[open]') && document.body.textContent.includes('Network transfer (0)');",
  );
  await runtime.request("POST", `/session/${sessionId}/refresh`, {});
  await click("Network transfer (0)");
  await waitFor("return document.querySelector('.active-lens-pill');");
  assert.equal(
    await evaluate(
      "return document.querySelector('.graph-empty-state strong').textContent",
    ),
    "This Lens is empty",
  );
  await capture("reopened-desktop");
  await click("All papers60");
  await click("Search libraryK");
  const selectedTitles = [];
  for (const query of ["transformer", "noise", "inversion"]) {
    await input(".search-input-row input", query);
    await delay(350);
    const checkbox = await waitFor(
      "return [...document.querySelectorAll('.result-checkbox')].find(e=>!e.checked);",
    );
    selectedTitles.push(
      await evaluate(
        "return arguments[0].closest('.search-result').querySelector('strong').textContent;",
        [checkbox],
      ),
    );
    await runtime.request(
      "POST",
      `/session/${sessionId}/element/${checkbox["element-6066-11e4-a52e-4f735466cecf"]}/click`,
      {},
    );
    assert.ok(
      await evaluate(
        "return Boolean(document.querySelector('.search-surface'));",
      ),
    );
    assert.equal(
      await evaluate("return document.activeElement.type;"),
      "checkbox",
    );
  }
  await input(".search-input-row input", "zzzznomatchselection");
  await waitFor("return document.querySelector('.no-results');");
  await click("Review selection (3)");
  await waitFor(
    "return document.querySelector('.selection-papers') && !document.querySelector('.search-backdrop');",
  );
  assert.deepEqual(
    await evaluate(
      "return [...document.querySelectorAll('.selection-paper-title strong')].map(e=>e.textContent);",
    ),
    selectedTitles,
  );
  await click("Find papers");
  await input(".search-input-row input", "inversion");
  await waitFor("return document.querySelector('.search-result-open');");
  await click("Review selection (3)");
  await waitFor("return !document.querySelector('.search-backdrop');");
  assert.equal(
    await evaluate(
      "return document.querySelectorAll('.selection-panel').length;",
    ),
    1,
  );
  assert.deepEqual(
    await evaluate(
      "return [...document.querySelectorAll('.selection-paper-title strong')].map(e=>e.textContent);",
    ),
    selectedTitles,
  );
  await capture("selection-desktop");
  await evaluate("document.querySelector('.selection-remove').click();");
  await click("Undo");
  await click("Clear selection");
  await click("Undo");
  assert.equal(
    await evaluate(
      "return document.querySelectorAll('.selection-papers li').length;",
    ),
    3,
  );
  await evaluate("document.querySelector('.selection-remove').click();");
  await evaluate("document.querySelector('.selection-remove').click();");
  await evaluate(
    "document.querySelector('[aria-label=\"Close selection\"]').click();",
  );
  await click("Edit");
  await click("Add selected papers (1)");
  await input(".lens-editor input", "Transfer evidence");
  await capture("edit-desktop");
  await click("Save Lens");
  await waitFor(
    "return !document.querySelector('dialog[open]') && document.body.textContent.includes('Transfer evidence (1)');",
  );
  const savedBeforeDraft = await evaluate(
    "return JSON.parse(sessionStorage.getItem('graphite-lenses-test-only'))[0];",
  );
  await click("Edit");
  await input(".lens-editor input", "Draft name survives search");
  await input(".lens-editor textarea", "Keep this research question.");
  await click("Remove");
  await click("Add papers");
  await waitFor("return document.activeElement.id==='lens-paper-query';");
  const draftTitles = [];
  for (const query of ["transformer", "noise"]) {
    await input("#lens-paper-query", query);
    await waitFor(
      "return document.querySelector('.lens-picker-list button:not(:disabled)');",
    );
    draftTitles.push(
      await evaluate(
        "const b=document.querySelector('.lens-picker-list button:not(:disabled)'); return b.closest('li').querySelector('strong').textContent;",
      ),
    );
    await evaluate(
      "document.querySelector('.lens-picker-list button:not(:disabled)').click();",
    );
    await waitFor(
      "return document.querySelector('.lens-picker-list .is-included button:disabled');",
    );
  }
  await input("#lens-paper-query", "qa-search-slow");
  await waitFor("return Boolean(window.__lensQa.finishSlowSearch);");
  await input("#lens-paper-query", "zzzznoresult");
  await waitFor(
    "return document.querySelector('.lens-picker-matches')?.textContent.includes('No papers match');",
  );
  await evaluate("window.__lensQa.finishSlowSearch();");
  await delay(250);
  assert.equal(
    await evaluate(
      "return document.querySelectorAll('.lens-picker-list li').length;",
    ),
    0,
  );
  await input("#lens-paper-query", "qa-search-failure");
  await waitFor(
    "return document.querySelector('.lens-picker-results [role=alert]');",
  );
  await click("Retry search");
  await waitFor(
    "return document.querySelector('.lens-picker-results [role=alert]');",
  );
  await input("#lens-paper-query", "learning");
  await waitFor("return document.querySelector('.lens-picker-pages');");
  await click("Next results");
  await waitFor(
    "return document.querySelector('.lens-picker-matches')?.textContent.startsWith('21-');",
  );
  await click("Previous results");
  await waitFor(
    "return document.querySelector('.lens-picker-matches')?.textContent.startsWith('1-');",
  );
  await capture("picker-desktop");
  const queryInput = await evaluate(
    "return document.querySelector('#lens-paper-query');",
  );
  const elementKey = "element-6066-11e4-a52e-4f735466cecf";
  await runtime.request(
    "POST",
    `/session/${sessionId}/element/${queryInput[elementKey]}/value`,
    { text: "\uE007" },
  );
  assert.ok(
    await evaluate(
      "return Boolean(document.querySelector('.lens-paper-picker'));",
    ),
  );
  await click("Done adding");
  await waitFor("return document.activeElement.textContent==='Add papers';");
  assert.deepEqual(
    await evaluate(
      "return {name:document.querySelector('.lens-editor input').value,question:document.querySelector('.lens-editor textarea').value,titles:[...document.querySelectorAll('.lens-members li > span')].map(e=>e.firstChild.textContent),changes:document.querySelector('.lens-draft-changes').textContent};",
    ),
    {
      name: "Draft name survives search",
      question: "Keep this research question.",
      titles: draftTitles,
      changes: "2 added / 1 removed",
    },
  );
  await click("Add papers");
  await waitFor("return document.activeElement.id==='lens-paper-query';");
  assert.equal(
    await evaluate("return document.querySelector('#lens-paper-query').value;"),
    "learning",
  );
  const pickerInput = await evaluate(
    "return document.querySelector('#lens-paper-query');",
  );
  await runtime.request(
    "POST",
    `/session/${sessionId}/element/${pickerInput[elementKey]}/value`,
    { text: "\uE00C" },
  );
  await waitFor(
    "return !document.querySelector('.lens-paper-picker') && document.querySelector('.lens-editor input');",
  );
  await click("Cancel");
  assert.deepEqual(
    await evaluate(
      "return JSON.parse(sessionStorage.getItem('graphite-lenses-test-only'))[0];",
    ),
    savedBeforeDraft,
  );
  await click("Edit");
  await click("Add papers");
  await input("#lens-paper-query", selectedTitles[2]);
  await waitFor(
    "return document.querySelector('.lens-picker-list .is-included button')?.textContent.includes('Already included');",
  );
  await click("Done adding");
  await click("Remove");
  await click("Add papers");
  await input("#lens-paper-query", "transformer");
  await waitFor(
    "return document.querySelector('.lens-picker-list button:not(:disabled)');",
  );
  await evaluate(
    "document.querySelector('.lens-picker-list button:not(:disabled)').click();",
  );
  await click("Done adding");
  await input(
    ".lens-editor textarea",
    "Question saved after in-editor search.",
  );
  await click("Save Lens");
  await waitFor("return !document.querySelector('dialog[open]');");
  const savedAfterDraft = await evaluate(
    "return JSON.parse(sessionStorage.getItem('graphite-lenses-test-only'))[0];",
  );
  assert.equal(
    savedAfterDraft.description,
    "Question saved after in-editor search.",
  );
  assert.equal(savedAfterDraft.paperIds.length, 1);
  assert.notEqual(savedAfterDraft.paperIds[0], savedBeforeDraft.paperIds[0]);
  assert.equal(savedAfterDraft.revision, savedBeforeDraft.revision + 1);
  assert.equal(
    await evaluate(
      "return document.querySelector('.selection-readout').textContent;",
    ),
    "1 selected",
  );
  await click("Ask about this Lens");
  await waitFor("return document.body.textContent.includes('This Lens only');");
  await capture("ask-desktop");
  await click("Edit");
  await runtime.request("POST", `/session/${sessionId}/window/rect`, {
    width: 1024,
    height: 768,
  });
  await capture("edit-compact");
  await click("Remove");
  await click("Save Lens");
  await waitFor(
    "return !document.querySelector('dialog[open]') && document.body.textContent.includes('Transfer evidence (0)');",
  );
  await click("Edit");
  await input(".lens-editor input", "");
  await click("Delete Lens");
  assert.ok(
    await evaluate(`
    const confirm = document.querySelector('.lens-delete-confirm');
    const bounds = confirm.getBoundingClientRect();
    return bounds.top >= 0 && bounds.bottom <= innerHeight &&
      !document.querySelector('.lens-editor input') &&
      document.activeElement.textContent === 'Keep Lens';
  `),
  );
  await click("Keep Lens");
  assert.equal(
    await evaluate(
      "return document.querySelectorAll('.lens-delete-confirm').length",
    ),
    0,
  );
  assert.equal(
    await evaluate("return document.activeElement.textContent;"),
    "Delete Lens",
  );
  await click("Delete Lens");
  await capture("delete-confirmation-compact");
  await evaluate("document.documentElement.dataset.theme='light';");
  await capture("delete-confirmation-light");
  await evaluate("document.documentElement.dataset.theme='dark';");
  await evaluate("window.__lensQa.failNextDelete = true;");
  await click("Confirm delete Lens");
  await waitFor(
    "return document.querySelector('.lens-delete-confirm [role=alert]');",
  );
  assert.equal(
    await evaluate(
      "return JSON.parse(sessionStorage.getItem('graphite-lenses-test-only')).length;",
    ),
    1,
  );
  await click("Confirm delete Lens");
  await waitFor(
    "return !document.querySelector('dialog[open]') && !document.body.textContent.includes('Transfer evidence (0)');",
  );
  await click("History");
  await evaluate(
    "document.querySelector('.agent-history-row > button').click();",
  );
  await waitFor(
    "return document.querySelector('.agent-answer-text')?.textContent.includes('Fixture evidence');",
  );
  await click("[S1]");
  for (const action of ["Show in Graph", "Open in Reader"]) {
    await click(action);
    await waitFor(
      "return !document.querySelector('.agent-workspace') && !document.querySelector('.agent-bar') && document.querySelector('.agent-launcher');",
    );
    await evaluate("document.querySelector('.agent-launcher').click();");
    await waitFor(
      "return document.querySelector('.agent-source-detail')?.textContent.includes('Fixture evidence');",
    );
  }
  await runtime.request("POST", `/session/${sessionId}/refresh`, {});
  await waitFor("return document.querySelector('.app-shell');");
  assert.equal(
    await evaluate("return document.querySelectorAll('.lens-entry').length;"),
    0,
  );
  const errors = await evaluate("return window.__lensQa.errors");
  assert.deepEqual(errors, []);
  await writeFile(
    resolve(runtime.outputDirectory, "verification.json"),
    JSON.stringify(
      {
        kind: "browser-command-stubs",
        create: true,
        sourceSwitchAtTop: true,
        inEditorPaperSearch: true,
        draftNameQuestionAndRemovalsPreserved: true,
        draftCancelPreservesSavedLens: true,
        searchPagingAndStaleResponses: true,
        searchErrorKeepsDraft: true,
        pickerEnterDoesNotSaveAndEscapeReturns: true,
        reload: true,
        rename: true,
        addMember: true,
        removeMember: true,
        deleteConfirmation: true,
        deleteConfirmationVisibleWithoutScrolling: true,
        deleteWithInvalidDraftName: true,
        deleteFailureAllowsRetry: true,
        deletionSurvivesReload: true,
        lensOnlyControl: true,
        sourceNavigationCollapsesAgentAndRetainsAnswer: true,
        errors,
        nativeRestartVerifiedSeparately:
          "database::tests::saved_lenses_survive_restart_and_protect_membership",
      },
      null,
      2,
    ),
  );
  process.stdout.write(
    "Lens UI workflow passed. Native persistence is tested separately.\n",
  );
} catch (error) {
  process.stderr.write(`${error.stack ?? error}\n`);
  throw error;
} finally {
  await rm(fixture, { force: true });
  if (sessionId)
    await runtime.request("DELETE", `/session/${sessionId}`).catch(() => {});
  await runtime.stop();
}
