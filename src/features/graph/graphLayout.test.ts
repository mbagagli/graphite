import { describe, expect, it } from "vitest";
import { demoGraph } from "../../adapters/demo/demoData";
import {
  prepareLayouts,
  rebasePaperCamera,
  rendererData,
  projectionKey,
  placeNodes,
} from "./graphLayout";

describe("shared graph layouts", () => {
  it("settles reproducibly irrespective of domain order, with a readable plane", () => {
    const a = prepareLayouts(demoGraph);
    const reversed = {
      ...demoGraph,
      nodes: [...demoGraph.nodes].reverse(),
      edges: [...demoGraph.edges].reverse(),
    };
    const b = prepareLayouts(reversed);
    expect(a.positions).toEqual(b.positions);
    expect(projectionKey(reversed)).toBe(a.key);
    const flat = [...a.positions["2d"].values()];
    expect(flat.every((point) => point.z === 0)).toBe(true);
    let nearest = Infinity;
    for (let i = 0; i < flat.length; i++)
      for (let j = i + 1; j < flat.length; j++)
        nearest = Math.min(
          nearest,
          Math.hypot(flat[i].x - flat[j].x, flat[i].y - flat[j].y),
        );
    expect(nearest).toBeGreaterThan(15);
    expect(
      new Set([...a.positions["3d"].values()].map((p) => p.z)).size,
    ).toBeGreaterThan(1);
  });
  it("isolates physics mutations and restores exact spatial coordinates", () => {
    const before = JSON.stringify(demoGraph);
    const layouts = prepareLayouts(demoGraph);
    const data = rendererData(demoGraph, layouts.positions["3d"]);
    const original = data.nodes.map(({ id, x, y, z }) => ({ id, x, y, z }));
    placeNodes(data.nodes, layouts.positions["2d"]);
    data.links[0].source = "changed";
    placeNodes(data.nodes, layouts.positions["3d"]);
    expect(data.nodes.map(({ id, x, y, z }) => ({ id, x, y, z }))).toEqual(
      original,
    );
    expect(JSON.stringify(demoGraph)).toBe(before);
  });
  it("handles empty graphs, duplicate IDs, and edges outside the projection", () => {
    const empty = { ...demoGraph, nodes: [], edges: [] };
    expect(prepareLayouts(empty).positions["2d"].size).toBe(0);
    const data = rendererData({
      ...demoGraph,
      nodes: [demoGraph.nodes[0], demoGraph.nodes[0]],
    });
    expect(data.nodes.length).toBe(1);
    expect(
      data.links.every(
        (link) =>
          link.source === data.nodes[0].id && link.target === data.nodes[0].id,
      ),
    ).toBe(true);
  });
  it("retains every entity in a full large library", () => {
    const nodes = Array.from({ length: 10_000 }, (_, i) => ({
      ...demoGraph.nodes[0],
      id: `node-${i}`,
    }));
    const data = rendererData({ ...demoGraph, nodes, edges: [] });
    expect(data.nodes.length).toBe(10_000);
    expect(new Set(data.nodes.map((node) => node.id)).size).toBe(10_000);
  });
});

it("keeps the active paper camera-relative position and zoom on projection changes", () => {
  const previous = { x: 40, y: -20, z: 70 };
  const next = { x: -200, y: 80, z: 0 };
  const camera = {
    position: { x: 100, y: 200, z: 600 },
    target: { x: 50, y: 10, z: 20 },
  };
  const moved = rebasePaperCamera(camera, previous, next);
  for (const axis of ["x", "y", "z"] as const) {
    expect(next[axis] - moved.position[axis]).toBe(
      previous[axis] - camera.position[axis],
    );
    expect(moved.position[axis] - moved.target[axis]).toBe(
      camera.position[axis] - camera.target[axis],
    );
  }
  expect(rebasePaperCamera(moved, next, previous)).toEqual(camera);
});

it("excludes organizational edges from layout forces without removing their overlays", () => {
  const scientific = {
    ...demoGraph,
    edges: demoGraph.edges.filter((e) =>
      [
        "citation",
        "semantically-similar",
        "bibliographic-coupling",
        "co-cited",
      ].includes(e.relation),
    ),
  };
  expect(prepareLayouts(demoGraph).positions).toEqual(
    prepareLayouts(scientific).positions,
  );
  expect(rendererData(demoGraph).links.length).toBe(demoGraph.edges.length);
  expect(projectionKey(demoGraph)).toContain("scientific-v1");
});

it("groups primary topics into disjoint regions without changing the graph", () => {
  const categorizedGraph = {
    ...demoGraph,
    nodes: demoGraph.nodes.map((node, i) => ({
      ...node,
      topicId: `topic-${i % 3}`,
    })),
  };
  const before = JSON.stringify(categorizedGraph);
  const grouped = prepareLayouts(categorizedGraph, "topics");
  expect(grouped.positions).toEqual(
    prepareLayouts(
      {
        ...categorizedGraph,
        nodes: [...categorizedGraph.nodes].reverse(),
      },
      "topics",
    ).positions,
  );
  expect(grouped.key).not.toBe(projectionKey(categorizedGraph));
  for (const mode of ["2d", "3d"] as const) {
    const boxes = new Map<string, { x: number; y: number }[]>();
    for (const node of categorizedGraph.nodes) {
      const points = boxes.get(node.topicId) ?? [];
      points.push(grouped.positions[mode].get(node.id)!);
      boxes.set(node.topicId, points);
    }
    const bounds = [...boxes.values()].map((points) => ({
      left: Math.min(...points.map((p) => p.x)),
      right: Math.max(...points.map((p) => p.x)),
      top: Math.min(...points.map((p) => p.y)),
      bottom: Math.max(...points.map((p) => p.y)),
    }));
    for (let i = 0; i < bounds.length; i++) {
      for (let j = i + 1; j < bounds.length; j++) {
        const a = bounds[i];
        const b = bounds[j];
        expect(
          a.right + 60 <= b.left ||
            b.right + 60 <= a.left ||
            a.bottom + 60 <= b.top ||
            b.bottom + 60 <= a.top,
        ).toBe(true);
      }
    }
  }
  const changed = {
    ...categorizedGraph,
    nodes: categorizedGraph.nodes.map((node, i) =>
      i ? node : { ...node, topicId: "new-topic" },
    ),
  };
  expect(projectionKey(changed, "topics")).not.toBe(grouped.key);
  expect(JSON.stringify(categorizedGraph)).toBe(before);
  expect(prepareLayouts(categorizedGraph).positions["2d"]).not.toEqual(
    grouped.positions["2d"],
  );
});

it("keeps empty and single-topic grouped layouts usable", () => {
  const graph = { ...demoGraph, nodes: [], edges: [] };
  expect(prepareLayouts(graph, "topics").positions["2d"].size).toBe(0);
  const single = {
    ...demoGraph,
    nodes: demoGraph.nodes.map((node) => ({ ...node, topicId: "one" })),
  };
  expect(prepareLayouts(single, "topics").positions).toEqual(
    prepareLayouts(single).positions,
  );
});
