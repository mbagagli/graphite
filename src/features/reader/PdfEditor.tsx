import {
  ChevronLeft,
  ChevronRight,
  Download,
  FilePlus,
  Highlighter,
  MousePointer2,
  Pencil,
  Redo2,
  RotateCcw,
  RotateCw,
  Save,
  StickyNote,
  Trash2,
  Type,
  Undo2,
  X,
} from "lucide-react";
import { getDocument, type PDFDocumentProxy } from "pdfjs-dist";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { ModalDialog } from "../../components/ModalDialog";
import { PdfEditorClient } from "./PdfEditorClient";
import { PdfEditorPage, type PdfEditorTool } from "./PdfEditorPage";
import { ReaderPageNumber } from "./ReaderPageNumber";
import { readerCopies } from "./readerCopies";
import { exportPdfCopy } from "./readerExport";
import { isReaderTextEntryTarget } from "./readerModel";
import { MAX_EDITABLE_PDF_BYTES } from "./pdfEditingTypes";
import type {
  PdfAnnotation,
  PdfEdit,
  PdfPageContents,
  PdfPoint,
  PdfTextObject,
} from "./pdfEditingTypes";
import "./pdf-editor.css";

const MAX_UNDO_STEPS = 12;
const MAX_UNDO_BYTES = 128 * 1024 * 1024;
interface Revision {
  bytes: Uint8Array<ArrayBuffer>;
  page: number;
}

function retainHistory(history: Revision[]) {
  let size = 0;
  return history
    .slice(-MAX_UNDO_STEPS)
    .reverse()
    .filter((item, index) => {
      size += item.bytes.byteLength;
      return index === 0 || size <= MAX_UNDO_BYTES;
    })
    .reverse();
}

export function PdfEditor({
  initialBytes,
  documentId,
  sourceRevision,
  title,
  initialPage,
  onClose,
}: {
  initialBytes: Uint8Array<ArrayBuffer>;
  documentId: string;
  sourceRevision: string;
  title: string;
  initialPage: number;
  onClose: (saved: boolean, page: number) => void;
}) {
  const [current, setCurrent] = useState<Revision>({
    bytes: initialBytes,
    page: initialPage,
  });
  const [savedBytes, setSavedBytes] = useState(initialBytes);
  const [savedOnce, setSavedOnce] = useState(false);
  const [undo, setUndo] = useState<Revision[]>([]);
  const [redo, setRedo] = useState<Revision[]>([]);
  const [loaded, setLoaded] = useState<{
    bytes: Uint8Array<ArrayBuffer>;
    pdf: PDFDocumentProxy;
  } | null>(null);
  const [inspected, setInspected] = useState<{
    bytes: Uint8Array<ArrayBuffer>;
    page: number;
    contents: PdfPageContents;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const [tool, setTool] = useState<PdfEditorTool>("select");
  const [panel, setPanel] = useState<"notes" | "pages">("notes");
  const [width, setWidth] = useState(650);
  const [zoom, setZoom] = useState(1);
  const [moveTo, setMoveTo] = useState(1);
  const [textTarget, setTextTarget] = useState<PdfTextObject | null>(null);
  const [noteTarget, setNoteTarget] = useState<PdfPoint | PdfAnnotation | null>(
    null,
  );
  const [draft, setDraft] = useState("");
  const [font, setFont] = useState<"original" | "noto">("original");
  const [confirmClose, setConfirmClose] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const client = useRef<PdfEditorClient | null>(null);
  const pageArea = useRef<HTMLDivElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const dirty = current.bytes !== savedBytes;
  const ready = loaded?.bytes === current.bytes;
  const pdf = loaded?.pdf;
  const locked = busy || !ready;
  const pageCount = pdf?.numPages ?? 1;
  const contents =
    inspected?.bytes === current.bytes && inspected.page === current.page
      ? inspected.contents
      : null;

  useEffect(() => {
    const worker = new PdfEditorClient();
    client.current = worker;
    return () => {
      worker.dispose();
      client.current = null;
    };
  }, []);

  useEffect(() => {
    let active = true;
    const loading = getDocument({ data: current.bytes.slice() });
    loading.promise
      .then((next) => {
        if (!active) return;
        setLoaded({ bytes: current.bytes, pdf: next });
        setCurrent((value) => ({
          ...value,
          page: Math.min(value.page, next.numPages - 1),
        }));
      })
      .catch((reason: unknown) => {
        if (active) setError(message(reason));
      });
    return () => {
      active = false;
      void loading.destroy();
    };
  }, [current.bytes]);

  useEffect(() => {
    let active = true;
    client.current
      ?.inspect(current.bytes, current.page)
      .then((contents) => {
        if (active)
          setInspected({ bytes: current.bytes, page: current.page, contents });
      })
      .catch((reason: unknown) => {
        if (active) setError(message(reason));
      });
    return () => {
      active = false;
    };
  }, [current.bytes, current.page]);

  useEffect(() => {
    const area = pageArea.current;
    if (!area) return;
    const observer = new ResizeObserver(() =>
      setWidth(Math.max(240, area.clientWidth - 48)),
    );
    observer.observe(area);
    return () => observer.disconnect();
  }, []);

  function changePage(page: number) {
    setCurrent((value) => ({
      ...value,
      page: Math.max(0, Math.min(pageCount - 1, page)),
    }));
    pageArea.current?.scrollTo({ top: 0, behavior: "instant" });
  }

  async function apply(change: PdfEdit) {
    if (!client.current || locked) return false;
    setBusy(true);
    setError(null);
    try {
      const bytes = await client.current.edit(current.bytes, change);
      setUndo((history) => retainHistory([...history, current]));
      setRedo([]);
      const page =
        change.kind === "move-page"
          ? change.destination
          : change.kind === "insert-blank" || change.kind === "insert-pdf"
            ? change.page + 1
            : change.kind === "delete-page"
              ? Math.min(change.page, pageCount - 2)
              : current.page;
      setCurrent({ bytes, page });
      setStatus("Unsaved changes");
      return true;
    } catch (reason) {
      setError(message(reason));
      return false;
    } finally {
      setBusy(false);
    }
  }

  function undoChange() {
    if (locked || !undo.length) return;
    setRedo((history) => retainHistory([...history, current]));
    setCurrent(undo.at(-1)!);
    setUndo((history) => history.slice(0, -1));
    setError(null);
  }

  function redoChange() {
    if (locked || !redo.length) return;
    setUndo((history) => retainHistory([...history, current]));
    setCurrent(redo.at(-1)!);
    setRedo((history) => history.slice(0, -1));
    setError(null);
  }

  async function saveCopy(close = false) {
    if (busy) return;
    setBusy(true);
    try {
      await readerCopies.save(documentId, sourceRevision, current.bytes);
      setSavedBytes(current.bytes);
      setSavedOnce(true);
      setStatus("Saved in Graphite");
      if (close) onClose(true, current.page);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setBusy(false);
    }
  }

  function requestClose() {
    if (busy) return;
    if (dirty) setConfirmClose(true);
    else onClose(savedOnce, current.page);
  }

  function handleKey(event: KeyboardEvent<HTMLDivElement>) {
    if (textTarget || noteTarget || confirmClose || confirmDelete) return;
    const modifier = event.metaKey || event.ctrlKey;
    if (modifier && event.key.toLowerCase() === "s") {
      event.preventDefault();
      void saveCopy();
      return;
    }
    if (
      event.target instanceof HTMLElement &&
      isReaderTextEntryTarget(
        event.target.tagName,
        event.target.isContentEditable,
      )
    )
      return;
    if (modifier && event.key.toLowerCase() === "z") {
      event.preventDefault();
      if (event.shiftKey) redoChange();
      else undoChange();
    } else if (
      !locked &&
      (event.key === "PageDown" || event.key === "ArrowRight")
    ) {
      event.preventDefault();
      changePage(current.page + 1);
    } else if (
      !locked &&
      (event.key === "PageUp" || event.key === "ArrowLeft")
    ) {
      event.preventDefault();
      changePage(current.page - 1);
    }
  }

  const tools = [
    ["select", "Select", MousePointer2],
    ["highlight", "Highlight", Highlighter],
    ["note", "Note", StickyNote],
    ["drawing", "Draw", Pencil],
    ["text", "Edit text", Type],
  ] as const;

  return (
    <ModalDialog
      title="Edit PDF"
      titleId="pdf-editor-title"
      className="pdf-editor-dialog"
      onClose={requestClose}
    >
      <div className="pdf-editor-body" onKeyDown={handleKey}>
        <p className="pdf-editor-source">
          {title}{" "}
          <span>Editing a local copy. The source PDF stays unchanged.</span>
        </p>
        <div
          className="pdf-editor-toolbar"
          role="toolbar"
          aria-label="PDF editing tools"
        >
          {tools.map(([value, label, Icon]) => (
            <button
              type="button"
              key={value}
              aria-pressed={tool === value}
              disabled={locked}
              onClick={() => setTool(value)}
            >
              <Icon size={15} />
              {label}
            </button>
          ))}
          <span className="pdf-editor-toolbar-divider" />
          <button
            type="button"
            disabled={!undo.length || locked}
            onClick={undoChange}
            title="Undo (Ctrl/Command+Z)"
          >
            <Undo2 size={15} />
            Undo
          </button>
          <button
            type="button"
            disabled={!redo.length || locked}
            onClick={redoChange}
            title="Redo (Ctrl/Command+Shift+Z)"
          >
            <Redo2 size={15} />
            Redo
          </button>
          <button
            type="button"
            aria-pressed={panel === "pages"}
            onClick={() => setPanel(panel === "pages" ? "notes" : "pages")}
          >
            <FilePlus size={15} />
            Pages
          </button>
        </div>
        {error && (
          <div className="pdf-editor-error" role="alert">
            <span>{error}</span>
            <button
              type="button"
              aria-label="Dismiss PDF editing error"
              onClick={() => setError(null)}
            >
              <X size={16} />
            </button>
          </div>
        )}
        <div className="pdf-editor-workspace">
          <main className="pdf-editor-preview" ref={pageArea}>
            <div className="pdf-editor-navigation">
              <button
                type="button"
                aria-label="Previous editor page"
                disabled={locked || current.page === 0}
                onClick={() => changePage(current.page - 1)}
              >
                <ChevronLeft size={16} />
              </button>
              <ReaderPageNumber
                value={current.page + 1}
                maximum={pageCount}
                onChange={(page) => changePage(page - 1)}
              />{" "}
              / {pageCount}
              <button
                type="button"
                aria-label="Next editor page"
                disabled={locked || current.page >= pageCount - 1}
                onClick={() => changePage(current.page + 1)}
              >
                <ChevronRight size={16} />
              </button>
              <button
                type="button"
                disabled={locked || zoom <= 0.5}
                onClick={() => setZoom((value) => Math.max(0.5, value - 0.25))}
              >
                -
              </button>
              <button
                type="button"
                title="Fit page width"
                onClick={() => setZoom(1)}
              >
                {zoom === 1 ? "Fit width" : `${Math.round(zoom * 100)}%`}
              </button>
              <button
                type="button"
                disabled={locked || zoom >= 3}
                onClick={() => setZoom((value) => Math.min(3, value + 0.25))}
              >
                +
              </button>
            </div>
            {pdf ? (
              <PdfEditorPage
                pdf={pdf}
                pageNumber={current.page + 1}
                width={width * zoom}
                tool={tool}
                contents={contents}
                disabled={locked}
                onError={setError}
                onEdit={(change) => {
                  void apply(change);
                }}
                onText={(text) => {
                  setTextTarget(text);
                  setDraft(text.text);
                  setFont("original");
                }}
                onNote={(point) => {
                  setNoteTarget(point);
                  setDraft("");
                }}
              />
            ) : (
              <p className="pdf-editor-loading" role="status">
                Opening PDF...
              </p>
            )}
          </main>
          <aside className="pdf-editor-sidebar">
            {panel === "pages" ? (
              <>
                <h3>Page {current.page + 1}</h3>
                <div className="pdf-editor-page-actions">
                  <button
                    type="button"
                    disabled={locked}
                    onClick={() => {
                      void apply({
                        kind: "rotate",
                        page: current.page,
                        turns: -1,
                      });
                    }}
                  >
                    <RotateCcw size={15} />
                    Rotate left
                  </button>
                  <button
                    type="button"
                    disabled={locked}
                    onClick={() => {
                      void apply({
                        kind: "rotate",
                        page: current.page,
                        turns: 1,
                      });
                    }}
                  >
                    <RotateCw size={15} />
                    Rotate right
                  </button>
                  <label>
                    Move to page
                    <input
                      type="number"
                      min={1}
                      max={pageCount}
                      value={moveTo}
                      onChange={(event) =>
                        setMoveTo(Number(event.target.value))
                      }
                    />
                  </label>
                  <button
                    type="button"
                    disabled={
                      locked ||
                      !Number.isInteger(moveTo) ||
                      moveTo < 1 ||
                      moveTo > pageCount ||
                      moveTo === current.page + 1
                    }
                    onClick={() => {
                      void apply({
                        kind: "move-page",
                        page: current.page,
                        destination: moveTo - 1,
                      });
                    }}
                  >
                    Move page
                  </button>
                  <button
                    type="button"
                    disabled={locked}
                    onClick={() => {
                      void apply({ kind: "insert-blank", page: current.page });
                    }}
                  >
                    <FilePlus size={15} />
                    Insert blank after
                  </button>
                  <button
                    type="button"
                    disabled={locked}
                    onClick={() => fileInput.current?.click()}
                  >
                    <FilePlus size={15} />
                    Insert PDF after
                  </button>
                  <input
                    ref={fileInput}
                    type="file"
                    accept="application/pdf,.pdf"
                    hidden
                    onChange={async (event) => {
                      const file = event.target.files?.[0];
                      event.target.value = "";
                      if (!file) return;
                      try {
                        if (file.size > MAX_EDITABLE_PDF_BYTES) {
                          throw new Error(
                            "PDF editing supports files up to 128 MB.",
                          );
                        }
                        await apply({
                          kind: "insert-pdf",
                          page: current.page,
                          bytes: new Uint8Array(await file.arrayBuffer()),
                        });
                      } catch (reason) {
                        setError(message(reason));
                      }
                    }}
                  />
                  <button
                    type="button"
                    className="pdf-delete-button"
                    disabled={locked || pageCount === 1}
                    onClick={() => setConfirmDelete(true)}
                  >
                    <Trash2 size={15} />
                    Delete page
                  </button>
                </div>
              </>
            ) : (
              <>
                <h3>
                  {tool === "text" ? "Edit existing text" : "On this page"}
                </h3>
                <p>
                  {tool === "highlight"
                    ? "Select text on the page to highlight it."
                    : tool === "note"
                      ? "Click the page to place a note."
                      : tool === "drawing"
                        ? "Drag on the page to draw."
                        : tool === "text"
                          ? "Click a text run to change its words. Nearby text does not reflow."
                          : "Select text to copy it, or choose a tool above."}
                </p>
                {tool === "text" && contents?.hasGroupedText && (
                  <p>Text inside grouped artwork may not be editable here.</p>
                )}
                {tool === "text" && contents && !contents.text.length && (
                  <p>
                    No editable text runs were found. Scanned text needs OCR
                    before it can be edited.
                  </p>
                )}
                <ul className="pdf-annotation-list">
                  {contents?.annotations.map((annotation) => (
                    <li key={annotation.index}>
                      <strong>
                        {annotation.kind === "note"
                          ? "Note"
                          : annotation.kind === "highlight"
                            ? "Highlight"
                            : annotation.kind === "drawing"
                              ? "Drawing"
                              : "Annotation"}
                      </strong>
                      {annotation.text && <p>{annotation.text}</p>}
                      <div>
                        {annotation.kind === "note" && (
                          <button
                            type="button"
                            disabled={locked}
                            onClick={() => {
                              setNoteTarget(annotation);
                              setDraft(annotation.text);
                            }}
                          >
                            Edit note
                          </button>
                        )}
                        <button
                          type="button"
                          disabled={locked}
                          onClick={() => {
                            void apply({
                              kind: "remove-annotation",
                              page: current.page,
                              annotation: annotation.index,
                            });
                          }}
                        >
                          Remove
                        </button>
                      </div>
                    </li>
                  ))}
                </ul>
                {contents?.annotations.length === 0 && (
                  <p className="pdf-editor-empty">
                    No annotations on this page.
                  </p>
                )}
              </>
            )}
          </aside>
        </div>
        <footer className="pdf-editor-footer">
          <span role="status">
            {locked
              ? "Preparing PDF..."
              : dirty
                ? "Unsaved changes"
                : status || "Ready to edit"}
          </span>
          <button
            type="button"
            disabled={locked}
            onClick={async () => {
              setBusy(true);
              setError(null);
              try {
                if (await exportPdfCopy(current.bytes, title))
                  setStatus("PDF exported");
              } catch (reason) {
                setError(message(reason));
              } finally {
                setBusy(false);
              }
            }}
          >
            <Download size={15} />
            Export PDF
          </button>
          <button
            type="button"
            className="pdf-save-button"
            disabled={locked || !dirty}
            onClick={() => {
              void saveCopy();
            }}
          >
            <Save size={15} />
            Save copy
          </button>
        </footer>
      </div>
      {(textTarget || noteTarget) && (
        <ModalDialog
          title={textTarget ? "Edit text" : "PDF note"}
          titleId="pdf-edit-detail-title"
          className="pdf-edit-detail"
          onClose={() => {
            setTextTarget(null);
            setNoteTarget(null);
          }}
        >
          <div className="modal-dialog-body">
            {error && <p role="alert">{error}</p>}
            <label>
              {textTarget ? "Replacement text" : "Note"}
              <textarea
                autoFocus
                rows={4}
                value={draft}
                onChange={(event) => setDraft(event.target.value)}
              />
            </label>
            {textTarget && (
              <fieldset>
                <legend>Font</legend>
                <label>
                  <input
                    type="radio"
                    name="pdf-font"
                    checked={font === "original"}
                    onChange={() => setFont("original")}
                  />
                  Keep original ({textTarget.font})
                </label>
                <label>
                  <input
                    type="radio"
                    name="pdf-font"
                    checked={font === "noto"}
                    onChange={() => setFont("noto")}
                  />
                  Noto Sans (bundled)
                </label>
                <p>
                  Some embedded fonts contain only the original characters. Noto
                  Sans replaces the font for this text run.
                </p>
              </fieldset>
            )}
            <div className="pdf-edit-dialog-actions">
              <button
                type="button"
                onClick={() => {
                  setTextTarget(null);
                  setNoteTarget(null);
                }}
              >
                Cancel
              </button>
              <button
                type="button"
                disabled={locked || (!textTarget && !draft.trim())}
                onClick={async () => {
                  const change: PdfEdit = textTarget
                    ? {
                        kind: "text",
                        page: current.page,
                        object: textTarget.index,
                        expectedText: textTarget.text,
                        text: draft,
                        font,
                      }
                    : noteTarget && "index" in noteTarget
                      ? {
                          kind: "edit-note",
                          page: current.page,
                          annotation: noteTarget.index,
                          text: draft,
                        }
                      : {
                          kind: "note",
                          page: current.page,
                          point: noteTarget as PdfPoint,
                          text: draft,
                        };
                  if (await apply(change)) {
                    setTextTarget(null);
                    setNoteTarget(null);
                  }
                }}
              >
                Apply
              </button>
            </div>
          </div>
        </ModalDialog>
      )}
      {confirmDelete && (
        <ModalDialog
          title="Delete this page?"
          titleId="pdf-delete-title"
          onClose={() => setConfirmDelete(false)}
        >
          <div className="modal-dialog-body">
            <p>
              Page {current.page + 1} will be removed from this edited copy. You
              can undo this change.
            </p>
            <div className="pdf-edit-dialog-actions">
              <button type="button" onClick={() => setConfirmDelete(false)}>
                Cancel
              </button>
              <button
                type="button"
                disabled={locked}
                onClick={async () => {
                  if (await apply({ kind: "delete-page", page: current.page }))
                    setConfirmDelete(false);
                }}
              >
                Delete page
              </button>
            </div>
          </div>
        </ModalDialog>
      )}
      {confirmClose && (
        <ModalDialog
          title="Save your changes?"
          titleId="pdf-close-title"
          onClose={() => setConfirmClose(false)}
        >
          <div className="modal-dialog-body">
            <p>Save the edited PDF in Graphite before closing?</p>
            <div className="pdf-edit-dialog-actions">
              <button type="button" onClick={() => setConfirmClose(false)}>
                Keep editing
              </button>
              <button
                type="button"
                onClick={() => onClose(savedOnce, current.page)}
              >
                Discard changes
              </button>
              <button
                type="button"
                disabled={busy}
                onClick={() => {
                  void saveCopy(true);
                }}
              >
                Save copy
              </button>
            </div>
          </div>
        </ModalDialog>
      )}
    </ModalDialog>
  );
}

function message(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason);
}
