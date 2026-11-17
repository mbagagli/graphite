import type { PdfEdit, PdfPageContents } from "./pdfEditingTypes";
import type { PdfWorkerRequest } from "./pdfEditing.worker";

/** Run PDF mutations off the interface thread and release them with the editor. */
export class PdfEditorClient {
  private readonly worker = new Worker(
    new URL("./pdfEditing.worker.ts", import.meta.url),
    { type: "module" },
  );
  private nextId = 0;
  private pending = new Map<
    number,
    {
      resolve: (value: PdfPageContents | Uint8Array<ArrayBuffer>) => void;
      reject: (reason: Error) => void;
    }
  >();

  constructor() {
    this.worker.onmessage = (
      event: MessageEvent<{
        id: number;
        result: PdfPageContents | Uint8Array<ArrayBuffer>;
        error?: string;
      }>,
    ) => {
      const response = event.data;
      const task = this.pending.get(response.id);
      if (!task) return;
      this.pending.delete(response.id);
      if (response.error) task.reject(new Error(response.error));
      else task.resolve(response.result);
    };
    this.worker.onerror = () =>
      this.dispose(
        "The PDF editing worker stopped. Reopen the editor to retry.",
      );
  }

  inspect(
    bytes: Uint8Array<ArrayBuffer>,
    page: number,
  ): Promise<PdfPageContents> {
    return this.request({
      id: ++this.nextId,
      kind: "inspect",
      bytes: bytes.slice(),
      page,
    }) as Promise<PdfPageContents>;
  }

  edit(
    bytes: Uint8Array<ArrayBuffer>,
    change: PdfEdit,
  ): Promise<Uint8Array<ArrayBuffer>> {
    return this.request({
      id: ++this.nextId,
      kind: "edit",
      bytes: bytes.slice(),
      change,
    }) as Promise<Uint8Array<ArrayBuffer>>;
  }

  dispose(message = "PDF editing was closed.") {
    this.worker.terminate();
    for (const task of this.pending.values()) task.reject(new Error(message));
    this.pending.clear();
  }

  private request(request: PdfWorkerRequest) {
    return new Promise<PdfPageContents | Uint8Array<ArrayBuffer>>(
      (resolve, reject) => {
        this.pending.set(request.id, { resolve, reject });
        this.worker.postMessage(request, [request.bytes.buffer]);
      },
    );
  }
}
