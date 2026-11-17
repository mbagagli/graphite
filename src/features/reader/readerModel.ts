import type { ReaderState } from "../../services/contracts";

export type ReaderViewMode = "single" | "spread";
export type ReaderNavigationDirection = "previous" | "next";

interface ReaderPdfOutlineNode {
  title: string;
  dest: string | unknown[] | null;
  items: ReaderPdfOutlineNode[];
}

interface ReaderPdfReference {
  num: number;
  gen: number;
}

export interface ReaderOutlineDocument {
  numPages: number;
  getOutline(): Promise<ReaderPdfOutlineNode[] | null>;
  getDestination(name: string): Promise<unknown[] | null>;
  getPageIndex(reference: ReaderPdfReference): Promise<number>;
}

export interface ReaderOutlineEntry {
  id: string;
  title: string;
  pageNumber: number | null;
  depth: number;
}

export function readerPageWindow(
  pageNumber: number,
  pageCount: number,
): number[] {
  if (pageCount < 1) return [];
  const current = Math.min(Math.max(1, Math.round(pageNumber)), pageCount);
  return [current - 1, current, current + 1].filter(
    (page) => page >= 1 && page <= pageCount,
  );
}

export function readerVisiblePages(
  pageNumber: number,
  pageCount: number,
  viewMode: ReaderViewMode,
): number[] {
  if (pageCount < 1) return [];
  const current = Math.min(Math.max(1, Math.round(pageNumber)), pageCount);
  if (viewMode === "single" || current === pageCount) return [current];
  return [current, current + 1];
}

export function readerNavigationTarget(
  pageNumber: number,
  pageCount: number,
  viewMode: ReaderViewMode,
  direction: ReaderNavigationDirection,
): number {
  if (pageCount < 1) return 1;
  const current = Math.min(Math.max(1, Math.round(pageNumber)), pageCount);
  const step = viewMode === "spread" ? 2 : 1;
  return Math.min(
    Math.max(1, current + (direction === "next" ? step : -step)),
    pageCount,
  );
}

export function isReaderTextEntryTarget(
  tagName: string,
  isContentEditable: boolean,
): boolean {
  return (
    isContentEditable ||
    tagName === "INPUT" ||
    tagName === "TEXTAREA" ||
    tagName === "SELECT"
  );
}

export async function resolveReaderOutline(
  document: ReaderOutlineDocument,
  limit = 80,
): Promise<ReaderOutlineEntry[]> {
  const outline = await document.getOutline();
  if (!outline?.length) return [];

  const flattened: Array<{
    id: string;
    title: string;
    dest: string | unknown[] | null;
    depth: number;
  }> = [];
  const visit = (
    entries: ReaderPdfOutlineNode[],
    depth: number,
    parentPath: number[],
  ) => {
    for (let index = 0; index < entries.length; index += 1) {
      if (flattened.length >= limit) return;
      const entry = entries[index];
      const path = [...parentPath, index];
      flattened.push({
        id: `outline-${path.join("-")}`,
        title: entry.title,
        dest: entry.dest,
        depth,
      });
      visit(entry.items ?? [], depth + 1, path);
    }
  };
  visit(outline, 0, []);

  return Promise.all(
    flattened.map(async (entry) => ({
      id: entry.id,
      title: entry.title,
      pageNumber: await resolveOutlinePage(document, entry.dest),
      depth: entry.depth,
    })),
  );
}

export function activeReaderOutlineId(
  entries: ReaderOutlineEntry[],
  pageNumber: number,
): string | null {
  let active: ReaderOutlineEntry | null = null;
  for (const entry of entries) {
    if (
      entry.pageNumber !== null &&
      entry.pageNumber <= pageNumber &&
      (!active || entry.pageNumber >= (active.pageNumber ?? 0))
    ) {
      active = entry;
    }
  }
  return active?.id ?? null;
}

export function restoreReaderState(
  documentId: string,
  stored: ReaderState | null,
  pageCount: number,
): ReaderState {
  return {
    documentId,
    pageNumber: Math.min(
      Math.max(1, stored?.pageNumber ?? 1),
      Math.max(1, pageCount),
    ),
    zoom: Math.min(Math.max(0.4, stored?.zoom ?? 1), 3),
    fitMode: stored?.fitMode ?? "width",
    outlineOpen: stored?.outlineOpen ?? true,
    contextOpen: stored?.contextOpen ?? true,
  };
}

async function resolveOutlinePage(
  document: ReaderOutlineDocument,
  destination: string | unknown[] | null,
): Promise<number | null> {
  const explicit =
    typeof destination === "string"
      ? await document.getDestination(destination)
      : destination;
  if (!Array.isArray(explicit) || explicit.length === 0) return null;
  const target = explicit[0];
  if (typeof target === "number" && Number.isInteger(target)) {
    const pageNumber = target + 1;
    return pageNumber >= 1 && pageNumber <= document.numPages
      ? pageNumber
      : null;
  }
  if (!isPdfReference(target)) return null;
  try {
    const pageNumber = (await document.getPageIndex(target)) + 1;
    return pageNumber >= 1 && pageNumber <= document.numPages
      ? pageNumber
      : null;
  } catch {
    return null;
  }
}

function isPdfReference(value: unknown): value is ReaderPdfReference {
  return (
    typeof value === "object" &&
    value !== null &&
    "num" in value &&
    "gen" in value &&
    typeof value.num === "number" &&
    typeof value.gen === "number"
  );
}
