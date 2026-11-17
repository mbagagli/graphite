import { PdfEditingEngine } from "./PdfEditingEngine";
import type { PdfEdit } from "./pdfEditingTypes";

export type PdfWorkerRequest =
  | {
      id: number;
      kind: "inspect";
      bytes: Uint8Array<ArrayBuffer>;
      page: number;
    }
  | {
      id: number;
      kind: "edit";
      bytes: Uint8Array<ArrayBuffer>;
      change: PdfEdit;
    };

const engine = Promise.all([
  fetch(new URL("@embedpdf/pdfium/pdfium.wasm", import.meta.url)).then(
    (response) => {
      if (!response.ok)
        throw new Error("The local PDF editing engine could not be loaded.");
      return response.arrayBuffer();
    },
  ),
  fetch(
    new URL("../../assets/fonts/NotoSans-Regular.ttf", import.meta.url),
  ).then(async (response) => {
    if (!response.ok)
      throw new Error("The bundled PDF editing font could not be loaded.");
    return new Uint8Array(await response.arrayBuffer());
  }),
]).then(([wasm, font]) => PdfEditingEngine.create(wasm, font));

self.onmessage = async (event: MessageEvent<PdfWorkerRequest>) => {
  const request = event.data;
  try {
    const ready = await engine;
    if (request.kind === "inspect") {
      self.postMessage({
        id: request.id,
        result: ready.inspect(request.bytes, request.page),
      });
    } else {
      const result = ready.edit(request.bytes, request.change);
      self.postMessage(
        { id: request.id, result },
        { transfer: [result.buffer] },
      );
    }
  } catch (error) {
    self.postMessage({
      id: request.id,
      error: error instanceof Error ? error.message : String(error),
    });
  }
};
