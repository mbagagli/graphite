import type { ResearchItem } from "../../domain/research";
import type {
  GraphProjectionRequest,
  ItemQueryRequest,
  SearchService,
  LibraryAdapter,
  WorkspaceService,
  GraphService,
  DocumentService,
  CitationService,
  SemanticService,
  ResearchAgentService,
  AnalyticsService,
  LensService,
  ReaderState,
  ResearchNeighborhood,
} from "../../services/contracts";
import { writeSourceMode } from "../../services/storage";
import {
  browserAgentEnvironment,
  browserWorkspace,
} from "./browserDiagnostics";
import { demoGraph, demoItems, demoSnapshot } from "./demoData";

const desktopMessage =
  "Open the Graphite desktop app to download the real demo PDFs, index their text, and use local research models.";
async function desktopRequired(): Promise<never> {
  throw new Error(desktopMessage);
}

function selectedItems(request: ItemQueryRequest): ResearchItem[] {
  return demoItems
    .filter(
      (item) =>
        (!request.collectionId ||
          item.collectionIds.includes(request.collectionId)) &&
        (!request.itemType ||
          item.provenance.zoteroItemType === request.itemType) &&
        (request.yearFrom === undefined || item.year! >= request.yearFrom) &&
        (request.yearTo === undefined || item.year! <= request.yearTo) &&
        (!request.tag || item.tags.includes(request.tag)),
    )
    .sort((a, b) =>
      request.sort === "title"
        ? a.title.localeCompare(b.title, "en")
        : b.year! - a.year! || a.title.localeCompare(b.title, "en"),
    );
}

function projection(request: GraphProjectionRequest) {
  const selected = selectedItems(request);
  const items =
    request.limit !== undefined && request.seedItemIds.length
      ? selected.filter((item) => request.seedItemIds.includes(item.id))
      : selected;
  const ids = new Set(items.map((item) => item.id));
  return {
    items,
    topics: demoSnapshot.topics,
    graph: {
      ...demoGraph,
      nodes: demoGraph.nodes.filter((node) => ids.has(node.id)),
      edges: demoGraph.edges.filter(
        (edge) => ids.has(edge.source) && ids.has(edge.target),
      ),
      totalNodeCount: items.length,
    },
  };
}

function unindexedNeighborhood(
  mode: ResearchNeighborhood["mode"],
  seedItemIds: string[],
): ResearchNeighborhood {
  const selected = projection({ seedItemIds, limit: seedItemIds.length });
  return {
    mode,
    projection: { ...selected, graph: { ...selected.graph, edges: [] } },
    totalRelationships: 0,
    hiddenRelationships: 0,
  };
}

const workspace: WorkspaceService = {
  load: async () => browserWorkspace,
  sync: async () => browserWorkspace,
  rebuild: async () => browserWorkspace,
  syncCorpus: desktopRequired,
  rebuildCorpus: desktopRequired,
  setActiveLibrary: async () => browserWorkspace,
  setMode: (mode) => {
    writeSourceMode(window.localStorage, mode);
    window.location.reload();
  },
};
const library: LibraryAdapter = {
  loadSnapshot: async () => demoSnapshot,
  getItem: async (id) => demoItems.find((item) => item.id === id) ?? null,
  query: async (request) => {
    const items = selectedItems(request);
    const offset = request.offset ?? 0;
    const limit = request.limit ?? 50;
    return {
      items: items.slice(offset, offset + limit),
      total: items.length,
      offset,
      limit,
    };
  },
};
const graph: GraphService = {
  loadGraph: async () => demoGraph,
  loadProjection: async (request) => projection(request),
};
const search: SearchService = {
  search: async (
    query,
    limit = 20,
    offset = 0,
    citationFilter = "all",
    mode = "keyword",
  ) => {
    const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
    const results =
      citationFilter !== "all" || mode === "semantic"
        ? []
        : demoItems
            .filter((item) => {
              const text = [
                item.title,
                item.doi,
                item.venue,
                ...item.tags,
                ...item.authors.map((author) => author.name),
              ]
                .join(" ")
                .toLocaleLowerCase();
              return (
                terms.length > 0 && terms.every((term) => text.includes(term))
              );
            })
            .map((item) => ({
              item,
              score: 1,
              reasons: ["Bibliographic metadata match"],
              snippet: null,
              matchSource: "metadata",
              retrievalModes: ["keyword" as const],
              ranking: {
                lexicalRank: null,
                semanticRank: null,
                lexicalScore: 1,
                semanticScore: null,
                fusionScore: null,
                exactMatch: null,
              },
              passages: [],
            }));
    return {
      results: results.slice(offset, offset + limit),
      total: results.length,
      offset,
      limit,
      corpusStatus: "metadata-only",
      retrievalMode: "keyword",
      semanticIndexedPapers: 0,
      semanticEligiblePapers: demoItems.length,
    };
  },
};
const readerStates = new Map<string, ReaderState>();
const document: DocumentService = {
  indexExtraction: desktopRequired,
  doclingStatus: async (documentId) => ({
    available: false,
    message: desktopMessage,
    job: { documentId, status: "unavailable", message: desktopMessage },
    choices: [],
  }),
  extractWithDocling: desktopRequired,
  cancelDocling: async () => {},
  selectExtraction: desktopRequired,
  resolve: async (documentId) => ({
    documentId,
    status: "unavailable",
    mediaType: "application/pdf",
    contentLength: null,
    revisionKey: null,
    message: desktopMessage,
    url: null,
  }),
  getText: async () => null,
  getReaderState: async (id) => readerStates.get(id) ?? null,
  saveReaderState: async (state) => {
    readerStates.set(state.documentId, state);
  },
};
const citations: CitationService = {
  index: desktopRequired,
  rebuild: desktopRequired,
  reresolve: desktopRequired,
  references: async (itemId, _filter, limit = 20, offset = 0) => ({
    coverage: {
      itemId,
      extractionStatus: "not-indexed",
      totalReferences: 0,
      resolvedLocal: 0,
      external: 0,
      ambiguous: 0,
      unresolved: 0,
      citedByLocal: 0,
    },
    references: [],
    total: 0,
    offset,
    limit,
  }),
  citedBy: async (_id, limit = 20, offset = 0) => ({
    items: [],
    total: 0,
    offset,
    limit,
  }),
  sharedReferences: async () => ({ links: [], total: 0 }),
  coCited: async () => ({ links: [], total: 0 }),
  neighborhood: async (ids, mode) => unindexedNeighborhood(mode, ids),
  evidence: async () => null,
  connectionDetails: async (edge) =>
    edge.relation === "shared-tag" && edge.evidenceLabel
      ? [
          edge.evidenceLabel,
          "Themes describe this collection; they are not semantic similarity or citation evidence.",
        ]
      : [],
  review: desktopRequired,
  missingWorks: async (limit = 20, offset = 0) => ({
    works: [],
    total: 0,
    offset,
    limit,
  }),
};
const semantic: SemanticService = {
  installModel: desktopRequired,
  index: desktopRequired,
  rebuild: desktopRequired,
  cancel: async () => {},
  similar: async (seedItemIds) => ({
    seedItemIds,
    results: [],
    indexedPapers: 0,
    eligiblePapers: demoItems.length,
  }),
  coverage: async (itemId) => ({
    itemId,
    paperStatus: "not-indexed",
    paperRepresentation: null,
    paperCoverage: null,
    documentCount: 1,
    chunkCount: 0,
    indexedChunkCount: 0,
    embeddingProfile: null,
  }),
  neighborhood: async (ids) => unindexedNeighborhood("semantic", ids),
  evidence: async () => null,
};
const agent: ResearchAgentService = {
  environment: async () => browserAgentEnvironment,
  saveSettings: desktopRequired,
  installModel: desktopRequired,
  unloadModel: async () => {},
  checkExternal: desktopRequired,
  run: desktopRequired,
  cancel: async () => {},
  conversations: async () => [],
  conversation: desktopRequired,
  deleteConversation: desktopRequired,
  exportMarkdown: desktopRequired,
};
const analytics: AnalyticsService = {
  compute: async () => browserWorkspace.analytics,
};
const lenses: LensService = {
  list: async () => [],
  save: desktopRequired,
  delete: desktopRequired,
};

export const browserDemoServices = {
  workspace,
  library,
  graph,
  search,
  document,
  citations,
  semantic,
  analytics,
  lenses,
  agent,
};
