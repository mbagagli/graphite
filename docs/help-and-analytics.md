# Help, About, and publication years

Help and About are beside the Zotero source control in the top bar. The
unused notification control has been removed. About reads the version from
the same package metadata used by the desktop build.

Help provides four tabs: START-UP GUIDE, HOW-TO, RAG-SYSTEM, and TIPS. It
covers the real 60-paper demo, its optional PDF download and offline import,
the read-only Zotero path, local PDF editing copies, explicit model
downloads, loopback providers, citation evidence, and troubleshooting.

In Analytics, each publication-year bar opens a scrollable list. The bar
count and the list derive from the same current loaded records. They do not
reuse cached year metrics after a graph scope change. A loaded projection
can be smaller than the active library; the timeline states both counts and
the current scope. Corpus and citation statistics keep their existing
library scope.

The list uses title order, with record identifier as the deterministic
tie-breaker. Each row shows title, authors, venue, year, and DOI when
available. Selecting a row activates that paper in the graph, highlights its
enabled connections and recenters the camera as a graph click would, while
preserving zoom and the current projection. The paper inspector stays open.
Records without a valid integer publication year appear in No year. Empty views and selections display an explicit empty message.

Year labels remain visible below every bar. When the timeline extends
beyond the panel, a persistent proportional scrollbar scrolls it without
requiring hover. Trackpad scrolling and keyboard navigation also work.
No slider is shown when all years already fit.

Year controls use native buttons. Enter or Space opens the list. Dialogs
use native modal focus containment; Escape closes them and restores focus
to the opener. Help tabs support Left, Right, Home, and End. Changing the
active library or switching between demo and Zotero resets the year dialog.

On macOS, these native dialog controls require Safari/WebKit 15.4 or newer.
The proposed OS metadata floor does not establish compatibility with an
unupdated WebKit installation. Platform acceptance remains subject to the
release verification matrix.

## Verification

Focused tests cover view/count agreement after scope changes, missing and
invalid years, stable title ties, empty selections, long lists, Help tabs,
version consistency, and the retained source control. With `npm run dev`
running, execute `node scripts/release-ui-smoke.mjs` for an isolated Firefox
journey covering keyboard activation, Escape, focus restoration, all four
tabs, paper navigation, and a 1,200-record scroll fixture. The script checks
1440x900 and 1024x700 layouts and saves screenshots and its verification
record under `artifacts/qa/release-ui`.

Select a standalone browser driver with `GRAPHITE_QA_GECKODRIVER` when the
system wrapper cannot shut down cleanly. For a containerized browser,
`TMPDIR` must point to a disposable profile directory visible to both the
driver and browser.

## Real collection image

After the opt-in native demo import test exports `workspace.json` and its
`evidence.json`, run `node scripts/real-demo-preview.mjs <workspace.json>`
against the local preview server. The script checks every DOI against
the public manifest and requires the native PDF-integrity and repeat-sync
evidence. It renders the actual app components with the exported metadata
through an ignored browser harness; research tools are disabled there.

The resulting image and attribution sidecar are
`artifacts/qa/real-demo-preview/real-demo.png` and
`real-demo.provenance.json`. The preview image
explicitly identifies itself as a verified collection metadata preview.
It does not establish installed desktop launch or live-model answer quality.
Replaying it does not replace the separately verified public release image.

## Recorded answer and citation replay

To exercise the actual agent and Reader UI with saved native model output:

```sh
node scripts/real-demo-preview.mjs <workspace-indexed.json> \
  --rag-replay <verified-pdf-directory>
```

The workspace's directory must also contain `evidence.json`,
`rag-supported.json`, and `rag-unsupported.json` from the native demo test.
The script verifies each cited document against the manifest PDF checksum
before launching the isolated browser. Only those two recorded questions
can be replayed; no model is executed in the browser.

The check submits the saved supported question through the normal composer,
compares the rendered answer exactly, activates every inline citation with
the keyboard, and verifies the source title and passage. It then checks
Show in Graph and Open in Reader against the correct paper, document, PDF
page count, and available page coordinate. The saved unsupported response
must render its exact insufficient-evidence text with no citation buttons.

For the verified native run, both cited passages lack page coordinates.
The UI displays `page unavailable` and opens the correct 23-page PDF at
page 1. This verifies the disclosed fallback, not passage-page localization.
The successful replay and screenshots are recorded in
`artifacts/qa/real-demo-citations/verification.json`. This evidence covers
browser UI replay; it is neither a fresh model run nor a packaged native
citation interaction check. The public website image is not modified.

A separate final Linux package check opened the saved native conversations
from History. Both S1 and S2 displayed the same unavailable-page label
and opened the correct PDF at page 1 of 23. The unsupported answer
displayed the exact insufficiency response with no inline citations.
This used an isolated public fixture with networking disabled and did
not generate another answer. See `artifacts/qa/native-citations/`.
