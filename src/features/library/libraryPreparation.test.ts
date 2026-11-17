import { describe, expect, it, vi } from "vitest";
import { browserWorkspace } from "../../adapters/demo/browserDiagnostics";
import type { WorkspacePayload } from "../../services/contracts";
import {
  prepareLibrary,
  type PreparationOperations,
  type PreparationProgress,
} from "./libraryPreparation";

function setup() {
  const data: WorkspacePayload = structuredClone(browserWorkspace);
  data.source.activeLibraryId = "library";
  data.source.lastError = null;
  data.corpus.lastError = null;
  Object.assign(data.citations, { queuedDocuments: 0, queuedResolutions: 0 });
  Object.assign(data.semantic, {
    modelInstalled: true,
    indexedPapers: 1,
    eligiblePapers: 1,
    chunkedDocuments: 1,
    eligibleDocuments: 1,
    indexedChunks: 2,
    stalePapers: 0,
    staleChunks: 0,
  });
  const operations = {
    acquireDemo: vi.fn(async () => true),
    sync: vi.fn(async () => structuredClone(data)),
    refresh: vi.fn(async () => structuredClone(data)),
    documents: vi.fn(async () => [
      {
        itemId: "paper",
        title: "Paper",
        documentId: "pdf",
        hasUsableText: false,
        mediaType: "application/pdf",
      },
    ]),
    extract: vi.fn(async () => {}),
    indexCitations: vi.fn(async () => structuredClone(data)),
    resolveCitations: vi.fn(async () => structuredClone(data)),
    indexSemantic: vi.fn(async () => structuredClone(data)),
  } satisfies PreparationOperations;
  const reports: PreparationProgress[] = [];
  const update = vi.fn();
  const controller = new AbortController();
  const run = (mode: "demo" | "zotero" = "demo", offline = false) =>
    prepareLibrary(
      mode,
      "library",
      offline,
      operations,
      controller.signal,
      (value) => reports.push(value),
      update,
    );
  return { data, operations, reports, controller, run, update };
}

describe("complete library preparation", () => {
  it("acquires, extracts and indexes without installing a model", async () => {
    const { operations, reports, run } = setup();
    await run();
    expect(operations.extract).toHaveBeenCalledOnce();
    expect(operations.indexSemantic).toHaveBeenCalledOnce();
    expect(reports.at(-1)?.status).toBe("complete");
    expect(operations.extract.mock.invocationCallOrder[0]).toBeLessThan(
      operations.indexSemantic.mock.invocationCallOrder[0],
    );
  });
  it("uses local ZIP selection through the same flow", async () => {
    const { operations, run } = setup();
    await run("demo", true);
    expect(operations.acquireDemo).toHaveBeenCalledWith(
      true,
      expect.any(AbortSignal),
      expect.any(Function),
    );
  });
  it("does nothing after cancelling the archive picker", async () => {
    const { operations, reports, run } = setup();
    operations.acquireDemo.mockResolvedValue(false);
    await run();
    expect(operations.extract).not.toHaveBeenCalled();
    expect(reports.at(-1)?.status).toBe("cancelled");
  });
  it("keeps text and citations when the embedding model is missing", async () => {
    const { data, operations, reports, run } = setup();
    data.semantic.modelInstalled = false;
    data.citations.queuedDocuments = 1;
    operations.indexCitations.mockImplementation(async () => {
      data.citations.queuedDocuments = 0;
      return structuredClone(data);
    });
    await run();
    expect(operations.extract).toHaveBeenCalledOnce();
    expect(operations.indexCitations).toHaveBeenCalledOnce();
    expect(operations.indexSemantic).not.toHaveBeenCalled();
    expect(reports.at(-1)?.status).toBe("needs-model");
  });
  it("drains citation and semantic batches including changed eligibility", async () => {
    const { data, operations, reports, run } = setup();
    data.citations.queuedDocuments = 2;
    operations.indexCitations.mockImplementation(async () => {
      data.citations.queuedDocuments--;
      data.citations.queuedResolutions += 4;
      return structuredClone(data);
    });
    operations.resolveCitations.mockImplementation(async () => {
      data.citations.queuedResolutions = 0;
      return structuredClone(data);
    });
    operations.indexSemantic
      .mockImplementationOnce(async () => ({
        ...data,
        semantic: { ...data.semantic, eligibleDocuments: 3 },
      }))
      .mockImplementationOnce(async () => ({
        ...data,
        semantic: {
          ...data.semantic,
          eligibleDocuments: 3,
          chunkedDocuments: 2,
        },
      }))
      .mockImplementationOnce(async () => ({
        ...data,
        semantic: {
          ...data.semantic,
          eligibleDocuments: 3,
          chunkedDocuments: 3,
        },
      }));
    await run();
    expect(operations.indexCitations).toHaveBeenCalledTimes(2);
    expect(operations.resolveCitations).toHaveBeenCalledOnce();
    expect(operations.indexSemantic).toHaveBeenCalledTimes(3);
    expect(reports.at(-1)?.status).toBe("complete");
  });
  it("stops a non-progressing index rather than looping or declaring success", async () => {
    const { data, operations, reports, run } = setup();
    data.semantic.eligibleDocuments = 2;
    await run();
    expect(operations.indexSemantic).toHaveBeenCalledOnce();
    expect(reports.at(-1)?.status).toBe("error");
    expect(reports.at(-1)?.detail).toContain("no progress");
  });
  it("continues other papers after a missing attachment or scan", async () => {
    const { operations, reports, run } = setup();
    const document = (await operations.documents())[0];
    operations.documents.mockResolvedValue([
      document,
      { ...document, itemId: "scan", title: "Scan" },
    ]);
    operations.extract.mockRejectedValueOnce(new Error("Needs OCR"));
    await run("zotero");
    expect(operations.acquireDemo).not.toHaveBeenCalled();
    expect(operations.extract).toHaveBeenCalledTimes(2);
    expect(reports.at(-1)?.status).toBe("partial");
    expect(reports.at(-1)?.issues).toEqual([
      { itemId: "paper", title: "Paper", message: "Needs OCR" },
    ]);
  });
  it("uses cached documents after Zotero is disconnected", async () => {
    const { operations, reports, run } = setup();
    operations.sync.mockRejectedValue(new Error("Zotero unavailable"));
    await run("zotero");
    expect(operations.extract).toHaveBeenCalledOnce();
    expect(reports.at(-1)?.status).toBe("partial");
    expect(reports.at(-1)?.issues[0].message).toContain("cached");
  });
  it("retains usable text on repeat preparation", async () => {
    const { operations, run } = setup();
    const document = (await operations.documents())[0];
    operations.documents.mockResolvedValue([
      { ...document, hasUsableText: true },
    ]);
    await run();
    expect(operations.extract).not.toHaveBeenCalled();
  });
  it("cancels before further batches and never reports completion", async () => {
    const { operations, reports, controller, run, update } = setup();
    operations.extract.mockImplementation(async () => controller.abort());
    await run();
    expect(operations.indexSemantic).not.toHaveBeenCalled();
    expect(reports.at(-1)?.status).toBe("cancelled");
    expect(update).toHaveBeenCalledOnce();
  });
  it("rejects source changes and zero-passage readiness", async () => {
    const first = setup();
    first.data.source.activeLibraryId = "other";
    await first.run();
    expect(first.reports.at(-1)?.status).toBe("error");
    const second = setup();
    second.data.semantic.indexedChunks = 0;
    await second.run();
    expect(second.reports.at(-1)?.status).toBe("partial");
    expect(second.reports.at(-1)?.detail).toContain(
      "No usable document passages",
    );
  });
  it("does not request a model for an empty document library", async () => {
    const { data, operations, reports, run } = setup();
    operations.documents.mockResolvedValue([]);
    data.semantic.modelInstalled = false;
    await run("zotero");
    expect(reports.at(-1)?.status).toBe("partial");
    expect(reports.at(-1)?.detail).toContain("No usable document text");
    expect(operations.indexSemantic).not.toHaveBeenCalled();
  });
});
