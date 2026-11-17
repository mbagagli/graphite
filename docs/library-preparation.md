# Prepare a library for AI

Graphite needs document passages before it can answer questions about papers.
A graph of titles and authors is useful for browsing, but it does not mean
that the documents are ready for retrieval-augmented generation (RAG).
RAG finds relevant passages in your library and gives them to an answer
model, with citations that you can inspect.

Use **Data Source > Prepare library for AI** for either the demo or your
selected Zotero library. Preparation runs on this computer and keeps its
results in Graphite's own storage. It does not edit Zotero or original PDFs.
The desktop app is required; the browser preview supports metadata only.

## Prepare the demo

1. Open **Data Source** and choose **Use demo data**. You can immediately
   explore the 60 real papers in the root collection and its theme-based
   child collections. Shared-topic connections are an optional graph overlay.
2. Choose **Prepare library for AI** to open the setup choices. Review the
   approximately 412 MB size, then choose **Download and prepare** to confirm
   downloading the demo ZIP from
   [Zenodo record 24282715](https://doi.org/10.5281/zenodo.24282715).
3. If you already have the demo ZIP, choose **Already downloaded?** in the
   same setup flow and select the ZIP. Do not unpack it. No internet or
   Zotero account is required for this import.
4. Follow the progress beneath the preparation button. Graphite verifies
   the PDFs, prepares searchable text, finds citation connections, and
   indexes passages for semantic search. Verified work is reused.
5. If you see **Embedding model needed**, install the search model using
   the existing **Install embedding model** control in Data Source, then choose
   **Resume preparation**. Preparation does not download models itself.

The ZIP contains original papers, attribution and checksums. Models are
separate downloads. The demo stays separate from your main library, and
switching back to Zotero restores that library's view.

## Prepare your Zotero library

1. In Zotero Desktop, open **Settings > Advanced** and enable
   **Allow other applications on this computer to communicate with Zotero**
   ([official setup](https://www.zotero.org/support/dev/web_api/v3/local_api)).
   Keep Zotero running and make the papers' PDFs available locally.
2. In Graphite, open **Data Source**, choose **Use Zotero**, and select the
   library you want to prepare. The action covers the whole selected library,
   not only the collection currently visible in the graph.
3. Choose **Prepare library for AI**. Graphite synchronizes metadata and
   available text, then extracts selectable text from readable local PDFs
   that still need it. Existing usable or manually selected extractions
   are retained. No papers are downloaded from publishers on your behalf.
4. Install the embedding model separately if prompted, then resume. The
   remaining indexing batches run automatically; you do not need to press
   a next-batch button repeatedly.
5. Review the final coverage and any items needing attention. Available
   papers remain usable even when others cannot be prepared. Attach missing
   PDFs or finish their download in Zotero, then retry preparation.

A scan without selectable text needs OCR before it can provide passages.
Graphite reports this condition and continues with other papers. Preparation
neither installs OCR software nor starts an optional extraction service.
Cached text can remain usable even when the original PDF is unavailable;
opening its cited page still requires the PDF.

## Install the two kinds of model separately

| Model                  | What it does                                                                            | Where to install it                                    |
| ---------------------- | --------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| Embedding/search model | Turns document passages into a searchable index for semantic retrieval and Find similar | Data Source's existing Install embedding model control |
| Answer model           | Writes a response using the retrieved passages                                          | TYRION's Setup controls                                |

An answer model is not needed to prepare the index. Without an embedding
model, Graphite can acquire papers and prepare text and citation data, then
pauses before semantic indexing. Install the missing model and resume.
Keyword search and reading remain available without an answer model.

TYRION displays a gentle notice beside its composer when a model or indexed
passages are missing. Its action opens the relevant setup control. A notice
can be dismissed without losing your prompt. Downloading a model is always
an explicit action and may require several gigabytes for an answer model.

Keep **Built-in model** in TYRION's Setup to let Graphite run the answer
model for you. If you already run a model in Ollama or LM Studio on this
computer, choose **Advanced: answer model > Another app on this computer**
and start that app's local server. You do not also need Graphite's built-in
answer model. Graphite still searches the prepared library and handles
citations; the embedding/search model remains necessary for semantic
retrieval. Direct LAN, VPS, GPT and Claude cloud connections are unsupported.

## Read progress and recover interruptions

The bar directly beneath **Prepare library for AI** shows the current
stage, with downloaded bytes or processed documents when the total is
known. A moving bar means the stage has no reliable total yet; it is not an
estimated percentage. Citation connections and semantic passages are
separate preparation stages.

You may close the setup popover while work continues. **Cancel** keeps
verified PDFs and completed text/indexing work. **Resume preparation** or
**Retry** processes what remains. An interrupted ZIP download may need to
start again. After quitting Graphite, reopen library setup and resume;
preparation does not restart automatically. Library switching is disabled
while preparation is running.

| Status                  | Meaning and next step                                                                                    |
| ----------------------- | -------------------------------------------------------------------------------------------------------- |
| Embedding model needed  | Text and citations can be prepared; install the search model, then resume semantic indexing              |
| Library prepared        | The available documents have been processed; review coverage and any listed problems                     |
| Ready to ask TYRION     | Indexed passages and an available answer provider are present; check that your chosen papers are covered |
| Items needing attention | Some papers lack usable text or files; available papers can still be used                                |

These statuses describe preparation, not the scientific correctness of an
answer. A small or partially prepared selection may still lack sufficient
evidence. Do not treat a successful download alone as RAG readiness.

## Check one answer

In the prepared demo, select **Deep learning for fast simulation of seismic
waves in complex media**. In TYRION, limit the search context to the selected
paper and ask: **Which neural-network architectures does this paper use?**
Open the cited passages and compare them with the original PDF. If the
question is unsupported, an insufficient-evidence response is appropriate.
Try the same workflow with one prepared paper in your own library.

For the demo contents and redistribution credits, see the
[demo walkthrough](release/DEMO.md). Maintainers can use the
[Zenodo publication instructions](release/ZENODO.md).

## Automatic topics

Topic discovery reuses the semantic paper index after preparation. Fresh
libraries have neutral paper nodes and keep citation and other relationship
controls available. Open Topic Map and choose Prepare library for AI to start
setup; model downloads always require an explicit action.

Topic Map reports analyzed coverage. Broad groups use existing paper embeddings;
labels come from distinctive title/abstract phrases. Hover labels for
representative paper titles. Group By Topic becomes available after a useful
map is found. Ambiguous or unindexed papers remain neutral. Small or homogeneous
libraries may not produce distinct groups.

Maps are saved locally and reused across collections and restarts. Updated
index inputs trigger a background refresh; only compatible assignments are
retained while it runs. Rebuilding/removing the index or changing its model
profile invalidates incompatible colors. Zotero tags and retrieval ranking
are unchanged. These groups describe model similarity, not scientific truth.
