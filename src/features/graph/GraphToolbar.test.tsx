import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { demoGraph } from "../../adapters/demo/demoData";
import { GraphToolbar, type GraphExplorationControls } from "./GraphToolbar";
import type { GraphLayerVisibility } from "../../domain/graph";
const controls: GraphExplorationControls = {
  selectedCount: 0,
  collections: [],
  similarScope: "library",
  semanticAvailable: false,
  citationIndexed: false,
  pending: null,
  error: null,
  notice: null,
  canGoBack: false,
  selectionConnectionsOnly: false,
  onScopeChange() {},
  onRetry() {},
  onSimilar() {},
  onCitations() {},
  onBack() {},
  onSelectionConnectionsChange() {},
};
const layers = {} as GraphLayerVisibility;
it("keeps exploration actions visible and explains an unindexed empty view", () => {
  const html = renderToStaticMarkup(
    <GraphToolbar
      graph={{ ...demoGraph, edges: [] }}
      layers={layers}
      controls={controls}
      onToggleLayer={() => {}}
    />,
  );
  expect(html).toContain('disabled="" title="Select a paper first"');
  expect(html).toContain("Find similar");
  expect(html).not.toContain("graph-selection-context");
  expect(html).toContain("Citations have not been indexed");
  expect(html).toContain("Library organization");
  expect(html).not.toContain("<select");
  expect(html).toContain('popover="auto"');
  expect(html).toContain("Find similar within");
});
it("shows explicit collection scope, union semantics and local errors", () => {
  const html = renderToStaticMarkup(
    <GraphToolbar
      graph={demoGraph}
      layers={layers}
      controls={{
        ...controls,
        selectedCount: 2,
        collections: [
          {
            id: "methods",
            name: "Methods",
            parentId: null,
            description: "",
            libraryId: "test-library",
          },
        ],
        semanticAvailable: true,
        similarScope: "collection:methods",
        error: "No indexed connections",
        canRetry: true,
      }}
      onToggleLayer={() => {}}
    />,
  );
  expect(html).toMatch(
    /type="radio"[^>]*checked=""[^>]*value="collection:methods"/,
  );
  expect(html).toContain("Methods");
  expect(html).toContain("combined representation");
  expect(html).toContain("any selected paper");
  expect(html).toContain('role="alert"');
  expect(html).toContain("Retry");
});
