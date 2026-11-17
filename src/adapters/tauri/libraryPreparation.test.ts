import { beforeEach, expect, it, vi } from "vitest";
import { extractPdfPages } from "./libraryPreparation";

const mocks = vi.hoisted(() => ({
  getDocument: vi.fn(),
  destroy: vi.fn(),
  cleanup: vi.fn(),
}));
vi.mock("../../services/runtime", () => ({ services: {} }));
vi.mock("pdfjs-dist", () => ({
  getDocument: mocks.getDocument,
  GlobalWorkerOptions: { workerSrc: "" },
}));

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getDocument.mockReturnValue({
    destroy: mocks.destroy,
    promise: Promise.resolve({
      numPages: 2,
      getPage: async (number: number) => ({
        cleanup: mocks.cleanup,
        getTextContent: async () => ({
          items: [
            { str: `Page ${number}`, hasEOL: true },
            { str: "Body", hasEOL: false },
          ],
        }),
      }),
    }),
  });
});

it("preserves page and line boundaries for evidence and citations", async () => {
  const pages = await extractPdfPages(
    "graphite-document://pdf",
    new AbortController().signal,
  );
  expect(pages).toEqual(["Page 1\nBody ", "Page 2\nBody "]);
  expect(mocks.cleanup).toHaveBeenCalledTimes(2);
  expect(mocks.destroy).toHaveBeenCalledOnce();
});

it("does not return incomplete page text after cancellation", async () => {
  const controller = new AbortController();
  mocks.getDocument.mockReturnValue({
    destroy: mocks.destroy,
    promise: Promise.resolve({
      numPages: 2,
      getPage: async () => ({
        cleanup: mocks.cleanup,
        getTextContent: async () => {
          controller.abort();
          return { items: [{ str: "Incomplete", hasEOL: false }] };
        },
      }),
    }),
  });
  await expect(
    extractPdfPages("graphite-document://pdf", controller.signal),
  ).rejects.toThrow();
  expect(mocks.cleanup).toHaveBeenCalledOnce();
  expect(mocks.destroy).toHaveBeenCalled();
});

it("identifies scans needing OCR without reporting them as prepared", async () => {
  mocks.getDocument.mockReturnValue({
    destroy: mocks.destroy,
    promise: Promise.resolve({
      numPages: 1,
      getPage: async () => ({
        cleanup: mocks.cleanup,
        getTextContent: async () => ({ items: [] }),
      }),
    }),
  });
  await expect(
    extractPdfPages("graphite-document://scan", new AbortController().signal),
  ).rejects.toThrow("OCR");
  expect(mocks.destroy).toHaveBeenCalledOnce();
});

it("bounds extracted text rather than accumulating oversized PDFs", async () => {
  mocks.getDocument.mockReturnValue({
    destroy: mocks.destroy,
    promise: Promise.resolve({
      numPages: 1,
      getPage: async () => ({
        cleanup: mocks.cleanup,
        getTextContent: async () => ({
          items: [{ str: "a".repeat(1024 * 1024 + 1), hasEOL: false }],
        }),
      }),
    }),
  });
  await expect(
    extractPdfPages("graphite-document://large", new AbortController().signal),
  ).rejects.toThrow("limit");
  expect(mocks.cleanup).toHaveBeenCalledOnce();
});
