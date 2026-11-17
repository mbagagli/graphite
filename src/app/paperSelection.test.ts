import { describe, expect, it } from "vitest";
import { browserWorkspace } from "../adapters/demo/browserDiagnostics";
import { emptyPaperSelection, paperSelectionReducer } from "./paperSelection";

const papers = browserWorkspace.snapshot.items.slice(0, 3);
const selected = papers.reduce(
  (state, paper) =>
    paperSelectionReducer(state, { type: "add", paper, source: "search" }),
  emptyPaperSelection,
);

describe("collected papers", () => {
  it("retains ordered metadata across additions and deduplicates IDs", () => {
    const duplicate = paperSelectionReducer(selected, {
      type: "add",
      paper: { ...papers[0], title: "Duplicate" },
      source: "search",
    });
    expect(duplicate).toBe(selected);
    expect(selected.papers).toEqual(papers);
    expect(emptyPaperSelection.papers).toEqual([]);
  });

  it("restores the exact order and metadata after removal or clear", () => {
    for (const action of [
      { type: "remove" as const, id: papers[1].id },
      { type: "toggle" as const, paper: papers[1], source: "search" as const },
      { type: "clear" as const },
    ]) {
      const changed = paperSelectionReducer(selected, action);
      expect(changed.papers).not.toContain(papers[1]);
      const restored = paperSelectionReducer(changed, { type: "undo" });
      expect(restored.papers).toEqual(papers);
      expect(restored.undo).toBeNull();
    }
  });

  it("does not let Undo discard papers added after a removal", () => {
    const removed = paperSelectionReducer(selected, {
      type: "remove",
      id: papers[1].id,
    });
    const added = paperSelectionReducer(removed, {
      type: "add",
      paper: papers[1],
      source: "search",
    });
    expect(added.undo).toBeNull();
    expect(paperSelectionReducer(added, { type: "undo" })).toBe(added);
  });

  it("clears metadata and Undo when the active library changes", () => {
    const removed = paperSelectionReducer(selected, {
      type: "remove",
      id: papers[1].id,
    });
    const reset = paperSelectionReducer(removed, { type: "reset" });
    expect(reset.papers).toEqual([]);
    expect(reset.notice).toContain("changed libraries");
    expect(paperSelectionReducer(reset, { type: "undo" })).toBe(reset);
  });

  it("makes explicit graph replacement undoable and ignores absent removals", () => {
    expect(
      paperSelectionReducer(selected, {
        type: "remove",
        id: "not-selected",
      }),
    ).toBe(selected);
    const replaced = paperSelectionReducer(selected, {
      type: "replace",
      paper: papers[2],
      source: "graph",
    });
    expect(replaced.papers).toEqual([papers[2]]);
    expect(paperSelectionReducer(replaced, { type: "undo" }).papers).toEqual(
      papers,
    );
  });
});
