import type { GraphEdge, GraphRelationType } from "../domain/graph";
import type { GraphProjection, ResearchNeighborhood } from "./contracts";

export interface SeedCoverage {
  id: string;
  reason: string | null;
}
export interface ExplorationResult extends ResearchNeighborhood {
  seedCoverage: SeedCoverage[];
  seedConnections: Record<string, string[]>;
}

export const explorationRelations: Record<string, GraphRelationType> = {
  references: "citation",
  "cited-by": "citation",
  citation: "citation",
  "shared-references": "bibliographic-coupling",
  "co-cited": "co-cited",
  semantic: "semantically-similar",
};

/** Merge per-seed evidence without multiplying duplicate undirected edges. */
export function mergeNeighborhoods(
  seeds: string[],
  base: GraphProjection,
  parts: ResearchNeighborhood[],
  mode: string,
  limit: number,
): ExplorationResult {
  const seedSet = new Set(seeds);
  const nodes = new Map(
    base.graph.nodes.filter((n) => seedSet.has(n.id)).map((n) => [n.id, n]),
  );
  const items = new Map(
    base.items.filter((p) => seedSet.has(p.id)).map((p) => [p.id, p]),
  );
  const topics = new Map(base.topics.map((t) => [t.id, t]));
  const edges = new Map<string, GraphEdge>();
  const attribution = new Map<string, Set<string>>();
  const strength = new Map<string, number>();
  const coverage: SeedCoverage[] = [];
  let unseen = 0;
  for (const [index, part] of parts.entries()) {
    const seed = seeds[index];
    const relevant = part.projection.graph.edges.filter(
      (e) =>
        e.relation === explorationRelations[mode] &&
        (e.source === seed || e.target === seed),
    );
    coverage.push({
      id: seed,
      reason: relevant.length
        ? null
        : "No indexed connections available for this exploration.",
    });
    unseen += part.hiddenRelationships;
    for (const node of part.projection.graph.nodes) nodes.set(node.id, node);
    for (const item of part.projection.items) items.set(item.id, item);
    for (const topic of part.projection.topics) topics.set(topic.id, topic);
    for (const edge of relevant) {
      const endpoints = edge.directed
        ? [edge.source, edge.target]
        : [edge.source, edge.target].sort();
      const key = JSON.stringify([edge.relation, ...endpoints]);
      const existing = edges.get(key);
      if (!existing || edge.evidenceCount > existing.evidenceCount)
        edges.set(key, edge);
      const other = edge.source === seed ? edge.target : edge.source;
      if (!attribution.has(other)) attribution.set(other, new Set());
      attribution.get(other)!.add(seed);
      strength.set(
        other,
        Math.max(strength.get(other) ?? 0, edge.evidenceCount),
      );
    }
  }
  const candidates = [...attribution.keys()]
    .filter((id) => !seedSet.has(id))
    .sort(
      (a, b) =>
        attribution.get(b)!.size - attribution.get(a)!.size ||
        (strength.get(b) ?? 0) - (strength.get(a) ?? 0) ||
        a.localeCompare(b, "en"),
    );
  const included = new Set([
    ...seeds,
    ...candidates.slice(0, Math.max(0, Math.min(1200 - seeds.length, limit))),
  ]);
  const visibleEdges = [...edges.values()].filter(
    (e) => included.has(e.source) && included.has(e.target),
  );
  return {
    projection: {
      items: [...items.values()].filter((p) => included.has(p.id)),
      topics: [...topics.values()],
      graph: {
        nodes: [...nodes.values()].filter((n) => included.has(n.id)),
        edges: visibleEdges,
        availableRelations: [...new Set(visibleEdges.map((e) => e.relation))],
        totalNodeCount: seedSet.size + candidates.length,
        hasMore: candidates.length + seedSet.size > included.size || unseen > 0,
      },
    },
    mode: mode as ResearchNeighborhood["mode"],
    totalRelationships: edges.size + unseen,
    hiddenRelationships: edges.size - visibleEdges.length + unseen,
    seedCoverage: coverage,
    seedConnections: Object.fromEntries(
      [...attribution].map(([id, ids]) => [id, [...ids].sort()]),
    ),
  };
}

/** Resolve metadata overlap using the same whitespace/case normalization as indexing. */
export function sharedMetadata(left: string[], right: string[]): string[] {
  const normalize = (value: string) =>
    value.trim().replace(/\s+/g, " ").toLowerCase();
  const matches = new Set(right.map(normalize));
  return [
    ...new Map(
      left
        .filter((value) => matches.has(normalize(value)))
        .map((value) => [normalize(value), value]),
    ).values(),
  ];
}
