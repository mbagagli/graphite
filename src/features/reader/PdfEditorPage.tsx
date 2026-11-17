import {
  TextLayer,
  type PDFDocumentProxy,
  type PDFPageProxy,
  type PageViewport,
} from "pdfjs-dist";
import { useEffect, useRef, useState } from "react";
import { PdfPageCanvas } from "./PdfPageCanvas";
import type {
  PdfEdit,
  PdfPageContents,
  PdfPoint,
  PdfTextObject,
} from "./pdfEditingTypes";
import "pdfjs-dist/web/pdf_viewer.css";

export type PdfEditorTool =
  "select" | "highlight" | "note" | "drawing" | "text";

interface RenderedPage {
  pdf: PDFDocumentProxy;
  page: PDFPageProxy;
  viewport: PageViewport;
}

export function PdfEditorPage({
  pdf,
  pageNumber,
  width,
  tool,
  contents,
  disabled,
  onEdit,
  onText,
  onNote,
  onError,
}: {
  pdf: PDFDocumentProxy;
  pageNumber: number;
  width: number;
  tool: PdfEditorTool;
  contents: PdfPageContents | null;
  disabled: boolean;
  onEdit: (change: PdfEdit) => void;
  onText: (text: PdfTextObject) => void;
  onNote: (point: PdfPoint) => void;
  onError: (message: string) => void;
}) {
  const [rendered, setRendered] = useState<RenderedPage | null>(null);
  const textHost = useRef<HTMLDivElement>(null);
  const stroke = useRef<PdfPoint[] | null>(null);
  const [strokePreview, setStrokePreview] = useState<PdfPoint[]>([]);
  const ready =
    !disabled &&
    rendered?.pdf === pdf &&
    rendered.page.pageNumber === pageNumber;

  useEffect(() => {
    const host = textHost.current;
    if (!host || !rendered) return;
    let active = true;
    let layer: TextLayer | undefined;
    const container = document.createElement("div");
    container.className = "textLayer";
    container.style.setProperty(
      "--total-scale-factor",
      String(rendered.viewport.scale),
    );
    container.style.setProperty(
      "--scale-factor",
      String(rendered.viewport.scale),
    );
    rendered.page
      .getTextContent()
      .then(async (textContentSource) => {
        if (!active) return;
        layer = new TextLayer({
          textContentSource,
          container,
          viewport: rendered.viewport,
        });
        await layer.render();
        if (active) host.replaceChildren(container);
      })
      .catch((error: unknown) => {
        if (active)
          onError(error instanceof Error ? error.message : String(error));
      });
    return () => {
      active = false;
      layer?.cancel();
    };
  }, [rendered, onError]);

  function pdfPoint(clientX: number, clientY: number, box: DOMRect): PdfPoint {
    const point = rendered!.viewport.convertToPdfPoint(
      clientX - box.left,
      clientY - box.top,
    );
    return { x: point[0], y: point[1] };
  }

  return (
    <div className={`pdf-editor-page tool-${tool}`} aria-busy={!ready}>
      <PdfPageCanvas
        pdf={pdf}
        pageNumber={pageNumber}
        targetWidth={width}
        onRendered={setRendered}
        onRenderError={onError}
      />
      <div
        ref={textHost}
        className="pdf-editor-text-layer"
        style={{
          visibility:
            ready && (tool === "select" || tool === "highlight")
              ? "visible"
              : "hidden",
        }}
        onPointerUp={() => {
          if (tool !== "highlight" || !ready || !textHost.current) return;
          const selection = window.getSelection();
          if (
            !selection?.rangeCount ||
            selection.isCollapsed ||
            !textHost.current.contains(selection.anchorNode) ||
            !textHost.current.contains(selection.focusNode)
          )
            return;
          const box = textHost.current.getBoundingClientRect();
          const seen = new Set<string>();
          const rects = [...selection.getRangeAt(0).getClientRects()]
            .filter((rect) => rect.width > 1 && rect.height > 1)
            .map((rect) => {
              const first = pdfPoint(rect.left, rect.top, box);
              const second = pdfPoint(rect.right, rect.bottom, box);
              return {
                left: Math.min(first.x, second.x),
                right: Math.max(first.x, second.x),
                bottom: Math.min(first.y, second.y),
                top: Math.max(first.y, second.y),
              };
            })
            .filter((rect) => {
              const key = JSON.stringify(rect);
              if (seen.has(key)) return false;
              seen.add(key);
              return true;
            });
          selection.removeAllRanges();
          if (rects.length)
            onEdit({ kind: "highlight", page: pageNumber - 1, rects });
        }}
      />
      {ready && (tool === "note" || tool === "drawing") && (
        <div
          className="pdf-editor-drawing-layer"
          aria-label={
            tool === "note"
              ? "Click the page to place a note"
              : "Draw on the page"
          }
          onPointerDown={(event) => {
            if (event.button !== 0) return;
            event.preventDefault();
            const box = event.currentTarget.getBoundingClientRect();
            if (tool === "note") {
              onNote(pdfPoint(event.clientX, event.clientY, box));
              return;
            }
            event.currentTarget.setPointerCapture(event.pointerId);
            stroke.current = [
              { x: event.clientX - box.left, y: event.clientY - box.top },
            ];
            setStrokePreview(stroke.current);
          }}
          onPointerMove={(event) => {
            if (!stroke.current || stroke.current.length >= 20_000) return;
            const box = event.currentTarget.getBoundingClientRect();
            stroke.current.push({
              x: event.clientX - box.left,
              y: event.clientY - box.top,
            });
            setStrokePreview([...stroke.current]);
          }}
          onPointerUp={(event) => {
            const points = stroke.current;
            stroke.current = null;
            setStrokePreview([]);
            if (event.currentTarget.hasPointerCapture(event.pointerId))
              event.currentTarget.releasePointerCapture(event.pointerId);
            if (!points || points.length < 2) return;
            onEdit({
              kind: "drawing",
              page: pageNumber - 1,
              points: points.map((point) => {
                const [x, y] = rendered!.viewport.convertToPdfPoint(
                  point.x,
                  point.y,
                );
                return { x, y };
              }),
            });
          }}
          onPointerCancel={() => {
            stroke.current = null;
            setStrokePreview([]);
          }}
        >
          <svg width="100%" height="100%" aria-hidden="true">
            <polyline
              points={strokePreview
                .map((point) => `${point.x},${point.y}`)
                .join(" ")}
              fill="none"
              stroke="#e0ad3a"
              strokeWidth={2}
            />
          </svg>
        </div>
      )}
      {ready &&
        tool === "text" &&
        contents?.text.map((text) => {
          const rect = [
            ...rendered!.viewport.convertToViewportPoint(
              text.bounds.left,
              text.bounds.bottom,
            ),
            ...rendered!.viewport.convertToViewportPoint(
              text.bounds.right,
              text.bounds.top,
            ),
          ];
          return (
            <button
              key={text.index}
              type="button"
              className="pdf-text-target"
              aria-label={`Edit text: ${text.text}`}
              title={text.text}
              style={{
                left: Math.min(rect[0], rect[2]),
                top: Math.min(rect[1], rect[3]),
                width: Math.max(8, Math.abs(rect[2] - rect[0])),
                height: Math.max(10, Math.abs(rect[3] - rect[1])),
              }}
              onClick={() => onText(text)}
            />
          );
        })}
    </div>
  );
}
