import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { services } from "../../services/runtime";
import type { DemoProgress } from "../demo/demoSetup";
import type {
  PreparationDocument,
  PreparationOperations,
} from "../../features/library/libraryPreparation";

export interface DemoArchiveInfo {
  manifestId: string;
  fileName: string;
  url: string | null;
  sha256: string;
  bytes: number;
}

export const getDemoArchiveInfo = () =>
  invoke<DemoArchiveInfo>("get_demo_archive_info");

const MAX_PDF_PAGES = 10_000;
const MAX_PAGE_TEXT_BYTES = 1024 * 1024;
const MAX_DOCUMENT_TEXT_BYTES = 10 * MAX_PAGE_TEXT_BYTES;

/** Read bounded page text with the bundled PDF worker and release its memory. */
export async function extractPdfPages(
  url: string,
  signal: AbortSignal,
): Promise<string[]> {
  const { getDocument, GlobalWorkerOptions } = await import("pdfjs-dist");
  signal.throwIfAborted();
  GlobalWorkerOptions.workerSrc = new URL(
    "pdfjs-dist/build/pdf.worker.mjs",
    import.meta.url,
  ).toString();
  const loading = getDocument({ url });
  const abort = () => {
    void loading.destroy();
  };
  signal.addEventListener("abort", abort, { once: true });
  try {
    const pdf = await loading.promise;
    if (pdf.numPages > MAX_PDF_PAGES)
      throw new Error("This PDF exceeds the 10,000-page preparation limit.");
    const pages: string[] = [];
    let bytes = 0;
    for (let number = 1; number <= pdf.numPages; number++) {
      signal.throwIfAborted();
      const page = await pdf.getPage(number);
      try {
        const content = await page.getTextContent();
        const text = content.items
          .map((item) =>
            "str" in item ? `${item.str}${item.hasEOL ? "\n" : " "}` : "",
          )
          .join("");
        const size = new TextEncoder().encode(text).length;
        bytes += size;
        if (size > MAX_PAGE_TEXT_BYTES || bytes > MAX_DOCUMENT_TEXT_BYTES) {
          throw new Error(
            "This PDF exceeds the text preparation limit. Its original is unchanged.",
          );
        }
        pages.push(text);
      } finally {
        page.cleanup();
      }
    }
    signal.throwIfAborted();
    if (pages.every((text) => !text.trim())) {
      throw new Error(
        "No selectable text. This PDF may need OCR before it can provide evidence.",
      );
    }
    return pages;
  } finally {
    signal.removeEventListener("abort", abort);
    await loading.destroy();
  }
}

export const preparationOperations: PreparationOperations = {
  async acquireDemo(offline, signal, progress) {
    const stop = await listen<DemoProgress>(
      "graphite-demo-progress",
      ({ payload }) => {
        const downloading = payload.phase === "downloading-archive";
        progress(
          downloading ? payload.bytes : payload.completed,
          downloading ? payload.totalBytes : payload.total,
          downloading,
        );
      },
    );
    try {
      signal.throwIfAborted();
      const result = await invoke(
        offline ? "import_demo_pdfs" : "download_demo_pdfs",
      );
      signal.throwIfAborted();
      return result !== null;
    } finally {
      stop();
    }
  },
  sync: () => services.workspace.sync(),
  refresh: () => services.workspace.load(true),
  documents: (libraryId) =>
    invoke<PreparationDocument[]>("get_library_preparation_documents", {
      libraryId,
    }),
  async extract(document, signal) {
    const documentId = document.documentId!;
    const resolved = await services.document.resolve(documentId);
    if (resolved.status !== "ready" || !resolved.url || !resolved.revisionKey) {
      throw new Error(
        resolved.message ??
          "PDF unavailable. Download its attachment in Zotero, then retry.",
      );
    }
    const pages = await extractPdfPages(resolved.url, signal);
    signal.throwIfAborted();
    await invoke("store_document_pdf_pages", {
      documentId,
      revisionKey: resolved.revisionKey,
      pages,
    });
  },
  indexCitations: () => services.citations.index(undefined, 48),
  resolveCitations: () => services.citations.reresolve(),
  indexSemantic: () => services.semantic.index(),
};

export async function cancelLibraryPreparation(): Promise<void> {
  await Promise.all([
    invoke("cancel_demo_preparation"),
    services.semantic.cancel(),
  ]);
}
