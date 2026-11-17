import {
  forceSimulation,
  forceLink,
  forceManyBody,
  forceCenter,
  forceCollide,
} from "d3-force-3d";
import type { ResearchGraph } from "../../domain/graph";

export type GraphMode = "2d" | "3d";
export type GraphGrouping = "relations" | "topics";
export interface GraphPosition {
  x: number;
  y: number;
  z: number;
}
export interface GraphCamera {
  position: GraphPosition;
  target: GraphPosition;
}
export type GraphLayout = Map<string, GraphPosition>;
export interface GraphLayouts {
  key: string;
  positions: Record<GraphMode, GraphLayout>;
  preparationMs: number;
}
export interface GraphSession {
  mode?: GraphMode;
  restore?: boolean;
  layouts?: GraphLayouts;
  cameras: Partial<Record<GraphMode, GraphCamera>>;
  startupPlayed: boolean;
  anchors?: Partial<Record<GraphMode, { id: string; position: GraphPosition }>>;
}
export interface RenderNode extends GraphPosition {
  id: string;
  fx?: number;
  fy?: number;
  fz?: number;
}
export const LAYOUT_TICKS = 180;
export const FLAT_LAYOUT_TICKS = 240;
export const FLAT_COLLISION_RADIUS = 9;

export const SCIENTIFIC_RELATIONS = new Set([
  "citation",
  "semantically-similar",
  "bibliographic-coupling",
  "co-cited",
]);

export function projectionKey(
  graph: ResearchGraph,
  grouping: GraphGrouping = "relations",
): string {
  return JSON.stringify([
    "scientific-v1",
    grouping,
    grouping === "topics"
      ? graph.nodes.map((node) => [node.id, node.topicId]).sort()
      : null,
    graph.nodes.map((node) => node.id).sort(),
    graph.edges.map((edge) => [edge.id, edge.source, edge.target]).sort(),
  ]);
}

export function rendererData(graph: ResearchGraph, layout?: GraphLayout) {
  let seed = 42;
  const random = () => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed / 4294967296 - 0.5;
  };
  const unique = new Map(graph.nodes.map((node) => [node.id, node]));
  const nodes: RenderNode[] = [...unique.values()]
    .sort((a, b) => a.id.localeCompare(b.id, "en"))
    .map((node) => ({
      id: node.id,
      ...(layout?.get(node.id) ?? {
        x: random() * 200,
        y: random() * 200,
        z: random() * 200,
      }),
    }));
  const ids = new Set(nodes.map((node) => node.id));
  const links = [...graph.edges]
    .filter((edge) => ids.has(edge.source) && ids.has(edge.target))
    .sort((a, b) => a.id.localeCompare(b.id, "en"))
    .map((edge) => ({ id: edge.id, source: edge.source, target: edge.target }));
  return { nodes, links };
}

export function prepareLayouts(
  graph: ResearchGraph,
  grouping: GraphGrouping = "relations",
): GraphLayouts {
  const started = performance.now();
  const settle = (dimensions: 2 | 3, seed?: GraphLayout): GraphLayout => {
    const data = rendererData(
      {
        ...graph,
        edges: graph.edges.filter((edge) =>
          SCIENTIFIC_RELATIONS.has(edge.relation),
        ),
      },
      seed,
    );
    if (dimensions === 2) for (const node of data.nodes) node.z = 0;
    const simulation = forceSimulation(data.nodes, dimensions)
      .stop()
      .force(
        "link",
        forceLink(data.links)
          .id((node) => node.id)
          .distance(35),
      )
      .force("charge", forceManyBody().strength(-60))
      .force("center", forceCenter());
    if (dimensions === 2)
      simulation.force(
        "collision",
        forceCollide(FLAT_COLLISION_RADIUS).iterations(3),
      );
    simulation.tick(dimensions === 2 ? FLAT_LAYOUT_TICKS : LAYOUT_TICKS);
    return new Map(data.nodes.map(({ id, x, y, z }) => [id, { x, y, z }]));
  };
  const spatial = settle(3);
  const flat = settle(2, spatial);
  return {
    key: projectionKey(graph, grouping),
    positions: {
      "3d":
        grouping === "topics" ? groupLayoutByTopic(graph, spatial) : spatial,
      "2d": grouping === "topics" ? groupLayoutByTopic(graph, flat) : flat,
    },
    preparationMs: performance.now() - started,
  };
}

const TOPIC_GROUP_GAP = 64;

/** Pack metadata groups while retaining their internal scientific layout. */
function groupLayoutByTopic(
  graph: ResearchGraph,
  layout: GraphLayout,
): GraphLayout {
  const groups = new Map<string, string[]>();
  for (const node of graph.nodes) {
    const topic = node.topicId ?? "";
    const ids = groups.get(topic) ?? [];
    if (!ids.includes(node.id)) ids.push(node.id);
    groups.set(topic, ids);
  }
  if (groups.size < 2) return layout;
  const boxes = [...groups.entries()]
    .sort(([a], [b]) => a.localeCompare(b, "en"))
    .map(([, ids]) => {
      const points = ids.map((id) => layout.get(id)!);
      const left = Math.min(...points.map((p) => p.x));
      const top = Math.min(...points.map((p) => p.y));
      const width = Math.max(...points.map((p) => p.x)) - left;
      const height = Math.max(...points.map((p) => p.y)) - top;
      return { ids, left, top, width, height };
    });
  const columns = Math.ceil(Math.sqrt(boxes.length));
  const result: GraphLayout = new Map();
  let y = 0;
  for (let row = 0; row < boxes.length; row += columns) {
    let x = 0;
    const cells = boxes.slice(row, row + columns);
    for (const box of cells) {
      for (const id of box.ids) {
        const point = layout.get(id)!;
        result.set(id, {
          x: point.x - box.left + x,
          y: point.y - box.top + y,
          z: point.z,
        });
      }
      x += box.width + TOPIC_GROUP_GAP;
    }
    y += Math.max(...cells.map((box) => box.height)) + TOPIC_GROUP_GAP;
  }
  const points = [...result.values()];
  const centerX = Math.max(...points.map((p) => p.x)) / 2;
  const centerY = Math.max(...points.map((p) => p.y)) / 2;
  for (const point of points) {
    point.x -= centerX;
    point.y -= centerY;
  }
  return result;
}

export function placeNodes(nodes: RenderNode[], layout: GraphLayout) {
  for (const node of nodes) {
    const point = layout.get(node.id);
    if (!point) continue;
    node.x = node.fx = point.x;
    node.y = node.fy = point.y;
    node.z = node.fz = point.z;
  }
}

/** Translate the camera with its active paper to retain screen position. */
export function rebasePaperCamera(
  camera: GraphCamera,
  previous: GraphPosition,
  next: GraphPosition,
): GraphCamera {
  const shift = (point: GraphPosition): GraphPosition => ({
    x: point.x + next.x - previous.x,
    y: point.y + next.y - previous.y,
    z: point.z + next.z - previous.z,
  });
  return { position: shift(camera.position), target: shift(camera.target) };
}
