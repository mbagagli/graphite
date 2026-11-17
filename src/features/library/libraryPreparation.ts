import type { WorkspacePayload } from "../../services/contracts";

export interface PreparationDocument {
  itemId: string;
  title: string;
  documentId: string | null;
  hasUsableText: boolean;
  mediaType: string | null;
}

export interface PreparationIssue {
  itemId: string;
  title: string;
  message: string;
}

export interface PreparationProgress {
  libraryId: string;
  status:
    "running" | "cancelled" | "needs-model" | "complete" | "partial" | "error";
  stage: "acquisition" | "text" | "citations" | "semantic" | "finished";
  detail: string;
  completed: number;
  total: number | null;
  unit: "bytes" | "documents" | "items";
  issues: PreparationIssue[];
}

export interface PreparationOperations {
  acquireDemo: (
    offline: boolean,
    signal: AbortSignal,
    progress: (completed: number, total: number, bytes: boolean) => void,
  ) => Promise<boolean>;
  sync: () => Promise<WorkspacePayload>;
  refresh: () => Promise<WorkspacePayload>;
  documents: (libraryId: string) => Promise<PreparationDocument[]>;
  extract: (
    document: PreparationDocument,
    signal: AbortSignal,
  ) => Promise<void>;
  indexCitations: () => Promise<WorkspacePayload>;
  resolveCitations: () => Promise<WorkspacePayload>;
  indexSemantic: () => Promise<WorkspacePayload>;
}

function pendingSemantic(data: WorkspacePayload): number {
  const s = data.semantic;
  return (
    Math.max(0, s.eligiblePapers - s.indexedPapers) +
    s.stalePapers +
    Math.max(0, s.eligibleDocuments - s.chunkedDocuments) +
    s.staleChunks
  );
}

/** Prepare available evidence in bounded batches, without installing models. */
export async function prepareLibrary(
  mode: "demo" | "zotero",
  libraryId: string,
  offline: boolean,
  operations: PreparationOperations,
  signal: AbortSignal,
  report: (progress: PreparationProgress) => void,
  update: (data: WorkspacePayload) => void,
): Promise<void> {
  let progress: PreparationProgress = {
    libraryId,
    status: "running",
    stage: "acquisition",
    detail: mode === "demo" ? "Preparing demo PDFs" : "Synchronizing Zotero",
    completed: 0,
    total: null,
    unit: "documents",
    issues: [],
  };
  const publish = (change: Partial<PreparationProgress>) => {
    progress = { ...progress, ...change };
    report(progress);
  };
  const accept = (data: WorkspacePayload) => {
    signal.throwIfAborted();
    if (data.source.activeLibraryId !== libraryId) {
      throw new Error(
        "The active library changed. Reopen library setup and retry.",
      );
    }
    update(data);
    return data;
  };
  publish({});
  try {
    signal.throwIfAborted();
    if (mode === "demo") {
      const acquired = await operations.acquireDemo(
        offline,
        signal,
        (completed, total, bytes) =>
          publish({
            detail: bytes ? "Downloading demo archive" : "Verifying demo PDFs",
            completed,
            total: total || null,
            unit: bytes ? "bytes" : "documents",
          }),
      );
      if (!acquired) {
        publish({
          status: "cancelled",
          detail: "No archive selected. Nothing changed.",
        });
        return;
      }
    } else {
      let synced: WorkspacePayload;
      try {
        synced = accept(await operations.sync());
      } catch (reason) {
        signal.throwIfAborted();
        synced = accept(await operations.refresh());
        publish({
          issues: [
            {
              itemId: "library",
              title: "Zotero synchronization",
              message: `${String(reason)} Available cached documents will still be prepared.`,
            },
          ],
        });
      }
      if (synced.source.lastError || synced.corpus.lastError) {
        publish({
          issues: [
            {
              itemId: "library",
              title: "Zotero synchronization",
              message: `${synced.source.lastError ?? synced.corpus.lastError} Available cached documents will still be prepared.`,
            },
          ],
        });
      }
    }
    const documents = await operations.documents(libraryId);
    signal.throwIfAborted();
    publish({
      stage: "text",
      detail: "Preparing document text",
      completed: 0,
      total: documents.length,
      unit: "documents",
    });
    let usable = 0;
    for (const [index, document] of documents.entries()) {
      signal.throwIfAborted();
      try {
        if (!document.hasUsableText) {
          if (
            !document.documentId ||
            document.mediaType !== "application/pdf"
          ) {
            throw new Error(
              "No usable PDF or text. Add or download the attachment in Zotero, then retry.",
            );
          }
          await operations.extract(document, signal);
        }
        usable++;
      } catch (reason) {
        signal.throwIfAborted();
        publish({
          issues: [
            ...progress.issues,
            {
              itemId: document.itemId,
              title: document.title,
              message:
                reason instanceof Error ? reason.message : String(reason),
            },
          ],
        });
      }
      publish({ completed: index + 1 });
    }
    let data = accept(await operations.refresh());
    publish({
      stage: "citations",
      detail: "Preparing citation connections",
      completed: 0,
      total: null,
      unit: "documents",
    });
    while (
      data.citations.queuedDocuments > 0 ||
      data.citations.queuedResolutions > 0
    ) {
      signal.throwIfAborted();
      const previous =
        data.citations.queuedDocuments + data.citations.queuedResolutions;
      const previousDocuments = data.citations.queuedDocuments;
      data = accept(
        await (previousDocuments > 0
          ? operations.indexCitations()
          : operations.resolveCitations()),
      );
      const remaining =
        data.citations.queuedDocuments + data.citations.queuedResolutions;
      if (
        remaining >= previous &&
        data.citations.queuedDocuments >= previousDocuments
      ) {
        throw new Error(
          data.citations.lastError ??
            "Citation preparation made no progress. Retry or inspect library diagnostics.",
        );
      }
      publish({
        completed: data.citations.indexedDocuments,
        total: data.citations.eligibleDocuments || null,
      });
    }
    if (usable === 0) {
      publish({
        status: "partial",
        stage: "finished",
        total: null,
        detail:
          "No usable document text yet. Add readable PDFs or review the items needing attention before preparing AI search.",
      });
      return;
    }
    if (!data.semantic.modelInstalled) {
      publish({
        status: "needs-model",
        stage: "finished",
        total: null,
        detail:
          "Text and citations prepared. Install the embedding model below, then resume preparation.",
      });
      return;
    }
    publish({
      stage: "semantic",
      detail: "Indexing papers and document passages",
      completed: 0,
      total: null,
      unit: "items",
    });
    // One initial batch refreshes eligibility after newly extracted text.
    let pending = pendingSemantic(data);
    do {
      signal.throwIfAborted();
      const eligible =
        data.semantic.eligiblePapers + data.semantic.eligibleDocuments;
      data = accept(await operations.indexSemantic());
      const next = pendingSemantic(data);
      publish({
        completed: data.semantic.indexedPapers + data.semantic.chunkedDocuments,
        total: data.semantic.eligiblePapers + data.semantic.eligibleDocuments,
      });
      const eligibilityGrew =
        data.semantic.eligiblePapers + data.semantic.eligibleDocuments >
        eligible;
      if (next > 0 && pending > 0 && next >= pending && !eligibilityGrew) {
        throw new Error(
          data.semantic.lastError ??
            "Indexing made no progress. Check the listed problems and retry preparation.",
        );
      }
      pending = next;
    } while (pending > 0);
    if (data.semantic.indexedChunks === 0) {
      publish({
        status: "partial",
        stage: "finished",
        total: null,
        detail:
          "No usable document passages yet. Review the papers needing attention before asking TYRION.",
      });
      return;
    }
    publish({
      status: progress.issues.length ? "partial" : "complete",
      stage: "finished",
      completed: usable,
      total: documents.length,
      unit: "documents",
      detail: progress.issues.length
        ? "Available documents prepared. Some papers need attention."
        : "Library prepared. TYRION also needs an available answer model.",
    });
  } catch (reason) {
    try {
      const latest = await operations.refresh();
      if (latest.source.activeLibraryId === libraryId) update(latest);
    } catch {
      publish({
        issues: [
          ...progress.issues,
          {
            itemId: "coverage",
            title: "Coverage refresh",
            message:
              "Completed work is saved. Reopen library setup or retry to refresh coverage.",
          },
        ],
      });
    }
    publish({
      status: signal.aborted ? "cancelled" : "error",
      total: null,
      detail: signal.aborted
        ? "Preparation cancelled. Completed work is kept."
        : reason instanceof Error
          ? reason.message
          : String(reason),
    });
  }
}
