import type { GraphEdge } from "../../domain/graph";
import { Channel, convertFileSrc, invoke } from "@tauri-apps/api/core";

import type { ResearchGraph } from "../../domain/graph";
import type {
  LibrarySnapshot,
  ResearchItem,
  ResearchLens,
} from "../../domain/research";
import type {
  AgentRequest,
  AgentEnvironment,
  AgentRunResult,
  AgentSettings,
  AgentStreamEvent,
  AnalyticsService,
  AnalyticsSnapshot,
  CitationItemPage,
  CitationService,
  CoCitedWorkPage,
  DocumentOpenResult,
  DocumentService,
  DoclingStatus,
  DocumentTextPayload,
  GraphProjection,
  GraphProjectionRequest,
  GraphService,
  ExternalWorkPage,
  ItemPage,
  ItemQueryRequest,
  LensService,
  SaveLensRequest,
  LibraryAdapter,
  ResearchAgentService,
  ConversationDetail,
  ConversationSummary,
  ExternalProviderConfig,
  ModelInstallEvent,
  ReaderState,
  ReferenceFilter,
  ReferencePage,
  RelationshipEvidence,
  ResearchNeighborhood,
  NeighborhoodMode,
  SearchPage,
  SearchMode,
  SearchService,
  SemanticService,
  SimilarPaperPage,
  PaperSemanticCoverage,
  SemanticRelationshipEvidence,
  WorkspacePayload,
  WorkspaceService,
  StructuralPaperPage,
} from "../../services/contracts";
import { type SourceMode, writeSourceMode } from "../../services/storage";

class TauriWorkspaceBridge implements WorkspaceService {
  private cached: Promise<WorkspacePayload> | null = null;

  constructor(private readonly mode: SourceMode) {}

  load(force = false): Promise<WorkspacePayload> {
    if (force || !this.cached) {
      this.cached = invoke<WorkspacePayload>(
        this.mode === "demo"
          ? "activate_demo_library"
          : "activate_zotero_library",
      );
    }
    return this.cached;
  }

  invalidate(): void {
    this.cached = null;
  }

  async sync(): Promise<WorkspacePayload> {
    if (this.mode === "demo") return this.syncCorpus();
    const metadata = await invoke<WorkspacePayload>("sync_zotero");
    this.cached = Promise.resolve(metadata);
    try {
      const payload = await this.syncCorpus();
      return payload;
    } catch (reason: unknown) {
      const message = reason instanceof Error ? reason.message : String(reason);
      const payload = {
        ...metadata,
        corpus: { ...metadata.corpus, status: "error", lastError: message },
      };
      this.cached = Promise.resolve(payload);
      return payload;
    }
  }

  async rebuild(): Promise<WorkspacePayload> {
    if (this.mode === "demo") return this.rebuildCorpus();
    const payload = await invoke<WorkspacePayload>("rebuild_zotero_cache");
    this.cached = Promise.resolve(payload);
    try {
      return await this.syncCorpus();
    } catch {
      return payload;
    }
  }

  async syncCorpus(): Promise<WorkspacePayload> {
    const payload = await invoke<WorkspacePayload>("sync_document_corpus");
    this.cached = Promise.resolve(payload);
    return payload;
  }

  async rebuildCorpus(): Promise<WorkspacePayload> {
    const payload = await invoke<WorkspacePayload>("rebuild_document_index");
    this.cached = Promise.resolve(payload);
    return payload;
  }

  async setActiveLibrary(libraryId: string): Promise<WorkspacePayload> {
    const payload = await invoke<WorkspacePayload>("set_active_library", {
      libraryId,
    });
    this.cached = Promise.resolve(payload);
    return payload;
  }

  setMode(mode: "demo" | "zotero"): void {
    writeSourceMode(window.localStorage, mode);
    window.location.reload();
  }

  accept(payload: WorkspacePayload): void {
    this.cached = Promise.resolve(payload);
  }
}

class TauriLibraryAdapter implements LibraryAdapter {
  constructor(private readonly bridge: TauriWorkspaceBridge) {}

  async loadSnapshot(): Promise<LibrarySnapshot> {
    return (await this.bridge.load()).snapshot;
  }

  async getItem(id: string): Promise<ResearchItem | null> {
    const projection = await invoke<GraphProjection>("load_graph_projection", {
      request: { seedItemIds: [id], limit: 1, offset: 0 },
    });
    return projection.items.find((item) => item.id === id) ?? null;
  }

  query(request: ItemQueryRequest): Promise<ItemPage> {
    return invoke<ItemPage>("query_library_items", { request });
  }
}

class TauriGraphService implements GraphService {
  constructor(private readonly bridge: TauriWorkspaceBridge) {}

  async loadGraph(): Promise<ResearchGraph> {
    return (await this.bridge.load()).graph;
  }

  loadProjection(request: GraphProjectionRequest): Promise<GraphProjection> {
    return invoke<GraphProjection>("load_graph_projection", { request });
  }
}

class TauriAnalyticsService implements AnalyticsService {
  constructor(private readonly bridge: TauriWorkspaceBridge) {}

  async compute(): Promise<AnalyticsSnapshot> {
    return (await this.bridge.load()).analytics;
  }
}

class TauriSearchService implements SearchService {
  async search(
    query: string,
    limit = 20,
    offset = 0,
    citationFilter = "all",
    mode: SearchMode = "hybrid",
  ): Promise<SearchPage> {
    return invoke<SearchPage>("search_library", {
      request: { query, limit, offset, citationFilter, mode },
    });
  }
}

class TauriSemanticService implements SemanticService {
  constructor(private readonly bridge: TauriWorkspaceBridge) {}

  async installModel(): Promise<WorkspacePayload> {
    const payload = await invoke<WorkspacePayload>("install_semantic_model");
    this.bridge.accept(payload);
    return payload;
  }

  async index(maxPapers = 24, maxDocuments = 6): Promise<WorkspacePayload> {
    const payload = await invoke<WorkspacePayload>("index_semantic_corpus", {
      request: { maxPapers, maxDocuments },
    });
    this.bridge.accept(payload);
    return payload;
  }

  async rebuild(): Promise<WorkspacePayload> {
    const payload = await invoke<WorkspacePayload>("rebuild_semantic_index");
    this.bridge.accept(payload);
    return payload;
  }

  cancel(): Promise<void> {
    return invoke<void>("cancel_semantic_indexing");
  }

  similar(
    seedItemIds: string[],
    collectionId?: string,
    limit = 24,
    minimumSimilarity = 0.52,
  ): Promise<SimilarPaperPage> {
    return invoke<SimilarPaperPage>("query_similar_papers", {
      request: { seedItemIds, collectionId, limit, minimumSimilarity },
    });
  }

  coverage(itemId: string): Promise<PaperSemanticCoverage> {
    return invoke<PaperSemanticCoverage>("get_paper_semantic_coverage", {
      itemId,
    });
  }

  neighborhood(
    seedItemIds: string[],
    maxNodes = 48,
    collectionId?: string,
  ): Promise<ResearchNeighborhood> {
    return invoke<ResearchNeighborhood>("load_research_neighborhood", {
      collectionId,
      request: {
        seedItemIds,
        mode: "semantic",
        maxNodes,
        maxEdges: Math.min(300, maxNodes * 3),
        offset: 0,
      },
    });
  }

  evidence(edgeId: string): Promise<SemanticRelationshipEvidence | null> {
    return invoke<SemanticRelationshipEvidence | null>(
      "get_semantic_relationship_evidence",
      { edgeId },
    );
  }
}

class TauriDocumentService implements DocumentService {
  indexExtraction(documentId: string): Promise<void> {
    return invoke("index_document_extraction", { documentId });
  }
  doclingStatus(documentId: string): Promise<DoclingStatus> {
    return invoke("get_docling_status", { documentId });
  }
  extractWithDocling(documentId: string): Promise<void> {
    return invoke("extract_document_docling", { documentId });
  }
  cancelDocling(documentId: string): Promise<void> {
    return invoke("cancel_document_docling", { documentId });
  }
  selectExtraction(documentId: string, extractionId: string): Promise<void> {
    return invoke("select_document_extraction", { documentId, extractionId });
  }
  async resolve(documentId: string): Promise<DocumentOpenResult> {
    const result = await invoke<Omit<DocumentOpenResult, "url">>(
      "resolve_document",
      { documentId },
    );
    return {
      ...result,
      url:
        result.status === "ready"
          ? `${convertFileSrc(documentId, "graphite-document")}?revision=${encodeURIComponent(result.revisionKey ?? "")}`
          : null,
    };
  }

  getText(documentId: string): Promise<DocumentTextPayload | null> {
    return invoke<DocumentTextPayload | null>("get_document_text", {
      documentId,
    });
  }

  getReaderState(documentId: string): Promise<ReaderState | null> {
    return invoke<ReaderState | null>("get_reader_state", { documentId });
  }

  saveReaderState(state: ReaderState): Promise<void> {
    return invoke<void>("save_reader_state", { readerState: state });
  }
}

class TauriCitationService implements CitationService {
  constructor(private readonly bridge: TauriWorkspaceBridge) {}

  async index(
    priorityItemId?: string,
    maxDocuments = 32,
  ): Promise<WorkspacePayload> {
    const payload = await invoke<WorkspacePayload>("index_citation_corpus", {
      request: { priorityItemId, maxDocuments },
    });
    this.bridge.accept(payload);
    return payload;
  }

  async rebuild(): Promise<WorkspacePayload> {
    const payload = await invoke<WorkspacePayload>("rebuild_citation_index");
    this.bridge.accept(payload);
    return payload;
  }

  async reresolve(): Promise<WorkspacePayload> {
    const payload = await invoke<WorkspacePayload>("reresolve_citations");
    this.bridge.accept(payload);
    return payload;
  }

  references(
    itemId: string,
    filter: ReferenceFilter = "all",
    limit = 40,
    offset = 0,
  ): Promise<ReferencePage> {
    return invoke<ReferencePage>("query_references", {
      request: { itemId, filter, limit, offset },
    });
  }

  citedBy(itemId: string, limit = 25, offset = 0): Promise<CitationItemPage> {
    return invoke<CitationItemPage>("query_cited_by", {
      request: { itemId, limit, offset },
    });
  }

  sharedReferences(itemId: string, limit = 20): Promise<StructuralPaperPage> {
    return invoke<StructuralPaperPage>("query_shared_references", {
      itemId,
      limit,
    });
  }

  coCited(itemId: string, limit = 20): Promise<CoCitedWorkPage> {
    return invoke<CoCitedWorkPage>("query_co_cited", { itemId, limit });
  }

  neighborhood(
    seedItemIds: string[],
    mode: NeighborhoodMode,
    maxNodes = 80,
    offset = 0,
  ): Promise<ResearchNeighborhood> {
    return invoke<ResearchNeighborhood>("load_research_neighborhood", {
      request: {
        seedItemIds,
        mode,
        maxNodes,
        maxEdges: Math.min(360, maxNodes * 2),
        offset,
      },
    });
  }

  connectionDetails(edge: GraphEdge): Promise<string[]> {
    return invoke<string[]>("get_connection_details", {
      sourceId: edge.source,
      targetId: edge.target,
      relation: edge.relation,
    });
  }

  evidence(edgeId: string): Promise<RelationshipEvidence | null> {
    return invoke<RelationshipEvidence | null>("get_relationship_evidence", {
      request: { edgeId },
    });
  }

  review(
    referenceId: string,
    decision: "accept" | "reject" | "unresolved",
    targetItemId?: string,
  ): Promise<void> {
    return invoke<void>("review_reference_resolution", {
      request: { referenceId, decision, targetItemId },
    });
  }

  missingWorks(limit = 25, offset = 0): Promise<ExternalWorkPage> {
    return invoke<ExternalWorkPage>("query_missing_works", {
      request: { limit, offset },
    });
  }
}

class TauriLensService implements LensService {
  constructor(private readonly bridge: TauriWorkspaceBridge) {}

  list(libraryId: string): Promise<ResearchLens[]> {
    return invoke("list_lenses", { libraryId });
  }

  async save(request: SaveLensRequest): Promise<ResearchLens> {
    const lens = await invoke<ResearchLens>("save_lens", { request });
    this.bridge.invalidate();
    return lens;
  }

  async delete(libraryId: string, id: string, revision: number): Promise<void> {
    await invoke("delete_lens", { libraryId, id, revision });
    this.bridge.invalidate();
  }
}

class TauriResearchAgentService implements ResearchAgentService {
  environment(): Promise<AgentEnvironment> {
    return invoke<AgentEnvironment>("get_agent_environment");
  }

  saveSettings(settings: AgentSettings): Promise<AgentEnvironment> {
    return invoke<AgentEnvironment>("save_agent_settings", { settings });
  }

  installModel(
    profileId: string,
    onEvent: (event: ModelInstallEvent) => void,
  ): Promise<AgentEnvironment> {
    const channel = new Channel<ModelInstallEvent>();
    channel.onmessage = onEvent;
    return invoke<AgentEnvironment>("install_agent_model", {
      profileId,
      onEvent: channel,
    });
  }

  unloadModel(): Promise<void> {
    return invoke<void>("unload_agent_model");
  }

  checkExternal(config: ExternalProviderConfig): Promise<string> {
    return invoke<string>("check_external_agent_provider", { config });
  }

  run(
    request: AgentRequest,
    onEvent: (event: AgentStreamEvent) => void,
  ): Promise<AgentRunResult> {
    const channel = new Channel<AgentStreamEvent>();
    channel.onmessage = onEvent;
    return invoke<AgentRunResult>("run_research_agent", {
      request,
      onEvent: channel,
    });
  }

  cancel(): Promise<void> {
    return invoke<void>("cancel_research_agent");
  }

  conversations(): Promise<ConversationSummary[]> {
    return invoke<ConversationSummary[]>("list_agent_conversations");
  }

  conversation(conversationId: string): Promise<ConversationDetail> {
    return invoke<ConversationDetail>("load_agent_conversation", {
      conversationId,
    });
  }

  deleteConversation(conversationId: string): Promise<void> {
    return invoke<void>("delete_agent_conversation", { conversationId });
  }

  exportMarkdown(conversationId: string): Promise<string> {
    return invoke<string>("export_agent_conversation", { conversationId });
  }
}

export function createTauriServices(mode: SourceMode = "zotero") {
  const bridge = new TauriWorkspaceBridge(mode);
  return {
    workspace: bridge,
    library: new TauriLibraryAdapter(bridge),
    graph: new TauriGraphService(bridge),
    search: new TauriSearchService(),
    document: new TauriDocumentService(),
    citations: new TauriCitationService(bridge),
    semantic: new TauriSemanticService(bridge),
    analytics: new TauriAnalyticsService(bridge),
    lenses: new TauriLensService(bridge),
    agent: new TauriResearchAgentService(),
  };
}
