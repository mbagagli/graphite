import type { PDFDocumentProxy, PDFPageProxy, PageViewport } from "pdfjs-dist";
import { useEffect, useRef, useState } from "react";

interface PdfPageCanvasProps {
  pdf: PDFDocumentProxy;
  pageNumber: number;
  scale?: number;
  targetWidth?: number;
  maxPixelRatio?: number;
  className?: string;
  ariaLabel?: string;
  onRenderError?: (message: string) => void;
  onRendered?: (value: {
    pdf: PDFDocumentProxy;
    page: PDFPageProxy;
    viewport: PageViewport;
  }) => void;
}

export function PdfPageCanvas({
  pdf,
  pageNumber,
  scale = 1,
  targetWidth,
  maxPixelRatio = 2,
  className,
  ariaLabel,
  onRenderError,
  onRendered,
}: PdfPageCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const renderRevision = useRef(0);
  const errorCallback = useRef(onRenderError);
  const renderedCallback = useRef(onRendered);
  const [rendered, setRendered] = useState<{
    pdf: PDFDocumentProxy;
    pageNumber: number;
    scale: number;
    targetWidth?: number;
    maxPixelRatio: number;
  } | null>(null);
  const busy =
    rendered?.pdf !== pdf ||
    rendered?.pageNumber !== pageNumber ||
    rendered?.scale !== scale ||
    rendered?.targetWidth !== targetWidth ||
    rendered?.maxPixelRatio !== maxPixelRatio;

  useEffect(() => {
    errorCallback.current = onRenderError;
    renderedCallback.current = onRendered;
  }, [onRenderError, onRendered]);

  useEffect(() => {
    const revision = ++renderRevision.current;
    let renderTask: { cancel: () => void; promise: Promise<unknown> } | null =
      null;
    pdf
      .getPage(pageNumber)
      .then(async (page) => {
        if (revision !== renderRevision.current || !canvasRef.current) return;
        const naturalViewport = page.getViewport({ scale: 1 });
        const renderScale = targetWidth
          ? targetWidth / naturalViewport.width
          : scale;
        const viewport = page.getViewport({ scale: renderScale });
        // Paint offscreen so page changes and zoom never clear the visible page.
        const canvas = document.createElement("canvas");
        const ratio = Math.min(window.devicePixelRatio || 1, maxPixelRatio);
        canvas.width = Math.floor(viewport.width * ratio);
        canvas.height = Math.floor(viewport.height * ratio);
        const context = canvas.getContext("2d");
        if (!context) throw new Error("The PDF canvas is unavailable.");
        renderTask = page.render({
          canvas,
          canvasContext: context,
          viewport,
          transform: ratio === 1 ? undefined : [ratio, 0, 0, ratio, 0, 0],
        });
        await renderTask.promise;
        const visible = canvasRef.current;
        if (revision !== renderRevision.current || !visible) return;
        const visibleContext = visible.getContext("2d");
        if (!visibleContext) throw new Error("The PDF canvas is unavailable.");
        visible.width = canvas.width;
        visible.height = canvas.height;
        visible.style.width = `${viewport.width}px`;
        visible.style.height = `${viewport.height}px`;
        visibleContext.drawImage(canvas, 0, 0);
        visible.dataset.pageNumber = String(pageNumber);
        setRendered({ pdf, pageNumber, scale, targetWidth, maxPixelRatio });
        renderedCallback.current?.({ pdf, page, viewport });
      })
      .catch((reason: unknown) => {
        const message = errorMessage(reason);
        if (
          revision === renderRevision.current &&
          !message.toLowerCase().includes("cancel")
        ) {
          errorCallback.current?.(message);
        }
      });
    return () => {
      renderRevision.current += 1;
      renderTask?.cancel();
    };
  }, [maxPixelRatio, pageNumber, pdf, scale, targetWidth]);

  return (
    <canvas
      ref={canvasRef}
      className={className}
      aria-busy={busy}
      aria-label={
        busy ? "Loading PDF page" : (ariaLabel ?? `PDF page ${pageNumber}`)
      }
    />
  );
}

interface PdfThumbnailProps {
  pdf: PDFDocumentProxy;
  pageNumber: number;
  active: boolean;
  onSelect: (pageNumber: number) => void;
}

export function PdfThumbnail({
  pdf,
  pageNumber,
  active,
  onSelect,
}: PdfThumbnailProps) {
  const previewRef = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const preview = previewRef.current;
    if (!preview || typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: "220px 0px" },
    );
    observer.observe(preview);
    return () => observer.disconnect();
  }, []);

  return (
    <button
      type="button"
      className={active ? "is-active" : ""}
      onClick={() => onSelect(pageNumber)}
      aria-label={`Open page ${pageNumber}`}
      aria-current={active ? "page" : undefined}
    >
      <span>{pageNumber}</span>
      <div className="thumbnail-preview" ref={previewRef}>
        {visible && !failed ? (
          <PdfPageCanvas
            pdf={pdf}
            pageNumber={pageNumber}
            targetWidth={108}
            maxPixelRatio={1.5}
            ariaLabel={`Preview of page ${pageNumber}`}
            onRenderError={() => setFailed(true)}
          />
        ) : failed ? (
          <small>Preview unavailable</small>
        ) : (
          <small>Loading preview</small>
        )}
      </div>
    </button>
  );
}

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}
