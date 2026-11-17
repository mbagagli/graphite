import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { browserWorkspace } from "../../adapters/demo/browserDiagnostics";
import { SelectionPanel } from "./SelectionPanel";

it.each([0, 2])(
  "makes a %i-paper selection reviewable and editable",
  (count) => {
    const papers = browserWorkspace.snapshot.items.slice(0, count);
    const noop = vi.fn();
    const markup = renderToStaticMarkup(
      <SelectionPanel
        papers={papers}
        notice={null}
        canUndo={false}
        canSave
        onClose={noop}
        onSearch={noop}
        onOpenPaper={noop}
        onRemove={noop}
        onClear={noop}
        onUndo={noop}
        onSave={noop}
      />,
    );
    const save = markup.match(/<button[^>]*>Save as Lens<\/button>/)?.[0];
    expect(save?.includes("disabled")).toBe(count === 0);
    expect(markup).toContain('aria-labelledby="paper-selection-title"');
    expect(markup).not.toContain('aria-modal="true"');
    if (count) {
      expect(markup).toContain(papers[0].title.replaceAll("&", "&amp;"));
      expect(markup).toContain("from selection");
    } else {
      expect(markup).toContain("Start with a paper that interests you");
    }
  },
);
