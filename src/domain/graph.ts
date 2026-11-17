import type { ResearchItem, TopicId } from "./research";

export type GraphRelationType =
  | "citation"
  | "bibliographic-coupling"
  | "co-cited"
  | "semantically-similar"
  | "shared-creator"
  | "shared-collection"
  | "shared-tag"
  | "zotero-related";

export interface GraphNode {
  id: string;
  paperId: ResearchItem["id"] | null;
  entityKind: "research-item" | "external-work";
  label: string;
  detail: string | null;
  topicId: TopicId;
  x: number;
  y: number;
  importance: number;
}

export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  relation: GraphRelationType;
  weight: number;
  directed: boolean;
  evidenceCount: number;
  evidenceLabel: string | null;
}

export interface ResearchGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  availableRelations: GraphRelationType[];
  totalNodeCount: number;
  hasMore: boolean;
}

export type GraphLayerVisibility = Record<GraphRelationType, boolean>;
