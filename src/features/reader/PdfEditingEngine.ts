import { init, type WrappedPdfiumModule } from "@embedpdf/pdfium";
import {
  MAX_EDITABLE_PDF_BYTES,
  type PdfAnnotation,
  type PdfEdit,
  type PdfPageContents,
  type PdfRect,
  type PdfTextObject,
} from "./pdfEditingTypes";

const PDF_TEXT_OBJECT = 1;
const PDF_FORM_OBJECT = 5;
const ANNOTATION_NOTE = 1;
const ANNOTATION_HIGHLIGHT = 9;
const ANNOTATION_INK = 15;
const PDF_FONT_TRUETYPE = 2;
const SAVE_WITHOUT_INCREMENTAL_HISTORY = 2;

/** Edit disposable copies; a failed operation never changes the caller's PDF. */
export class PdfEditingEngine {
  private constructor(
    private readonly api: WrappedPdfiumModule,
    private readonly font: Uint8Array<ArrayBuffer>,
  ) {}

  static async create(wasmBinary: ArrayBuffer, font: Uint8Array<ArrayBuffer>) {
    const api = await init({ wasmBinary });
    api.PDFiumExt_Init();
    return new PdfEditingEngine(api, font);
  }

  inspect(bytes: Uint8Array<ArrayBuffer>, page: number): PdfPageContents {
    return this.withDocument(bytes, (document) =>
      this.withPage(document, page, (handle) => {
        const textPage = this.api.FPDFText_LoadPage(handle);
        this.require(textPage, "Read this page's text");
        try {
          const text: PdfTextObject[] = [];
          let hasGroupedText = false;
          for (
            let index = 0;
            index < this.api.FPDFPage_CountObjects(handle);
            index++
          ) {
            const object = this.api.FPDFPage_GetObject(handle, index);
            const kind = this.api.FPDFPageObj_GetType(object);
            if (kind === PDF_FORM_OBJECT) hasGroupedText = true;
            if (kind !== PDF_TEXT_OBJECT) continue;
            const content = this.textOf(object, textPage);
            if (!content.trim()) continue;
            const bounds = this.memory(16, (ptr) => {
              this.require(
                this.api.FPDFPageObj_GetBounds(
                  object,
                  ptr,
                  ptr + 4,
                  ptr + 8,
                  ptr + 12,
                ),
                "Read the text bounds",
              );
              return this.readBounds(ptr, false);
            });
            const fontSize = this.memory(4, (ptr) => {
              this.require(
                this.api.FPDFTextObj_GetFontSize(object, ptr),
                "Read the font size",
              );
              return this.api.pdfium.getValue(ptr, "float");
            });
            const font = this.api.FPDFTextObj_GetFont(object);
            const nameSize = this.api.FPDFFont_GetBaseFontName(font, 0, 0);
            const name = this.memory(Math.max(1, nameSize), (ptr) => {
              this.api.FPDFFont_GetBaseFontName(font, ptr, nameSize);
              return this.api.pdfium.UTF8ToString(ptr);
            });
            text.push({ index, text: content, font: name, fontSize, bounds });
          }
          return {
            text,
            annotations: this.annotations(handle),
            hasGroupedText,
          };
        } finally {
          this.api.FPDFText_ClosePage(textPage);
        }
      }),
    );
  }

  edit(
    bytes: Uint8Array<ArrayBuffer>,
    change: PdfEdit,
  ): Uint8Array<ArrayBuffer> {
    return this.withDocument(bytes, (document) => {
      const count = this.api.FPDF_GetPageCount(document);
      this.pageIndex(change.page, count);
      if (change.kind === "delete-page") {
        if (count === 1) throw new Error("A PDF must keep at least one page.");
        this.api.FPDFPage_Delete(document, change.page);
      } else if (change.kind === "move-page") {
        this.pageIndex(change.destination, count);
        this.memory(4, (ptr) => {
          this.api.pdfium.setValue(ptr, change.page, "i32");
          this.require(
            this.api.FPDF_MovePages(document, ptr, 1, change.destination),
            "Move the page",
          );
        });
      } else if (change.kind === "insert-pdf") {
        this.withDocument(change.bytes, (source) => {
          this.require(
            this.api.FPDF_ImportPagesByIndex(
              document,
              source,
              0,
              0,
              change.page + 1,
            ),
            "Insert the PDF pages",
          );
        });
      } else {
        this.withPage(document, change.page, (page) => {
          if (change.kind === "rotate") {
            if (!Number.isInteger(change.turns))
              throw new Error("Rotation must use quarter turns.");
            const rotation =
              (this.api.FPDFPage_GetRotation(page) + change.turns) % 4;
            this.api.FPDFPage_SetRotation(page, (rotation + 4) % 4);
          } else if (change.kind === "insert-blank") {
            const blank = this.api.FPDFPage_New(
              document,
              change.page + 1,
              this.api.FPDF_GetPageWidth(page),
              this.api.FPDF_GetPageHeight(page),
            );
            this.require(blank, "Insert a blank page");
            this.api.FPDF_ClosePage(blank);
          } else if (change.kind === "text") {
            this.replaceText(document, page, change);
          } else if (change.kind === "remove-annotation") {
            this.annotationIndex(page, change.annotation);
            this.require(
              this.api.FPDFPage_RemoveAnnot(page, change.annotation),
              "Remove the annotation",
            );
          } else if (change.kind === "edit-note") {
            this.annotationIndex(page, change.annotation);
            const annotation = this.api.FPDFPage_GetAnnot(
              page,
              change.annotation,
            );
            try {
              this.setAnnotationText(annotation, change.text);
            } finally {
              this.api.FPDFPage_CloseAnnot(annotation);
            }
          } else {
            this.addAnnotation(page, change);
          }
          this.require(
            this.api.FPDFPage_GenerateContent(page),
            "Save the page content",
          );
        });
      }
      return this.save(document);
    });
  }

  private replaceText(
    document: number,
    page: number,
    change: Extract<PdfEdit, { kind: "text" }>,
  ) {
    if (
      !Number.isInteger(change.object) ||
      change.object < 0 ||
      change.object >= this.api.FPDFPage_CountObjects(page)
    ) {
      throw new Error("The selected text no longer exists. Select it again.");
    }
    const object = this.api.FPDFPage_GetObject(page, change.object);
    if (this.api.FPDFPageObj_GetType(object) !== PDF_TEXT_OBJECT) {
      throw new Error("Select an editable text run on this page.");
    }
    if (change.text.length > 10_000 || /[\r\n\0]/u.test(change.text)) {
      throw new Error(
        "Edit one line at a time, using at most 10,000 characters.",
      );
    }
    let textPage = this.api.FPDFText_LoadPage(page);
    try {
      if (this.textOf(object, textPage) !== change.expectedText) {
        throw new Error(
          "The selected text changed. Select it again before editing.",
        );
      }
    } finally {
      this.api.FPDFText_ClosePage(textPage);
    }
    if (!change.text) {
      this.require(
        this.api.FPDFPage_RemoveObject(page, object),
        "Remove the text",
      );
      this.api.FPDFPageObj_Destroy(object);
      return;
    }
    let editedObject = object;
    if (change.font === "noto") {
      editedObject = this.createNotoText(document, object);
      this.api.FPDFPage_InsertObject(page, editedObject);
      this.require(
        this.api.FPDFPage_RemoveObject(page, object),
        "Replace the original text",
      );
      this.api.FPDFPageObj_Destroy(object);
    }
    this.withText(change.text, (ptr) => {
      this.require(
        this.api.FPDFText_SetText(editedObject, ptr),
        "Set the replacement text",
      );
    });
    this.require(
      this.api.FPDFPage_GenerateContent(page),
      "Update the text content",
    );
    textPage = this.api.FPDFText_LoadPage(page);
    try {
      if (this.textOf(editedObject, textPage) !== change.text) {
        throw new Error(
          change.font === "original"
            ? "The embedded font cannot encode this edit. Choose Noto Sans to use the bundled font."
            : "The bundled font cannot encode all these characters. The PDF has not been changed.",
        );
      }
    } finally {
      this.api.FPDFText_ClosePage(textPage);
    }
  }

  private createNotoText(document: number, source: number): number {
    return this.withBytes(this.font, (fontPtr) => {
      const font = this.api.FPDFText_LoadFont(
        document,
        fontPtr,
        this.font.length,
        PDF_FONT_TRUETYPE,
        true,
      );
      this.require(font, "Load the bundled editing font");
      let object = 0;
      try {
        return this.memory(24, (ptr) => {
          this.require(
            this.api.FPDFTextObj_GetFontSize(source, ptr),
            "Read the font size",
          );
          object = this.api.FPDFPageObj_CreateTextObj(
            document,
            font,
            this.api.pdfium.getValue(ptr, "float"),
          );
          this.require(object, "Create the replacement text");
          this.require(
            this.api.FPDFPageObj_GetMatrix(source, ptr),
            "Read the text position",
          );
          this.require(
            this.api.FPDFPageObj_SetMatrix(object, ptr),
            "Keep the text position",
          );
          this.copyTextColor(source, object);
          return object;
        });
      } catch (error) {
        if (object) this.api.FPDFPageObj_Destroy(object);
        throw error;
      } finally {
        this.api.FPDFFont_Close(font);
      }
    });
  }

  private copyTextColor(source: number, target: number) {
    this.memory(16, (ptr) => {
      this.require(
        this.api.FPDFPageObj_GetFillColor(
          source,
          ptr,
          ptr + 4,
          ptr + 8,
          ptr + 12,
        ),
        "Read the text color",
      );
      this.require(
        this.api.FPDFPageObj_SetFillColor(
          target,
          this.api.pdfium.getValue(ptr, "i32"),
          this.api.pdfium.getValue(ptr + 4, "i32"),
          this.api.pdfium.getValue(ptr + 8, "i32"),
          this.api.pdfium.getValue(ptr + 12, "i32"),
        ),
        "Keep the text color",
      );
    });
  }

  private addAnnotation(
    page: number,
    change: Extract<PdfEdit, { kind: "highlight" | "note" | "drawing" }>,
  ) {
    const kind =
      change.kind === "highlight"
        ? ANNOTATION_HIGHLIGHT
        : change.kind === "note"
          ? ANNOTATION_NOTE
          : ANNOTATION_INK;
    const annotation = this.api.FPDFPage_CreateAnnot(page, kind);
    this.require(annotation, "Create the annotation");
    try {
      this.require(
        this.api.FPDFAnnot_SetColor(annotation, 0, 224, 173, 58, 200),
        "Set the annotation color",
      );
      this.require(
        this.api.FPDFAnnot_SetFlags(annotation, 4),
        "Make the annotation printable",
      );
      if (change.kind === "highlight") {
        if (!change.rects.length || change.rects.length > 1000)
          throw new Error("Select some text on this page to highlight.");
        for (const rect of change.rects) {
          this.floats(
            [
              rect.left,
              rect.top,
              rect.right,
              rect.top,
              rect.left,
              rect.bottom,
              rect.right,
              rect.bottom,
            ],
            (ptr) => {
              this.require(
                this.api.FPDFAnnot_AppendAttachmentPoints(annotation, ptr),
                "Highlight the selected text",
              );
            },
          );
        }
        this.setAnnotationRect(annotation, {
          left: Math.min(...change.rects.map((r) => r.left)),
          right: Math.max(...change.rects.map((r) => r.right)),
          top: Math.max(...change.rects.map((r) => r.top)),
          bottom: Math.min(...change.rects.map((r) => r.bottom)),
        });
      } else if (change.kind === "note") {
        this.setAnnotationRect(annotation, {
          left: change.point.x,
          bottom: change.point.y - 20,
          right: change.point.x + 20,
          top: change.point.y,
        });
        this.setAnnotationText(annotation, change.text);
      } else {
        if (change.points.length < 2 || change.points.length > 20_000)
          throw new Error("Draw a stroke with 2 to 20,000 points.");
        this.floats(
          change.points.flatMap((p) => [p.x, p.y]),
          (ptr) => {
            if (
              this.api.FPDFAnnot_AddInkStroke(
                annotation,
                ptr,
                change.points.length,
              ) < 0
            )
              throw new Error("The drawing could not be saved.");
          },
        );
        this.require(
          this.api.FPDFAnnot_SetBorder(annotation, 0, 0, 1.5),
          "Set the drawing stroke",
        );
        this.setAnnotationRect(annotation, {
          left: Math.min(...change.points.map((p) => p.x)) - 2,
          right: Math.max(...change.points.map((p) => p.x)) + 2,
          bottom: Math.min(...change.points.map((p) => p.y)) - 2,
          top: Math.max(...change.points.map((p) => p.y)) + 2,
        });
      }
      this.api.EPDFAnnot_GenerateAppearance(annotation);
    } finally {
      this.api.FPDFPage_CloseAnnot(annotation);
    }
  }

  private annotations(page: number): PdfAnnotation[] {
    return Array.from(
      { length: this.api.FPDFPage_GetAnnotCount(page) },
      (_, index): PdfAnnotation | null => {
        const annotation = this.api.FPDFPage_GetAnnot(page, index);
        try {
          const subtype = this.api.FPDFAnnot_GetSubtype(annotation);
          if (
            ![ANNOTATION_NOTE, ANNOTATION_HIGHLIGHT, ANNOTATION_INK].includes(
              subtype,
            )
          ) {
            return null;
          }
          const size = this.api.FPDFAnnot_GetStringValue(
            annotation,
            "Contents",
            0,
            0,
          );
          const text = this.memory(Math.max(2, size), (ptr) => {
            this.api.FPDFAnnot_GetStringValue(
              annotation,
              "Contents",
              ptr,
              size,
            );
            return size ? this.api.pdfium.UTF16ToString(ptr) : "";
          });
          const bounds = this.memory(16, (ptr) => {
            this.require(
              this.api.FPDFAnnot_GetRect(annotation, ptr),
              "Read the annotation position",
            );
            return this.readBounds(ptr, true);
          });
          const kind =
            subtype === ANNOTATION_NOTE
              ? "note"
              : subtype === ANNOTATION_HIGHLIGHT
                ? "highlight"
                : subtype === ANNOTATION_INK
                  ? "drawing"
                  : "other";
          return { index, kind, text, bounds };
        } finally {
          this.api.FPDFPage_CloseAnnot(annotation);
        }
      },
    ).filter((annotation): annotation is PdfAnnotation => annotation !== null);
  }

  private setAnnotationText(annotation: number, text: string) {
    if (!text.trim() || text.length > 10_000 || text.includes("\0")) {
      throw new Error("Enter a note of 1 to 10,000 characters.");
    }
    this.withText(text, (ptr) => {
      this.require(
        this.api.FPDFAnnot_SetStringValue(annotation, "Contents", ptr),
        "Save the note",
      );
    });
  }

  private setAnnotationRect(annotation: number, rect: PdfRect) {
    this.floats([rect.left, rect.top, rect.right, rect.bottom], (ptr) => {
      this.require(
        this.api.FPDFAnnot_SetRect(annotation, ptr),
        "Position the annotation",
      );
    });
  }

  private readBounds(ptr: number, rectOrder: boolean): PdfRect {
    const value = (offset: number) =>
      this.api.pdfium.getValue(ptr + offset, "float");
    return {
      left: value(0),
      bottom: value(rectOrder ? 12 : 4),
      right: value(8),
      top: value(rectOrder ? 4 : 12),
    };
  }

  private textOf(object: number, textPage: number): string {
    const size = this.api.FPDFTextObj_GetText(object, textPage, 0, 0);
    if (!size) return "";
    return this.memory(size, (ptr) => {
      this.api.FPDFTextObj_GetText(object, textPage, ptr, size);
      return this.api.pdfium.UTF16ToString(ptr);
    });
  }

  private save(document: number): Uint8Array<ArrayBuffer> {
    const writer = this.api.PDFiumExt_OpenFileWriter();
    this.require(writer, "Prepare the edited PDF");
    try {
      this.require(
        this.api.FPDF_SaveAsCopy(
          document,
          writer,
          SAVE_WITHOUT_INCREMENTAL_HISTORY,
        ),
        "Save the edited PDF",
      );
      const size = this.api.PDFiumExt_GetFileWriterSize(writer);
      return this.memory(size, (ptr) => {
        this.api.PDFiumExt_GetFileWriterData(writer, ptr, size);
        return this.heap.slice(ptr, ptr + size);
      });
    } finally {
      this.api.PDFiumExt_CloseFileWriter(writer);
    }
  }

  private withDocument<T>(
    bytes: Uint8Array<ArrayBuffer>,
    action: (document: number) => T,
  ): T {
    if (bytes.length > MAX_EDITABLE_PDF_BYTES)
      throw new Error("PDF editing supports files up to 128 MB.");
    return this.withBytes(bytes, (ptr) => {
      const document = this.api.FPDF_LoadMemDocument(ptr, bytes.length, "");
      this.require(
        document,
        "Open this PDF for editing (encrypted files require an unencrypted copy)",
      );
      try {
        return action(document);
      } finally {
        this.api.FPDF_CloseDocument(document);
      }
    });
  }

  private withPage<T>(
    document: number,
    index: number,
    action: (page: number) => T,
  ): T {
    this.pageIndex(index, this.api.FPDF_GetPageCount(document));
    const page = this.api.FPDF_LoadPage(document, index);
    this.require(page, "Open the PDF page");
    try {
      return action(page);
    } finally {
      this.api.FPDF_ClosePage(page);
    }
  }

  private pageIndex(index: number, count: number) {
    if (!Number.isInteger(index) || index < 0 || index >= count)
      throw new Error("Choose a page in this PDF.");
  }

  private annotationIndex(page: number, index: number) {
    if (
      !Number.isInteger(index) ||
      index < 0 ||
      index >= this.api.FPDFPage_GetAnnotCount(page)
    ) {
      throw new Error("The annotation no longer exists. Select it again.");
    }
  }

  private withBytes<T>(
    bytes: Uint8Array<ArrayBuffer>,
    action: (ptr: number) => T,
  ): T {
    return this.memory(bytes.length, (ptr) => {
      this.heap.set(bytes, ptr);
      return action(ptr);
    });
  }

  private withText<T>(text: string, action: (ptr: number) => T): T {
    const size = (text.length + 1) * 2;
    return this.memory(size, (ptr) => {
      this.api.pdfium.stringToUTF16(text, ptr, size);
      return action(ptr);
    });
  }

  private floats<T>(values: number[], action: (ptr: number) => T): T {
    if (!values.every(Number.isFinite))
      throw new Error("PDF coordinates must be finite numbers.");
    return this.memory(values.length * 4, (ptr) => {
      values.forEach((value, index) =>
        this.api.pdfium.setValue(ptr + index * 4, value, "float"),
      );
      return action(ptr);
    });
  }

  private get heap(): Uint8Array<ArrayBuffer> {
    return (
      this.api.pdfium as typeof this.api.pdfium & {
        HEAPU8: Uint8Array<ArrayBuffer>;
      }
    ).HEAPU8;
  }

  private memory<T>(size: number, action: (ptr: number) => T): T {
    const ptr = this.api.pdfium.wasmExports.malloc(Math.max(1, size));
    this.require(ptr, "Allocate memory for this PDF");
    try {
      return action(ptr);
    } finally {
      this.api.pdfium.wasmExports.free(ptr);
    }
  }

  private require(value: number | boolean, operation: string): asserts value {
    if (!value)
      throw new Error(`${operation} failed. The PDF has not been changed.`);
  }
}
