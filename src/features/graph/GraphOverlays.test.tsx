import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { demoItems } from "../../adapters/demo/demoData";
import { GraphNavigationHelp, GraphSelection } from "./GraphOverlays";

describe("graph overlays", () => {
  it("renders the full selected title and every author", () => {
    const paper = demoItems.find((item) => item.authors.length > 1)!;
    const markup = renderToStaticMarkup(
      <GraphSelection paper={paper} additionalCount={2} />,
    );
    expect(markup).toContain(paper.title.replaceAll("&", "&amp;"));
    for (const author of paper.authors) expect(markup).toContain(author.name);
    expect(markup).toContain(`<em class="graph-selection-authors">`);
    if (paper.year != null) expect(markup).toContain(`[${paper.year}]`);
    expect(markup).toContain("+2 more selected");
    expect(markup).toContain('role="status"');
  });
  it("handles missing paper metadata without inventing authors", () => {
    expect(
      renderToStaticMarkup(
        <GraphSelection title="Paper title" additionalCount={0} />,
      ),
    ).toContain("Unknown author");
    expect(renderToStaticMarkup(<GraphSelection additionalCount={0} />)).toBe(
      "",
    );
  });
  it.each(["2d", "3d"] as const)(
    "provides dismissible, mode-specific %s help",
    (mode) => {
      const markup = renderToStaticMarkup(<GraphNavigationHelp mode={mode} />);
      expect(markup).toContain('popover="auto"');
      expect(markup).toContain('popoverTargetAction="hide"');
      expect(markup).toContain(`${mode.toUpperCase()} navigation`);
      expect(markup).toContain(mode === "3d" ? "Drag to orbit" : "Drag to pan");
      if (mode === "2d") expect(markup).not.toContain("Right-drag");
    },
  );
});
