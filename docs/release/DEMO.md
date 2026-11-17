# Real-paper demonstration

## Run the demo

1. Install Graphite Desktop and open **Data Source > Use demo data**.
2. Explore the 60 published papers immediately. Shared-topic connections
   are visible by default and are labelled separately from citations.
3. Choose **Prepare library for AI**, review the size, then confirm with
   **Download and prepare**. The default option downloads the
   approximately 412 MB ZIP from its configured, version-specific Zenodo
   record. **Already downloaded?** in the same flow accepts a local ZIP
   directly; no manual unpacking is needed.
4. Follow the progress bar beneath the button while Graphite verifies the
   PDFs and prepares document text, citations and semantic passages.
5. If the embedding model is missing, text/citation preparation finishes
   first. Install the search model using the existing Data Source control,
   then choose **Resume preparation**. Model downloads remain separate.
6. For source-grounded answers, install an answer model in **TYRION > Setup**
   or configure a supported local AI application there. TYRION's notice links
   to missing setup. A prepared library alone does not install an answer model.

See the [library preparation guide](../library-preparation.md) for both the
demo and your main Zotero library, including readiness, partial coverage,
missing models, cancellation and a first question.

No Zotero account or running Zotero installation is required for this demo.
Graphite stores it in its own source partition. Switching back to Zotero
restores the previous library; the demo never writes to that library.
The installer includes metadata. PDF download is optional and uses the
published, version-specific ZIP pinned in `demo/distribution.json`.
Offline import accepts the same ZIP. Nothing is uploaded. Cancellation keeps verified PDFs and
completed text/indexing work so preparation can resume. Invalid or changed
PDFs are rejected. An interrupted ZIP download may need to restart.

Browser mode (`npm run dev`) offers the real metadata, shared-topic graph,
search and publication-year navigation. PDF reading, citation extraction,
local indexing and model execution require the desktop app. Browser mode
never supplies invented passages or model answers.

## Collection and evidence

`demo/manifest.json`, version `seismology-ml-v2`, contains 60 distinct papers.
Their PDFs contain 921 pages and total 411,742,026 bytes. Each record includes
bibliographic metadata, source and PDF URLs, license evidence, attribution,
page and byte counts, and a SHA-256 checksum.

Selection follows actual reference lists. `demo/connections.json` records
97 independently reviewed directed bibliography links, with source PDF
checksums and one-based PDF page numbers. This file records selection
evidence; Graphite does not inject these links into its citation
index. The native app extracts references from the actual document text.

Topics include picking, detection, classification, source characterization,
wave simulation, environmental and volcanic seismology, velocity models,
and seismic interpretation. The collection also includes ML-enabling data
papers such as STEAD, INSTANCE and Noddyverse. Including a forecasting or
strain-anomaly paper does not endorse operational earthquake prediction.

Citation coverage depends on prepared text and reference resolution. Some
layouts, wrapped references, prepublication versions and bibliographic errors
remain unresolved. Shared-topic links describe curated subject overlap,
not citations. Use the original PDF to inspect references that automatic
extraction misses; the reviewed selection links are not an extraction-quality
guarantee.

`demo/rights-review.json` ties the review to each exact PDF. One paper uses
CC BY 2.0, two use CC BY 3.0 and 57 use CC BY 4.0. Keep the attribution and
rights files with redistributed copies. The papers retain their own
licenses; Graphite's GPL license does not replace them. Restricted research
datasets mentioned inside articles are not included in the PDF archive.

## Offline archive

Prepare missing PDFs explicitly, then build the archive:

```bash
npm run demo:prepare -- --download
npm run demo:archive -- --zenodo
```

For already acquired PDFs, run `npm run demo:check` without network access.
Both commands accept `--directory DIRECTORY`. Archive creation also accepts
`--output OUTPUT`; choose a new output directory when an older receipt is
already present. The tool refuses to overwrite a different archive.

The ZIP contains `graphite-demo-seismology-ml-v2/` with:

- `pdfs/<paper-id>.pdf`: 60 unchanged original PDFs.
- `manifest.json`: citation, source, license and integrity metadata.
- `RIGHTS.json` and `ATTRIBUTION.txt`: exact-PDF review and required credits.
- `README.txt`: desktop import instructions.
- `SHA256SUMS`: integrity records for every other file.

The archive uses fixed timestamps, sorted paths and fixed permissions. Its
entries are rechecked with ZIP CRC and SHA-256. No models, user databases,
private attachments or application state are included.

The published archive is
[graphite-demo-seismology-ml-v2-r1.zip](https://zenodo.org/records/24282715/files/graphite-demo-seismology-ml-v2-r1.zip):

```text
Bytes: 411918125
SHA-256: 8286328f700d7c3773587a86aed58e980c92d4ad7f61e9e10eb2da7b11f0fa28
```

The published record contains the three files listed in the
[Zenodo publication guide](ZENODO.md). Keep the build receipt locally beside
the bundle; it is not part of the upload or the public source tree.

The package workflow's optional **Build the offline PDF demo archive**
input prepares the same collection as a separate artifact. It is disabled
by default and does not enlarge the DEB or DMG. The owner controls workflow
execution and publication.

## Demonstration walkthrough

1. Start with the shared-topic graph and select an event-classification
   paper. Its related nodes remain visible while unrelated nodes dim.
2. Open Analytics, choose 2011 and inspect its two papers. Select one to
   return to the graph with its node and inspector active. Repeat in 2D
   and 3D; the selected paper remains the focus.
3. After PDF preparation, find Earthquake Transformer or STEAD. Explore
   local citations and open the underlying reference evidence. Compare a
   citation link with a separately labelled shared-topic link.
4. Open a PDF and inspect the cited page. Keyword retrieval works without
   a model; semantic similarity and agent answers require indexing and
   the optional local models.
5. For a grounded-answer demonstration, select the wave-simulation paper
   and ask which network architectures it uses. Check every cited passage
   in the PDF. Ask an unrelated question and confirm that insufficient
   evidence is reported honestly. A successful example does not establish
   general answer quality across the collection.

## Verification and developer tools

The [Zenodo guide](ZENODO.md) describes the archive integrity checks and an
explicit native download/import test. Native integration uses disposable
app data, the public PDFs and the same PDF.js text boundaries as desktop
setup. It requires no Zotero or model download.

The optional developer-only Zotero import is still available through
`npm run demo:import -- --zotero-url http://127.0.0.1:23129`. It requires an
already running, explicitly isolated empty profile on a nondefault port.
The tool verifies all 120 article and attachment records across API pages,
refuses a nonempty library, and never creates an account. Normal users
should use the built-in demo controls above.
