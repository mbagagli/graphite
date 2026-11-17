import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { browserWorkspace } from "../../adapters/demo/browserDiagnostics";
import { demoItems } from "../../adapters/demo/demoData";
import { AnalyticsView } from "./AnalyticsView";
import { groupPapersByYear } from "./analyticsYears";
import { YearPapersDialog } from "./YearPapersDialog";

describe("publication year browsing", () => {
  it("groups precisely the supplied view, including missing years", () => {
    const items = [
      { ...demoItems[0], id: "b", title: "Same title", year: 2022 },
      { ...demoItems[1], id: "a", title: "same title", year: 2022 },
      { ...demoItems[2], id: "c", title: "Earlier", year: 2018 },
      { ...demoItems[3], id: "d", year: null },
    ];
    const groups = groupPapersByYear(items);
    expect(groups.map((group) => group.year)).toEqual([2018, 2022, null]);
    expect(groups[1].papers.map((paper) => paper.id)).toEqual(["a", "b"]);
    expect(groups[2].papers.map((paper) => paper.id)).toEqual(["d"]);
    expect(groups.flatMap((group) => group.papers)).toHaveLength(items.length);
    expect(items.map((paper) => paper.id)).toEqual(["b", "a", "c", "d"]);
  });

  it("handles empty and invalid year metadata without inventing dates", () => {
    expect(groupPapersByYear([])).toEqual([]);
    expect(
      groupPapersByYear([{ ...demoItems[0], year: Number.NaN }])[0].year,
    ).toBeNull();
  });

  it("uses the current records even when cached analytics describe another view", () => {
    const workspace = browserWorkspace;
    const markup = renderToStaticMarkup(
      <AnalyticsView
        analytics={{
          ...workspace.analytics,
          byYear: [{ year: 1900, count: 999 }],
        }}
        topics={workspace.snapshot.topics}
        items={[
          { ...demoItems[0], year: 2022 },
          { ...demoItems[1], year: 2022 },
          { ...demoItems[2], year: null },
        ]}
        onSelectPaper={vi.fn()}
        provenance="real-demo"
        scopeLabel="Selected collection"
        totalItemCount={72}
      />,
    );
    expect(markup).toContain('aria-label="2022: 2 papers"');
    expect(markup).toContain("<small>2022</small>");
    expect(markup).toContain("<small>No year</small>");
    expect(markup).toContain('aria-label="No year: 1 paper"');
    expect(markup).not.toContain('aria-label="1900: 999 papers"');
    expect(markup).toContain("Selected collection: 3 loaded papers of 72");
    expect(markup).toContain('aria-haspopup="dialog"');
    expect(markup).toContain("semantic groups");
    expect(markup).toContain("Automatic semantic topics");
    expect(markup).not.toContain("Frequent topic tags");
  });

  it("keeps all records and identifying metadata available in long lists", () => {
    const papers = Array.from({ length: 1_200 }, (_, index) => ({
      ...demoItems[0],
      id: `record-${index}`,
      title: `Paper ${String(index).padStart(4, "0")}`,
      doi: `10.1234/example-${index}`,
      year: 2020,
    }));
    const markup = renderToStaticMarkup(
      <YearPapersDialog
        year={2020}
        papers={groupPapersByYear(papers)[0].papers}
        onClose={vi.fn()}
        onSelectPaper={vi.fn()}
      />,
    );
    expect(markup).toContain("2020 - 1200 papers");
    expect(markup.match(/<li>/g)).toHaveLength(1_200);
    expect(markup).toContain("Paper 1199");
    expect(markup).toContain("10.1234/example-1199");
    expect(markup).toContain(demoItems[0].authors[0].name);
    expect(markup).toContain('class="modal-dialog-body"');
  });

  it("explains a selection that becomes empty", () => {
    const markup = renderToStaticMarkup(
      <YearPapersDialog
        year={2020}
        papers={[]}
        onClose={vi.fn()}
        onSelectPaper={vi.fn()}
      />,
    );
    expect(markup).toContain("2020 - 0 papers");
    expect(markup).toContain("No papers match this year in the current view.");
  });
});
