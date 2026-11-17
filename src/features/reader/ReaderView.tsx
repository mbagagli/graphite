import { PanelDivider } from "../../shared/PanelDivider";
import {
  ArrowLeft,
  BookOpen,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Columns2,
  FileSearch,
  LoaderCircle,
  MessageSquareText,
  Minus,
  PanelLeft,
  Pencil,
  Plus,
  RectangleHorizontal,
  Search,
  X,
} from "lucide-react";
import { motion } from "motion/react";
import {
  GlobalWorkerOptions,
  getDocument,
  type PDFDocumentProxy,
  type PDFDocumentLoadingTask,
} from "pdfjs-dist";
import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type WheelEvent as ReactWheelEvent,
} from "react";

import type { ResearchItem, Topic } from "../../domain/research";
import type {
  DocumentOpenResult,
  DocumentTextPayload,
  ReaderState,
} from "../../services/contracts";
import { assistantDisplayName } from "../../shared/productIdentity";
import { readerCopies } from "./readerCopies";
import { MAX_EDITABLE_PDF_BYTES } from "./pdfEditingTypes";
import { services } from "../../services/runtime";
import { ExtractionControls } from "./ExtractionControls";
import { PaperSynthesis } from "./PaperSynthesis";
import { ReaderPageNumber } from "./ReaderPageNumber";
import { ReaderWheelGate } from "./readerWheel";
import { PdfPageCanvas, PdfThumbnail } from "./PdfPageCanvas";
import {
  activeReaderOutlineId,
  isReaderTextEntryTarget,
  readerNavigationTarget,
  readerPageWindow,
  readerVisiblePages,
  resolveReaderOutline,
  restoreReaderState,
  type ReaderOutlineEntry,
  type ReaderViewMode,
} from "./readerModel";
import { usePaperSynthesis } from "./usePaperSynthesis";

GlobalWorkerOptions.workerSrc = new URL(
  "pdfjs-dist/build/pdf.worker.mjs",
  import.meta.url,
).toString();

const PdfEditor = lazy(() =>
  import("./PdfEditor").then((module) => ({
    default: module.PdfEditor,
  })),
);

interface ReaderViewProps {
  paper: ResearchItem;
  topics: Topic[];
  initialDocumentId?: string;
  initialPageNumber?: number;
  onContextChange: (context: {
    documentId: string;
    pageNumber?: number;
  }) => void;
  onBack: () => void;
  onAsk: () => void;
}

export function ReaderView({
  paper,
  topics,
  initialDocumentId,
  initialPageNumber,
  onContextChange,
  onBack,
  onAsk,
}: ReaderViewProps) {
  const pdfs = useMemo(
    () =>
      paper.attachments.filter(
        (attachment) => attachment.mediaType === "application/pdf",
      ),
    [paper.attachments],
  );
  const [documentId, setDocumentId] = useState(
    pdfs.some((item) => item.documentId === initialDocumentId)
      ? (initialDocumentId ?? "")
      : (pdfs[0]?.documentId ?? ""),
  );
  const attachment = pdfs.find((item) => item.documentId === documentId);
  const [resolution, setResolution] = useState<DocumentOpenResult | null>(null);
  const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null);
  const [editorBytes, setEditorBytes] =
    useState<Uint8Array<ArrayBuffer> | null>(null);
  const [copyAvailable, setCopyAvailable] = useState(false);
  const [usingCopy, setUsingCopy] = useState(false);
  const [showOriginal, setShowOriginal] = useState(Boolean(initialPageNumber));
  const [copyVersion, setCopyVersion] = useState(0);
  const [sourceRevision, setSourceRevision] = useState("");
  const [openingEditor, setOpeningEditor] = useState(false);
  const editedPage = useRef<number | null>(null);
  const [fallbackText, setFallbackText] = useState<DocumentTextPayload | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [pageNumber, setPageNumber] = useState(initialPageNumber ?? 1);
  const [zoom, setZoom] = useState(1);
  const [fitMode, setFitMode] = useState<ReaderState["fitMode"]>("width");
  const [viewMode, setViewMode] = useState<ReaderViewMode>("single");
  const [outlineOpen, setOutlineOpen] = useState(true);
  const [contentsExpanded, setContentsExpanded] = useState(true);
  const [thumbnailsExpanded, setThumbnailsExpanded] = useState(true);
  const [contextOpen, setContextOpen] = useState(true);
  const [outline, setOutline] = useState<ReaderOutlineEntry[]>([]);
  const [findOpen, setFindOpen] = useState(false);
  const [findQuery, setFindQuery] = useState("");
  const [findPages, setFindPages] = useState<number[]>([]);
  const [findProgress, setFindProgress] = useState<number | null>(null);
  const pageAreaRef = useRef<HTMLElement>(null);
  const wheelGate = useRef(new ReaderWheelGate());
  const primaryTopic = topics.find((topic) => topic.id === paper.topics[0]);
  const synthesis = usePaperSynthesis(paper.id, documentId);

  useEffect(() => {
    if (!documentId) return;
    let active = true;
    let loadingTask: PDFDocumentLoadingTask | undefined;
    Promise.all([
      services.document.resolve(documentId),
      services.document.getText(documentId),
      services.document.getReaderState(documentId),
    ])
      .then(async ([nextResolution, text, storedState]) => {
        if (!active) return;
        setError(null);
        setResolution(nextResolution);
        setFallbackText(text);
        if (nextResolution.status !== "ready" || !nextResolution.url) return;
        loadingTask = getDocument({ url: nextResolution.url });
        let loaded = await loadingTask.promise;
        if (!active) return;
        const revision =
          nextResolution.revisionKey ?? loaded.fingerprints[0] ?? "";
        const copy = revision
          ? await readerCopies
              .load(documentId, revision)
              .catch((reason: unknown) => {
                if (active) setNotice(errorMessage(reason));
                return null;
              })
          : null;
        if (!active) return;
        setSourceRevision(revision);
        setCopyAvailable(Boolean(copy));
        setUsingCopy(Boolean(copy) && !showOriginal);
        if (copy && !showOriginal) {
          const bytes = new Uint8Array(await copy.pdf.arrayBuffer());
          if (!active) return;
          await loadingTask.destroy();
          if (!active) return;
          loadingTask = getDocument({ data: bytes });
          loaded = await loadingTask.promise;
        }
        if (!active) {
          return;
        }
        setPdf(loaded);
        const restored = restoreReaderState(
          documentId,
          storedState,
          loaded.numPages,
        );
        setPageNumber(
          editedPage.current !== null
            ? Math.min(editedPage.current, loaded.numPages)
            : initialDocumentId === documentId && initialPageNumber
              ? Math.min(Math.max(1, initialPageNumber), loaded.numPages)
              : restored.pageNumber,
        );
        editedPage.current = null;
        setZoom(restored.zoom);
        setFitMode(restored.fitMode);
        setOutlineOpen(restored.outlineOpen);
        setContextOpen(restored.contextOpen);
        const pdfOutline = await resolveReaderOutline(loaded);
        if (active) {
          setOutline(pdfOutline);
          setPageNumber((value) =>
            Math.min(Math.max(1, value), loaded.numPages),
          );
        }
      })
      .catch((reason: unknown) => {
        if (active) setError(errorMessage(reason));
      });
    return () => {
      active = false;
      void loadingTask?.destroy();
    };
  }, [
    documentId,
    initialDocumentId,
    initialPageNumber,
    showOriginal,
    copyVersion,
  ]);

  useEffect(() => {
    if (!documentId) return;
    onContextChange({
      documentId,
      pageNumber: usingCopy ? undefined : pageNumber,
    });
  }, [documentId, onContextChange, pageNumber, usingCopy]);

  const goToPage = useCallback(
    (value: number) => {
      if (!Number.isFinite(value)) return;
      wheelGate.current.block();
      setPageNumber(
        Math.min(Math.max(1, Math.trunc(value)), pdf?.numPages ?? 1),
      );
      window.requestAnimationFrame(() => {
        pageAreaRef.current?.scrollTo({
          top: 0,
          behavior: "instant",
        });
      });
    },
    [pdf?.numPages],
  );

  const navigatePage = useCallback(
    (direction: "previous" | "next") => {
      goToPage(
        readerNavigationTarget(
          pageNumber,
          pdf?.numPages ?? 1,
          viewMode,
          direction,
        ),
      );
    },
    [goToPage, pageNumber, pdf?.numPages, viewMode],
  );

  useEffect(() => {
    if (!pdf) return;
    const visiblePages = readerVisiblePages(pageNumber, pdf.numPages, viewMode);
    const preload = new Set(
      visiblePages.flatMap((page) => readerPageWindow(page, pdf.numPages)),
    );
    for (const page of preload) {
      if (!visiblePages.includes(page)) {
        void pdf.getPage(page).catch(() => {
          // Speculative preloading may fail; the visible render reports errors.
        });
      }
    }
  }, [pageNumber, pdf, viewMode]);

  useEffect(() => {
    if (!documentId || !pdf) return;
    const timer = window.setTimeout(() => {
      void services.document
        .saveReaderState({
          documentId,
          pageNumber,
          zoom,
          fitMode,
          outlineOpen,
          contextOpen,
        })
        .catch((reason: unknown) => setNotice(errorMessage(reason)));
    }, 350);
    return () => window.clearTimeout(timer);
  }, [contextOpen, documentId, fitMode, outlineOpen, pageNumber, pdf, zoom]);

  useEffect(() => {
    const handleKey = (event: KeyboardEvent) => {
      if (document.querySelector("dialog[open]")) return;
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        setFindOpen(true);
      } else if (event.key === "Escape" && findOpen) {
        setFindOpen(false);
      } else if (
        event.target instanceof HTMLElement &&
        isReaderTextEntryTarget(
          event.target.tagName,
          event.target.isContentEditable,
        )
      ) {
        return;
      } else if (
        event.key === "PageDown" ||
        event.key === "ArrowRight" ||
        event.key === "ArrowDown"
      ) {
        event.preventDefault();
        navigatePage("next");
      } else if (
        event.key === "PageUp" ||
        event.key === "ArrowLeft" ||
        event.key === "ArrowUp"
      ) {
        event.preventDefault();
        navigatePage("previous");
      } else if (event.key === "+" || event.key === "=") {
        event.preventDefault();
        setZoom((value) => Math.min(3, value + 0.1));
        setFitMode("custom");
      } else if (event.key === "-") {
        event.preventDefault();
        setZoom((value) => Math.max(0.4, value - 0.1));
        setFitMode("custom");
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [findOpen, navigatePage]);

  useEffect(() => {
    if (!pdf || findQuery.trim().length < 2) {
      return;
    }
    let active = true;
    const timer = window.setTimeout(async () => {
      const query = findQuery.trim().toLocaleLowerCase();
      const pages: number[] = [];
      setFindProgress(0);
      for (let index = 1; index <= pdf.numPages; index += 1) {
        if (!active) return;
        const page = await pdf.getPage(index);
        const text = await page.getTextContent();
        const joined = text.items
          .map((item) => ("str" in item ? item.str : ""))
          .join(" ")
          .toLocaleLowerCase();
        if (joined.includes(query)) pages.push(index);
        setFindProgress(index / pdf.numPages);
      }
      if (active) {
        setFindPages(pages);
        setFindProgress(null);
      }
    }, 220);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [findQuery, pdf]);

  const handlePageWheel = useCallback(
    (event: ReactWheelEvent<HTMLElement>) => {
      if (event.ctrlKey || event.metaKey || event.deltaY === 0) return;
      const target = event.target;
      if (
        target instanceof HTMLElement &&
        isReaderTextEntryTarget(target.tagName, target.isContentEditable)
      ) {
        return;
      }
      const area = event.currentTarget;
      const direction = event.deltaY > 0 ? "next" : "previous";
      const atBoundary =
        direction === "next"
          ? area.scrollTop + area.clientHeight >= area.scrollHeight - 2
          : area.scrollTop <= 2;
      if (!wheelGate.current.shouldTurn(event.deltaY, atBoundary)) return;
      const targetPage = readerNavigationTarget(
        pageNumber,
        pdf?.numPages ?? 1,
        viewMode,
        direction,
      );
      if (targetPage === pageNumber) return;
      goToPage(targetPage);
    },
    [goToPage, pageNumber, pdf?.numPages, viewMode],
  );

  const selectDocument = (nextDocumentId: string) => {
    setResolution(null);
    setPdf(null);
    setFallbackText(null);
    setError(null);
    setNotice(null);
    setOutline([]);
    setPageNumber(1);
    setFindQuery("");
    setFindPages([]);
    setFindProgress(null);
    setCopyAvailable(false);
    setUsingCopy(false);
    setShowOriginal(false);
    setDocumentId(nextDocumentId);
  };

  const updateFindQuery = (value: string) => {
    setFindQuery(value);
    if (value.trim().length < 2) {
      setFindPages([]);
      setFindProgress(null);
    }
  };

  const openEditor = async () => {
    if (!pdf || openingEditor) return;
    setOpeningEditor(true);
    setNotice(null);
    try {
      if ((resolution?.contentLength ?? 0) > MAX_EDITABLE_PDF_BYTES) {
        throw new Error("PDF editing supports files up to 128 MB.");
      }
      const saved =
        copyAvailable && showOriginal
          ? await readerCopies.load(documentId, sourceRevision)
          : null;
      const bytes = saved
        ? new Uint8Array(await saved.pdf.arrayBuffer())
        : await pdf.getData();
      if (bytes.length > MAX_EDITABLE_PDF_BYTES) {
        throw new Error("PDF editing supports files up to 128 MB.");
      }
      setEditorBytes(new Uint8Array(bytes));
    } catch (reason) {
      setNotice(errorMessage(reason));
    } finally {
      setOpeningEditor(false);
    }
  };

  const visiblePages = pdf
    ? readerVisiblePages(pageNumber, pdf.numPages, viewMode)
    : [];
  const activeOutlineId = activeReaderOutlineId(outline, pageNumber);

  return (
    <motion.section
      id="main-content"
      tabIndex={-1}
      className={`reader-view${outlineOpen ? " has-outline" : ""}${contextOpen ? " has-context" : ""}`}
      initial={{ opacity: 0, scale: 0.995 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.3 }}
    >
      <header className="reader-toolbar">
        <button type="button" className="back-button" onClick={onBack}>
          <ArrowLeft size={16} /> Back to graph
        </button>
        <div className="reader-title">
          <span style={{ background: primaryTopic?.color }} />
          <div>
            <strong>{paper.title}</strong>
            <small>
              {paper.authors[0]?.name ?? "Unknown creator"} /{" "}
              {paper.year ?? "No year"}
            </small>
          </div>
        </div>
        {pdfs.length > 1 && (
          <select
            name="reader-document"
            aria-label="Reader document"
            value={documentId}
            onChange={(event) => selectDocument(event.target.value)}
          >
            {pdfs.map((item) => (
              <option key={item.documentId} value={item.documentId}>
                {item.title}
              </option>
            ))}
          </select>
        )}
        <div className="reader-actions">
          {copyAvailable && (
            <span
              className="reader-copy-control"
              title="Search, citations, and the assistant use the original PDF."
            >
              {usingCopy ? "Edited copy" : "Original"}
              <button
                type="button"
                onClick={() => setShowOriginal((value) => !value)}
              >
                {usingCopy ? "View original" : "View edited copy"}
              </button>
            </span>
          )}
          <button
            type="button"
            className="reader-edit-button"
            disabled={!pdf || !sourceRevision || openingEditor}
            onClick={() => {
              void openEditor();
            }}
          >
            <Pencil size={14} />
            {openingEditor ? "Opening..." : "Edit PDF"}
          </button>
          <button
            type="button"
            onClick={() => setOutlineOpen((value) => !value)}
            aria-label="Toggle contents"
          >
            <PanelLeft size={15} />
          </button>
          <button
            type="button"
            onClick={() => setFindOpen(true)}
            aria-label="Find in paper"
          >
            <Search size={15} />
          </button>
          <button
            type="button"
            onClick={() => setContextOpen((value) => !value)}
            aria-label="Toggle research context"
          >
            <FileSearch size={15} />
          </button>
        </div>
      </header>

      {outlineOpen && (
        <aside
          className="reader-outline resizable-panels"
          aria-label="PDF navigation"
        >
          <section
            className={`reader-outline-section${contentsExpanded ? " is-expanded" : ""}`}
          >
            <button
              type="button"
              className="reader-outline-section-heading"
              aria-expanded={contentsExpanded}
              aria-controls="reader-outline-contents"
              onClick={() => setContentsExpanded((current) => !current)}
            >
              <span>CONTENTS</span>
              <span className="reader-outline-section-affordance">
                <PanelLeft size={12} />
                <ChevronDown size={13} />
              </span>
            </button>
            <div
              id="reader-outline-contents"
              className="reader-outline-section-content"
              role="region"
              aria-label="PDF table of contents"
              hidden={!contentsExpanded}
            >
              {outline.length > 0 ? (
                <nav aria-label="PDF table of contents entries">
                  {outline.map((entry) => (
                    <button
                      type="button"
                      key={entry.id}
                      className={
                        entry.id === activeOutlineId ? "is-active" : ""
                      }
                      disabled={entry.pageNumber === null}
                      onClick={() =>
                        entry.pageNumber !== null && goToPage(entry.pageNumber)
                      }
                      style={
                        {
                          "--reader-outline-depth": entry.depth,
                        } as CSSProperties
                      }
                    >
                      <span>
                        {entry.pageNumber === null
                          ? "--"
                          : String(entry.pageNumber).padStart(2, "0")}
                      </span>
                      {entry.title}
                    </button>
                  ))}
                </nav>
              ) : (
                <p className="reader-outline-empty">
                  This PDF does not provide a table of contents.
                </p>
              )}
            </div>
          </section>

          <PanelDivider
            storageKey="graphite.reader-panel-split"
            label="Resize Contents and Thumbnails panels"
            enabled={contentsExpanded && thumbnailsExpanded}
          />

          <section
            className={`reader-outline-section${thumbnailsExpanded ? " is-expanded" : ""}`}
          >
            <button
              type="button"
              className="reader-outline-section-heading"
              aria-expanded={thumbnailsExpanded}
              aria-controls="reader-outline-thumbnails"
              onClick={() => setThumbnailsExpanded((current) => !current)}
            >
              <span>THUMBNAILS</span>
              <span className="reader-outline-section-affordance">
                <RectangleHorizontal size={12} />
                <ChevronDown size={13} />
              </span>
            </button>
            <div
              id="reader-outline-thumbnails"
              className="reader-outline-section-content"
              role="region"
              aria-label="PDF page thumbnails"
              hidden={!thumbnailsExpanded}
            >
              <div className="thumbnail-stack">
                {pdf &&
                  Array.from(
                    { length: pdf.numPages },
                    (_, index) => index + 1,
                  ).map((page) => (
                    <PdfThumbnail
                      key={page}
                      pdf={pdf}
                      pageNumber={page}
                      active={visiblePages.includes(page)}
                      onSelect={goToPage}
                    />
                  ))}
              </div>
            </div>
          </section>
        </aside>
      )}

      <main
        className="reader-page-area"
        ref={pageAreaRef}
        onWheel={handlePageWheel}
      >
        <div className="reader-page-tools">
          <button
            type="button"
            onClick={() => navigatePage("previous")}
            disabled={pageNumber <= 1}
            aria-label="Previous page"
          >
            <ChevronLeft size={15} />
          </button>
          <label>
            <ReaderPageNumber
              value={pageNumber}
              maximum={pdf?.numPages ?? attachment?.pageCount ?? 1}
              onChange={goToPage}
            />{" "}
            / {pdf?.numPages ?? attachment?.pageCount ?? "-"}
          </label>
          <button
            type="button"
            onClick={() => navigatePage("next")}
            disabled={pageNumber >= (pdf?.numPages ?? 1)}
            aria-label="Next page"
          >
            <ChevronRight size={15} />
          </button>
          <i />
          <div
            className="reader-view-mode"
            role="group"
            aria-label="Page layout"
          >
            <button
              type="button"
              className={viewMode === "single" ? "is-active" : ""}
              aria-pressed={viewMode === "single"}
              onClick={() => setViewMode("single")}
            >
              <RectangleHorizontal size={13} /> Single
            </button>
            <button
              type="button"
              className={viewMode === "spread" ? "is-active" : ""}
              aria-pressed={viewMode === "spread"}
              onClick={() => setViewMode("spread")}
            >
              <Columns2 size={13} /> Spread
            </button>
          </div>
          <i />
          <button
            type="button"
            onClick={() => {
              setZoom((value) => Math.max(0.4, value - 0.1));
              setFitMode("custom");
            }}
            aria-label="Zoom out"
          >
            <Minus size={14} />
          </button>
          <button
            type="button"
            className="reader-zoom-value"
            onClick={() => {
              setZoom(1);
              setFitMode("width");
            }}
          >
            {Math.round(zoom * 100)}%
          </button>
          <button
            type="button"
            onClick={() => {
              setZoom((value) => Math.min(3, value + 0.1));
              setFitMode("custom");
            }}
            aria-label="Zoom in"
          >
            <Plus size={14} />
          </button>
        </div>
        {findOpen && (
          <div className="reader-find">
            <Search size={14} />
            <input
              autoFocus
              name="reader-find"
              autoComplete="off"
              value={findQuery}
              onChange={(event) => updateFindQuery(event.target.value)}
              placeholder="Find text in PDF"
              aria-label="Find text in PDF"
            />
            <span>
              {findProgress !== null
                ? `${Math.round(findProgress * 100)}%`
                : `${findPages.length} pages`}
            </span>
            {findPages.slice(0, 6).map((page) => (
              <button type="button" key={page} onClick={() => goToPage(page)}>
                {page}
              </button>
            ))}
            <button
              type="button"
              onClick={() => setFindOpen(false)}
              aria-label="Close find"
            >
              <X size={14} />
            </button>
          </div>
        )}
        {notice && (
          <div className="reader-notice" role="status">
            <span>{notice}</span>
            <button
              type="button"
              aria-label="Dismiss reader notice"
              onClick={() => setNotice(null)}
            >
              <X size={14} />
            </button>
          </div>
        )}
        {!resolution && !error && (
          <div className="reader-document-state">
            <LoaderCircle className="spin" size={22} /> Resolving local
            attachment...
          </div>
        )}
        {error && (
          <DocumentFallback
            title="The PDF could not be rendered"
            message={error}
            text={fallbackText}
          />
        )}
        {resolution && resolution.status !== "ready" && (
          <DocumentFallback
            title="Document unavailable"
            message={
              resolution.message ?? "The local attachment could not be opened."
            }
            text={fallbackText}
          />
        )}
        {resolution?.status === "ready" && !pdf && !error && (
          <div className="reader-document-state">
            <LoaderCircle className="spin" size={22} /> Loading PDF document...
          </div>
        )}
        {pdf && (
          <div
            className={`pdf-page-stage is-${viewMode}`}
            aria-label={
              visiblePages.length === 2
                ? `Pages ${visiblePages[0]} and ${visiblePages[1]} of ${pdf.numPages}`
                : `Page ${visiblePages[0]} of ${pdf.numPages}`
            }
          >
            {visiblePages.map((page, index) => (
              <PdfPageCanvas
                key={`${documentId}-${index}`}
                pdf={pdf}
                pageNumber={page}
                scale={zoom * (viewMode === "spread" ? 0.58 : 1.25)}
                ariaLabel={`Page ${page} of ${pdf.numPages}`}
                onRenderError={setError}
              />
            ))}
          </div>
        )}
      </main>

      {contextOpen && (
        <aside className="reader-context">
          <div className="reader-panel-heading">
            <FileSearch size={14} />
            <span>RESEARCH CONTEXT</span>
          </div>
          <section>
            <span className="eyebrow">CURRENT PAPER</span>
            <h3>{primaryTopic?.name ?? "Uncategorized"}</h3>
            <p>
              {paper.abstractText ?? "No abstract is stored for this paper."}
            </p>
          </section>
          <PaperSynthesis
            documentId={documentId}
            synthesis={synthesis}
            onOpenAgent={onAsk}
            onNavigatePage={goToPage}
          />
          <ExtractionControls
            key={documentId}
            documentId={documentId}
            onChanged={async () => {
              setFallbackText(await services.document.getText(documentId));
            }}
          />
          <section className="reader-corpus-status">
            <span className="eyebrow">DOCUMENT CORPUS</span>
            <strong>
              {fallbackText
                ? `${fallbackText.completeness} text`
                : "Text not indexed"}
            </strong>
            <small>
              {fallbackText?.indexedPages != null
                ? `${fallbackText.indexedPages} extracted pages`
                : "Saved text"}{" "}
              / {fallbackText?.provider ?? "no provider"}
            </small>
          </section>
          <button type="button" className="ask-paper-button" onClick={onAsk}>
            <MessageSquareText size={15} /> Ask {assistantDisplayName}
          </button>
          <div className="reader-placeholder-note">
            <BookOpen size={15} />
            <p>
              <strong>Local reader</strong> PDF bytes and indexed text stay on
              this computer.
            </p>
          </div>
        </aside>
      )}
      {editorBytes && (
        <Suspense
          fallback={
            <div className="reader-document-state">Opening PDF editor...</div>
          }
        >
          <PdfEditor
            initialBytes={editorBytes}
            documentId={documentId}
            sourceRevision={sourceRevision}
            title={paper.title}
            initialPage={pageNumber - 1}
            onClose={(saved, page) => {
              setEditorBytes(null);
              if (saved) {
                editedPage.current = page + 1;
                setShowOriginal(false);
                setCopyVersion((value) => value + 1);
              }
            }}
          />
        </Suspense>
      )}
    </motion.section>
  );
}

function DocumentFallback({
  title,
  message,
  text,
}: {
  title: string;
  message: string;
  text: DocumentTextPayload | null;
}) {
  return (
    <article className="document-fallback">
      <FileSearch size={24} />
      <h2>{title}</h2>
      <p>{message}</p>
      {text ? (
        <>
          <span className="eyebrow">SAVED TEXT / {text.completeness}</span>
          <pre>{text.content}</pre>
        </>
      ) : (
        <small>
          No indexed-text fallback is available for this attachment.
        </small>
      )}
    </article>
  );
}

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}
