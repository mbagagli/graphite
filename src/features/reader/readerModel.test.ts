import { describe, expect, it } from "vitest";

import {
  activeReaderOutlineId,
  isReaderTextEntryTarget,
  readerNavigationTarget,
  readerPageWindow,
  readerVisiblePages,
  resolveReaderOutline,
  restoreReaderState,
  type ReaderOutlineDocument,
} from "./readerModel";

describe("reader model", () => {
  it("restores bounded application-owned state", () => {
    expect(
      restoreReaderState(
        "document:one",
        {
          documentId: "document:one",
          pageNumber: 900,
          zoom: 8,
          fitMode: "custom",
          outlineOpen: false,
          contextOpen: true,
        },
        40,
      ),
    ).toEqual({
      documentId: "document:one",
      pageNumber: 40,
      zoom: 3,
      fitMode: "custom",
      outlineOpen: false,
      contextOpen: true,
    });
  });

  it("keeps a large document render window bounded", () => {
    expect(readerPageWindow(250, 500)).toEqual([249, 250, 251]);
    expect(readerPageWindow(1, 500)).toEqual([1, 2]);
    expect(readerPageWindow(500, 500)).toEqual([499, 500]);
  });

  it("derives predictable single-page and spread navigation", () => {
    expect(readerVisiblePages(1, 5, "single")).toEqual([1]);
    expect(readerVisiblePages(1, 5, "spread")).toEqual([1, 2]);
    expect(readerVisiblePages(5, 5, "spread")).toEqual([5]);
    expect(readerNavigationTarget(1, 5, "spread", "next")).toBe(3);
    expect(readerNavigationTarget(3, 5, "spread", "previous")).toBe(1);
    expect(readerNavigationTarget(5, 5, "spread", "next")).toBe(5);
    expect(readerNavigationTarget(2, 5, "single", "previous")).toBe(1);
  });

  it("does not claim arrow keys while a text-entry control is active", () => {
    expect(isReaderTextEntryTarget("INPUT", false)).toBe(true);
    expect(isReaderTextEntryTarget("TEXTAREA", false)).toBe(true);
    expect(isReaderTextEntryTarget("SELECT", false)).toBe(true);
    expect(isReaderTextEntryTarget("DIV", true)).toBe(true);
    expect(isReaderTextEntryTarget("BUTTON", false)).toBe(false);
  });

  it("maps nested, named, explicit, and numeric PDF destinations to pages", async () => {
    const document: ReaderOutlineDocument = {
      numPages: 8,
      getOutline: async () => [
        {
          title: "Introduction",
          dest: "intro",
          items: [
            {
              title: "Background",
              dest: [{ num: 17, gen: 0 }],
              items: [],
            },
          ],
        },
        { title: "Methods", dest: [4], items: [] },
        { title: "Unmapped note", dest: null, items: [] },
      ],
      getDestination: async (name) =>
        name === "intro" ? [{ num: 11, gen: 0 }] : null,
      getPageIndex: async (reference) =>
        reference.num === 11 ? 0 : reference.num === 17 ? 2 : 7,
    };

    await expect(resolveReaderOutline(document)).resolves.toEqual([
      {
        id: "outline-0",
        title: "Introduction",
        pageNumber: 1,
        depth: 0,
      },
      {
        id: "outline-0-0",
        title: "Background",
        pageNumber: 3,
        depth: 1,
      },
      {
        id: "outline-1",
        title: "Methods",
        pageNumber: 5,
        depth: 0,
      },
      {
        id: "outline-2",
        title: "Unmapped note",
        pageNumber: null,
        depth: 0,
      },
    ]);
  });

  it("tracks the nearest mapped outline entry at the active page", () => {
    const entries = [
      { id: "intro", title: "Introduction", pageNumber: 1, depth: 0 },
      { id: "methods", title: "Methods", pageNumber: 4, depth: 0 },
      { id: "results", title: "Results", pageNumber: 7, depth: 0 },
    ];
    expect(activeReaderOutlineId(entries, 1)).toBe("intro");
    expect(activeReaderOutlineId(entries, 6)).toBe("methods");
    expect(activeReaderOutlineId(entries, 8)).toBe("results");
    expect(
      activeReaderOutlineId(
        [{ id: "none", title: "Unmapped", pageNumber: null, depth: 0 }],
        3,
      ),
    ).toBeNull();
  });
});
