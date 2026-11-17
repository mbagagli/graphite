export const MAX_EDITABLE_PDF_BYTES = 128 * 1024 * 1024;

export interface PdfPoint {
  x: number;
  y: number;
}

export interface PdfRect {
  left: number;
  bottom: number;
  right: number;
  top: number;
}

export interface PdfTextObject {
  index: number;
  text: string;
  font: string;
  fontSize: number;
  bounds: PdfRect;
}

export interface PdfAnnotation {
  index: number;
  kind: "highlight" | "note" | "drawing" | "other";
  text: string;
  bounds: PdfRect;
}

export interface PdfPageContents {
  text: PdfTextObject[];
  annotations: PdfAnnotation[];
  hasGroupedText: boolean;
}

export type PdfEdit =
  | {
      kind: "text";
      page: number;
      object: number;
      expectedText: string;
      text: string;
      font: "original" | "noto";
    }
  | { kind: "rotate"; page: number; turns: number }
  | { kind: "delete-page"; page: number }
  | { kind: "move-page"; page: number; destination: number }
  | { kind: "insert-blank"; page: number }
  | { kind: "insert-pdf"; page: number; bytes: Uint8Array<ArrayBuffer> }
  | { kind: "highlight"; page: number; rects: PdfRect[] }
  | { kind: "note"; page: number; point: PdfPoint; text: string }
  | { kind: "drawing"; page: number; points: PdfPoint[] }
  | { kind: "remove-annotation"; page: number; annotation: number }
  | {
      kind: "edit-note";
      page: number;
      annotation: number;
      text: string;
    };
