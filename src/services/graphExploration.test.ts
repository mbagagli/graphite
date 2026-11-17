import { expect, it } from "vitest";
import { demoItems, demoGraph, demoSnapshot } from "../adapters/demo/demoData";
import { mergeNeighborhoods, sharedMetadata } from "./graphExploration";
import type { ResearchNeighborhood } from "./contracts";
const base = {
  items: demoItems,
  topics: demoSnapshot.topics,
  graph: demoGraph,
};
it("unions seeds, ranks common neighbors, retains uncovered seeds and does not multiply evidence", () => {
  const [a, b, c, d] = demoGraph.nodes;
  const edge = (source: string, target: string) => ({
    id: `${source}-${target}`,
    source,
    target,
    relation: "co-cited" as const,
    weight: 1,
    directed: false,
    evidenceCount: 2,
    evidenceLabel: "two citing papers",
  });
  const part = (edges: ReturnType<typeof edge>[]): ResearchNeighborhood => ({
    projection: {
      ...base,
      graph: { ...demoGraph, nodes: [a, b, c, d], edges },
    },
    mode: "co-cited",
    totalRelationships: edges.length,
    hiddenRelationships: 0,
  });
  const merged = mergeNeighborhoods(
    [a.id, b.id, d.id],
    base,
    [
      part([edge(a.id, b.id), edge(a.id, c.id)]),
      part([edge(b.id, a.id), edge(b.id, c.id)]),
      part([]),
    ],
    "co-cited",
    4,
  );
  expect(merged.projection.graph.edges.length).toBe(3);
  expect(
    merged.projection.graph.edges.every((e) => e.evidenceCount === 2),
  ).toBe(true);
  expect(merged.seedConnections[c.id]).toEqual([a.id, b.id].sort());
  expect(merged.seedCoverage[2].reason).toBeTruthy();
  expect(merged.projection.graph.nodes.map((n) => n.id)).toContain(d.id);
  const capped = mergeNeighborhoods(
    [a.id, b.id],
    base,
    [part([edge(a.id, c.id), edge(a.id, d.id)]), part([edge(b.id, c.id)])],
    "co-cited",
    1,
  );
  expect(capped.projection.graph.nodes.map((n) => n.id)).toContain(c.id);
  expect(capped.projection.graph.nodes.map((n) => n.id)).not.toContain(d.id);
  expect(capped.hiddenRelationships).toBe(1);
});
it("matches metadata with indexing-compatible normalization", () => {
  expect(
    sharedMetadata(["  Jane   Doe", "JANE DOE", "Other"], ["jane doe"]),
  ).toEqual(["JANE DOE"]);
});

it("ranks equal seed counts by evidence and preserves external referenced works", () => {
  const [a, b, c] = demoGraph.nodes;
  const external = {
    ...c,
    paperId: null,
    entityKind: "external-work" as const,
  };
  const part: ResearchNeighborhood = {
    projection: {
      ...base,
      graph: {
        ...demoGraph,
        nodes: [a, b, external],
        edges: [
          {
            id: "a-b",
            source: a.id,
            target: b.id,
            relation: "citation",
            directed: true,
            weight: 1,
            evidenceCount: 1,
            evidenceLabel: null,
          },
          {
            id: "a-c",
            source: a.id,
            target: c.id,
            relation: "citation",
            directed: true,
            weight: 1,
            evidenceCount: 3,
            evidenceLabel: null,
          },
        ],
      },
    },
    mode: "references",
    totalRelationships: 2,
    hiddenRelationships: 0,
  };
  const result = mergeNeighborhoods([a.id], base, [part], "references", 1);
  expect(result.projection.graph.nodes.map((n) => n.id)).toEqual([a.id, c.id]);
  expect(result.projection.graph.nodes[1].entityKind).toBe("external-work");
  expect(result.seedConnections[c.id]).toEqual([a.id]);
  const empty = mergeNeighborhoods(
    [a.id],
    base,
    [
      {
        ...part,
        projection: { ...base, graph: { ...demoGraph, edges: [] } },
      },
    ],
    "references",
    1,
  );
  expect(empty.seedCoverage[0].reason).toBeTruthy();
  expect(empty.projection.graph.nodes.map((n) => n.id)).toEqual([a.id]);
});
