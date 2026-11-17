import { useState, type KeyboardEvent } from "react";

import { ModalDialog } from "../components/ModalDialog";
import { applicationSubtitle } from "../shared/productIdentity";
import { version } from "../../package.json";
import { demoManifest } from "../adapters/demo/demoData";

const helpTabs = ["START-UP GUIDE", "HOW-TO", "RAG-SYSTEM", "TIPS"] as const;

export function HelpDialog({ onClose }: { onClose: () => void }) {
  const [activeTab, setActiveTab] = useState(0);

  function navigateTabs(event: KeyboardEvent<HTMLButtonElement>) {
    let next: number;
    if (event.key === "ArrowRight") next = (activeTab + 1) % helpTabs.length;
    else if (event.key === "ArrowLeft") {
      next = (activeTab + helpTabs.length - 1) % helpTabs.length;
    } else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = helpTabs.length - 1;
    else return;
    event.preventDefault();
    setActiveTab(next);
    document.getElementById(`help-tab-${next}`)?.focus();
  }

  return (
    <ModalDialog
      title="Help"
      titleId="help-title"
      className="help-dialog"
      onClose={onClose}
    >
      <div className="help-tabs" role="tablist" aria-label="Help topics">
        {helpTabs.map((tab, index) => (
          <button
            key={tab}
            type="button"
            role="tab"
            id={`help-tab-${index}`}
            aria-controls={`help-panel-${index}`}
            aria-selected={activeTab === index}
            tabIndex={activeTab === index ? 0 : -1}
            onClick={() => setActiveTab(index)}
            onKeyDown={navigateTabs}
          >
            {tab}
          </button>
        ))}
      </div>
      <div
        className="modal-dialog-body"
        role="tabpanel"
        id={`help-panel-${activeTab}`}
        aria-labelledby={`help-tab-${activeTab}`}
        tabIndex={0}
      >
        {activeTab === 0 && <StartUpGuide />}
        {activeTab === 1 && <HowTo />}
        {activeTab === 2 && <RagSystem />}
        {activeTab === 3 && <Tips />}
      </div>
    </ModalDialog>
  );
}

export function AboutDialog({ onClose }: { onClose: () => void }) {
  return (
    <ModalDialog title="About Graphite" titleId="about-title" onClose={onClose}>
      <div className="modal-dialog-body">
        <dl className="about-details">
          <dt>Version</dt>
          <dd>{version}</dd>
          <dt>Created by:</dt>
          <dd>Matteo Bagagli</dd>
          <dt>Project</dt>
          <dd>Graphite - {applicationSubtitle}</dd>
          <dt>License</dt>
          <dd>GNU General Public License v3.0 only (GPL-3.0-only)</dd>
        </dl>
        <p>Windows is currently unavailable. We are working on support.</p>
        <p>
          A local-first research environment for exploring, reading, searching,
          and questioning a scientific library. Zotero remains the read-only
          bibliographic source of truth.
        </p>
        <h3>Support and diagnostics</h3>
        <p>
          Start with Help and the Zotero source diagnostics. When reporting an
          issue through the project&apos;s distribution channel, include this
          version, your operating system, the failed action, and a reproducible
          example. Remove private paper content, credentials, and local paths.
        </p>
        <p>
          The complete license accompanies the application. Graphite comes
          without warranty; see that license for your rights and obligations.
        </p>
      </div>
    </ModalDialog>
  );
}

function StartUpGuide() {
  return (
    <>
      <h3>Choose a source</h3>
      <p>
        Open the source control beside Help. Switch between Use Zotero and Use
        demo data at the top of the panel. Use demo data opens the real
        seismology collection: {demoManifest.papers.length} published papers on
        machine learning, with their authors, publication years, themes, and
        source links.
      </p>
      <p>
        Choose Prepare library for AI. Graphite downloads the verified demo ZIP
        from Zenodo; Already downloaded? lets you choose the ZIP directly. Text,
        citations and semantic indexing are prepared automatically. The PDFs
        retain their CC BY licenses and attribution. Demo data stays separate
        from your Zotero library; no account or temporary Zotero profile is
        needed.
      </p>
      <h3>Connect your library</h3>
      <p>
        Start Zotero Desktop and enable the Advanced setting that allows local
        applications to communicate with it. In the installed Graphite app,
        choose Use Zotero and Sync now, then select the discovered library. The
        browser preview explores the real metadata; PDF setup and local research
        models require the desktop app.
      </p>
      <h3>Check coverage before asking questions</h3>
      <p>
        Inspect document text and PDF availability in the source control.
        Install the embedding model in library setup, then use Prepare library
        for AI. Without it, text and citations are prepared and indexing waits
        for you to install the model and resume. Open TYRION Setup to install an
        answer model. Library preparation never downloads models. Keyword search
        and reading do not require a generation model.
      </p>
    </>
  );
}

function HowTo() {
  return (
    <>
      <h3>Import and synchronize</h3>
      <p>
        Add records and attach PDFs in Zotero, then choose Sync now in Graphite.
        Graphite reads through Zotero&apos;s Local API and keeps its own derived
        cache; it does not edit Zotero records or PDFs. Prepare library for AI
        also extracts selectable text from readable local PDFs when needed.
        Missing attachments and scans needing OCR are listed; other papers are
        prepared normally. Completed work is kept when you cancel.
      </p>
      <h3>Find and read papers</h3>
      <p>
        The graph opens with all papers in the active library. Collections and
        Lenses show their available members. Choose All papers to return to the
        complete library; larger libraries can take longer to lay out.
      </p>
      <p>
        Use Search library or Control/Command+K. Keyword search is always
        available; Semantic and Hybrid require an installed embedding model and
        indexed content. Select a result to inspect its metadata, then open its
        PDF in Reader. Sources distinguish metadata from document passages.
      </p>
      <p>
        Click a graph paper or just beside it to select it, even when zoomed
        out. Overlapping targets select the paper nearest the viewer. Zoom in or
        rotate the 3D view to reach a paper behind it. Shift/Command-click adds
        to the selection; double-click moves closer.
      </p>
      <h3>Browse publication years</h3>
      <p>
        Open Analytics and select a year in Papers through time. The popup lists
        exactly the papers counted in the current loaded view, sorted by title.
        Graph scope changes affect these year counts; a bounded view is not the
        entire library. No year lists records with missing publication years.
        Drag the scrollbar below the years, or focus it and use the arrow, Home,
        and End keys. It remains visible without hovering. Select a paper to
        return to the graph with that work selected and the camera focused on
        it. Escape closes the popup.
      </p>
      <h3>Explore topic groups</h3>
      <p>
        Prepare the library for AI first: automatic topics reuse the semantic
        index. Until a map is available, paper nodes stay neutral and citations
        remain usable. Topic Map shows analyzed coverage and source-derived
        group labels; Group By Topic arranges those groups without changing
        connections. Hover a label for representative papers. These are local
        similarity groups, not verified scientific classifications. Zotero tags
        remain unchanged. Collections reuse the library-wide map.
      </p>
      <h3>Navigate a PDF</h3>
      <p>
        Use the page arrows, thumbnails, or Page Up and Page Down. To jump to a
        page, enter its number and press Enter. Scroll within a page normally;
        at its edge, a new scrolling gesture turns the page. Spread shows two
        pages together. Control/Command+F searches the current PDF.
      </p>
      <h3>Edit a local PDF copy</h3>
      <p>
        Edit PDF opens a separate working copy. Add highlights, notes, and
        drawings, or edit selectable original text runs. If an embedded font
        cannot represent the new characters, choose Noto Sans in the text
        dialog. Scanned pages and text grouped into artwork need OCR or an
        external editor first.
      </p>
      <p>
        Pages lets you rotate, reorder, insert, and delete pages. Undo and Redo
        apply within the editing session. Save copy keeps the result in
        Graphite; Export PDF saves an edited PDF. The original remains available
        with View original. Search, citations, and assistant evidence continue
        to use that original, even if you change page order in the copy.
      </p>
      <h3>Recover or refresh an index</h3>
      <p>
        Source diagnostics show text, citation, and semantic coverage. Resume
        preparation continues all remaining batches. Rebuild controls regenerate
        Graphite-owned data and preserve Zotero. Keep original attachments in
        Zotero.
      </p>
    </>
  );
}

function RagSystem() {
  return (
    <>
      <h3>From library to evidence</h3>
      <p>
        Graphite keeps metadata and indexed text in its own local database. Your
        library is read from Zotero; optional demo PDFs are verified and
        prepared locally. Local text chunks and embedding vectors support
        Keyword, Semantic, and Hybrid retrieval. Citation relationships describe
        the indexed local corpus, not global scholarly impact.
      </p>
      <h3>Choose who runs the answer model</h3>
      <p>
        Graphite searches your papers and finds relevant passages. An answer
        model uses those passages to write a response. In Setup, keep Built-in
        model selected to let Graphite download and run that model on this
        computer. No other AI app is needed. Lightweight and Balanced profiles
        have different memory needs; downloads require your explicit action.
      </p>
      <p>
        Inference backend offers Auto, CPU, and GPU. Auto can recover on CPU;
        explicit GPU reports a failure instead. GPU needs a compatible build and
        driver; the graphics card rendering the window is separate. Context
        window has four levels, with Medium as the default. More context can fit
        more passages but uses more memory and may be slower. It does not
        guarantee better answers or change Answer detail.
      </p>
      <p>
        Advanced: answer model also offers Another app on this computer. Use
        this only if you already run a model in Ollama, LM Studio, or another
        compatible app. Enable its local server and keep it running. That app
        controls its model, GPU and context size; Graphite still handles paper
        search and citations.
      </p>
      <p>
        OpenAI-compatible describes the connection format, not a connection to
        OpenAI. Direct LAN, VPS, GPT and Claude cloud connections are not
        supported. The optional API key is for your local server only and is
        never saved. Graphite sends the question, selected passages and relevant
        conversation history to that server; review the other app's settings for
        any onward connections.
      </p>
      <h3>Keep a research Lens</h3>
      <p>
        Select papers and choose Save selection as Lens in the sidebar. Give it
        a name and optionally a research question. Saved Lenses survive restart
        and belong to the current library. Edit beside a Lens lets you rename
        it, remove papers, or add the current selection. Delete Lens opens a
        confirmation view: choose Confirm delete Lens to remove it, or Keep Lens
        to return to your draft. Deleting a Lens keeps its papers. Save
        temporary Lens keeps the sources from a TYRION answer.
      </p>
      <p>
        Ask about this Lens opens its question with This Lens only selected.
        This explicit scope accepts 1-64 available papers and excludes earlier
        answers. Missing papers stay recorded, but cannot provide evidence.
      </p>
      <h3>Inspect sources and limitations</h3>
      <p>
        Choose the current paper, selected papers, collection, Lens, or library
        scope before asking. Scientific claims need retrieved document passages;
        metadata alone is not evidence of a method or result. Click a citation
        to inspect its exact source and open Reader where available. Weak or
        absent evidence should lead to a limitation or refusal, not a confident
        unsupported answer. Check the passage yourself.
      </p>
      <p>
        Documents are treated as untrusted source data, not instructions.
        Questions, answers, and source revisions are stored locally in history;
        you can delete conversations or explicitly export them. No library
        content is sent to a Graphite-operated service.
      </p>
    </>
  );
}

function Tips() {
  return (
    <>
      <h3>Improve retrieval quality</h3>
      <p>
        Ask a specific question and select the papers or collection that can
        answer it. Check title, year, identifiers, and text coverage. Try
        Keyword for exact terminology and Hybrid for conceptual queries.
        Similarity is a model-dependent ranking, not a probability or proof of
        agreement.
      </p>
      <h3>Handle PDF issues</h3>
      <p>
        Confirm the attachment opens in Zotero and Reader. Scanned, encrypted,
        malformed, or poorly extracted PDFs may have missing or partial text. Do
        not interpret missing retrieval results as absence of a scientific
        result. Inspect extraction diagnostics and any available text before
        rebuilding the text or semantic index.
      </p>
      <h3>Reconnect and resume</h3>
      <p>
        If Zotero is offline, cached metadata and text remain available. Start
        Zotero and Sync now to refresh them. For interrupted indexing, use the
        Resume preparation button. For model failures, check available memory,
        the selected profile, and Local setup diagnostics; a smaller profile can
        reduce memory needs.
      </p>
      <h3>Use the keyboard</h3>
      <p>
        Tab moves between controls, Enter or Space activates year buttons, and
        Escape closes dialogs and returns focus to the opener. Left and Right
        switch Help tabs; Home and End select the first and last tab. Reduced
        motion follows your system preference.
      </p>
    </>
  );
}
