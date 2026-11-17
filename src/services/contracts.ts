import type {
  ResearchGraph,
  GraphEdge,
  GraphRelationType,
} from "../domain/graph";
import type {
  LibrarySnapshot,
  ResearchItem,
  ResearchLens,
  TopicId,
} from "../domain/research";

export interface LibraryAdapter {
  loadSnapshot(): Promise<LibrarySnapshot>;
  getItem(id: string): Promise<ResearchItem | null>;
  query(request: ItemQueryRequest): Promise<ItemPage>;
}

export interface ItemQueryRequest {
  collectionId?: string;
  itemType?: string;
  yearFrom?: number;
  yearTo?: number;
  tag?: string;
  sort?: "title" | "year-desc";
  limit?: number;
  offset?: number;
}

export interface ItemPage {
  items: ResearchItem[];
  total: number;
  offset: number;
  limit: number;
}

export interface SearchResult {
  item: ResearchItem;
  score: number;
  reasons: string[];
  snippet: string | null;
  matchSource: string;
  retrievalModes: SearchMode[];
  ranking: RetrievalScoreBreakdown;
  passages: SemanticPassage[];
}

export interface SearchPage {
  results: SearchResult[];
  total: number;
  offset: number;
  limit: number;
  corpusStatus: string;
  retrievalMode: SearchMode;
  semanticIndexedPapers: number;
  semanticEligiblePapers: number;
}

export type SearchMode = "keyword" | "semantic" | "hybrid";

export interface RetrievalScoreBreakdown {
  lexicalRank: number | null;
  semanticRank: number | null;
  lexicalScore: number | null;
  semanticScore: number | null;
  fusionScore: number | null;
  exactMatch: string | null;
}

export interface SemanticPassage {
  chunkId: string;
  documentId: string;
  documentRevisionId: number;
  snippet: string;
  characterStart: number;
  characterEnd: number;
  pageStart: number | null;
  pageEnd: number | null;
  semanticScore: number;
  embeddingProfileId: string;
}

export interface SearchService {
  search(
    query: string,
    limit?: number,
    offset?: number,
    citationFilter?: CitationSearchFilter,
    mode?: SearchMode,
  ): Promise<SearchPage>;
}

export type CitationSearchFilter =
  | "all"
  | "references-indexed"
  | "cited-by-local"
  | "has-external"
  | "needs-review";

export interface GraphService {
  loadGraph(): Promise<ResearchGraph>;
  loadProjection(request: GraphProjectionRequest): Promise<GraphProjection>;
}

export interface GraphProjectionRequest {
  collectionId?: string;
  seedItemIds: string[];
  /** Omit to load every paper in the active library or collection. */
  limit?: number;
  offset?: number;
}

export interface GraphProjection {
  items: ResearchItem[];
  topics: LibrarySnapshot["topics"];
  graph: ResearchGraph;
}

export interface DocumentOpenResult {
  documentId: string;
  status: "ready" | "unavailable" | "unsupported";
  mediaType: string;
  contentLength: number | null;
  revisionKey: string | null;
  message: string | null;
  url: string | null;
}

export interface DocumentTextPayload {
  documentId: string;
  provider: string;
  content: string;
  indexedPages: number | null;
  totalPages: number | null;
  indexedChars: number | null;
  totalChars: number | null;
  completeness: "full" | "partial";
}

export interface ReaderState {
  documentId: string;
  pageNumber: number;
  zoom: number;
  fitMode: "width" | "page" | "custom";
  outlineOpen: boolean;
  contextOpen: boolean;
}

export interface DoclingStatus {
  available: boolean;
  message: string;
  job: { documentId: string; status: string; message: string };
  choices: {
    id: string;
    provider: string;
    createdAt: string;
    active: boolean;
  }[];
}

export interface DocumentService {
  indexExtraction(documentId: string): Promise<void>;
  doclingStatus(documentId: string): Promise<DoclingStatus>;
  extractWithDocling(documentId: string): Promise<void>;
  cancelDocling(documentId: string): Promise<void>;
  selectExtraction(documentId: string, extractionId: string): Promise<void>;
  resolve(documentId: string): Promise<DocumentOpenResult>;
  getText(documentId: string): Promise<DocumentTextPayload | null>;
  getReaderState(documentId: string): Promise<ReaderState | null>;
  saveReaderState(state: ReaderState): Promise<void>;
}

export interface TopicMetric {
  topicId: TopicId;
  count: number;
}

export interface YearMetric {
  year: number;
  count: number;
}

export interface AnalyticsSnapshot {
  byTopic: TopicMetric[];
  byYear: YearMetric[];
  mostConnected: ResearchItem[];
  pdfCoverage: number;
  documentCount: number;
  resolvedPdfCount: number;
  fullTextCount: number;
  partialTextCount: number;
  indexedPages: number;
  citation: CitationAnalytics;
}

export interface CitationPaperMetric {
  itemId: string;
  title: string;
  count: number;
}

export interface ExternalWorkMetric {
  workId: string;
  title: string;
  year: number | null;
  doi: string | null;
  referencedByCount: number;
}

export interface CitationAnalytics {
  mostCitedLocal: CitationPaperMetric[];
  mostReferencing: CitationPaperMetric[];
  frequentlyReferencedExternal: ExternalWorkMetric[];
  resolutionCoverage: number;
}

export interface AnalyticsService {
  compute(): Promise<AnalyticsSnapshot>;
}

export type SourceHealth =
  | "demo"
  | "not-synced"
  | "ready"
  | "offline-cache"
  | "syncing"
  | "error"
  | "incompatible";

export interface SyncSummary {
  status: "running" | "complete" | "failed";
  librariesScanned: number;
  objectsChanged: number;
  objectsDeleted: number;
  normalizationWarnings: number;
  startedAt: string;
  finishedAt: string | null;
}

export interface SourceDiagnostics {
  mode: "demo" | "zotero";
  health: SourceHealth;
  connectionStatus: string;
  serverId: string | null;
  apiVersion: string | null;
  schemaVersion: string | null;
  lastSyncedAt: string | null;
  lastError: string | null;
  activeLibraryId: string | null;
  sourceChanged: boolean;
  syncSummary: SyncSummary | null;
}

export interface WorkspacePayload {
  snapshot: LibrarySnapshot;
  graph: ResearchGraph;
  analytics: AnalyticsSnapshot;
  source: SourceDiagnostics;
  corpus: CorpusDiagnostics;
  extraction: ExtractionDiagnostics;
  citations: CitationDiagnostics;
  semantic: SemanticDiagnostics;
}

export interface EmbeddingProfileSummary {
  id: string;
  provider: string;
  modelIdentifier: string;
  modelRevision: string;
  artifactIdentity: string;
  embeddingDimension: number;
  similarityMetric: string;
  queryTemplate: string;
  documentTemplate: string;
  normalization: string;
  runtimeVersion: string;
  vectorBackend: string;
  vectorIndexFormatVersion: string;
}

export interface ChunkingProfileSummary {
  id: string;
  strategy: string;
  version: string;
  targetTokens: number;
  overlapTokens: number;
  maximumTokens: number;
  maximumChunksPerDocument: number;
  boundaryPolicy: string;
}

export interface RetrievalProfileSummary {
  id: string;
  maximumCandidatesPerVectorIndex: number;
  reciprocalRankFusionK: number;
  lexicalWeight: number;
  semanticWeight: number;
  maximumPassagesPerPaper: number;
  scopedPath: string;
  scopedChunkLimitPerPaper: number | null;
  contextAssembler: string;
  maximumEvidenceSources: number;
  compactEvidenceBudgetTokens: number;
  largeEvidenceBudgetTokens: number;
  tokenAccounting: string;
}

export interface SemanticDiagnostics {
  status: string;
  indexing: boolean;
  modelInstalled: boolean;
  modelIdentifier: string;
  modelDisplayName: string;
  modelSource: string;
  modelLicense: string;
  modelDownloadBytes: number;
  modelStorageBytes: number;
  modelRevision: string | null;
  activeProfile: EmbeddingProfileSummary | null;
  activeChunkingProfile: ChunkingProfileSummary;
  activeRetrievalProfile: RetrievalProfileSummary;
  vectorBackend: string;
  eligiblePapers: number;
  indexedPapers: number;
  stalePapers: number;
  eligibleDocuments: number;
  chunkedDocuments: number;
  generatedChunks: number;
  indexedChunks: number;
  staleChunks: number;
  currentDocumentRevisionsIndexed: number;
  staleDocumentRevisionsIndexed: number;
  maximumObservedChunksPerDocument: number;
  documentsAtChunkLimit: number;
  registeredPaperVectors: number;
  physicalPaperVectors: number | null;
  paperVectorCountMatches: boolean | null;
  registeredChunkVectors: number;
  physicalChunkVectors: number | null;
  chunkVectorCountMatches: boolean | null;
  metadataOnlyPapers: number;
  vectorIndexBytes: number;
  chunkMetadataBytes: number;
  lastIndexedAt: string | null;
  lastError: string | null;
}

export interface CorpusDiagnostics {
  status: string;
  documentCount: number;
  pdfCount: number;
  resolvedPdfCount: number;
  fullTextCount: number;
  partialTextCount: number;
  indexedPages: number;
  successfulVersion: number;
  lastSyncedAt: string | null;
  changedDocuments: number;
  warningCount: number;
  lastError: string | null;
}

export interface ExtractionDiagnostics {
  status: string;
  pending: number;
  good: number;
  degraded: number;
  failed: number;
  fallback: number;
  unsupported: number;
  unavailable: number;
  activeProviderProfile: string | null;
  activeRoutingProfile: string;
  activeQualityProfile: string;
  shadow: ShadowExtractionDiagnostics;
}

export interface ShadowExtractionDiagnostics {
  enabled: boolean;
  availability: string;
  routingProfile: string;
  pending: number;
  running: number;
  completed: number;
  failed: number;
  cancelled: number;
  unavailable: number;
  good: number;
  degraded: number;
  qualityFailed: number;
  providers: string[];
  lastProviderProfile: string | null;
  lastRoutingReason: string | null;
  lastDurationMs: number | null;
  lastPeakRssBytes: number | null;
  fallbackRoutes: number;
  comparisonDiscrepancies: number;
}

export interface CitationDiagnostics {
  status: string;
  eligibleDocuments: number;
  indexedDocuments: number;
  noReferenceSection: number;
  partialDocuments: number;
  failedDocuments: number;
  referencesExtracted: number;
  referencesWithDoi: number;
  resolvedLocal: number;
  externalWorks: number;
  ambiguousReferences: number;
  unresolvedReferences: number;
  citationEdges: number;
  queuedDocuments: number;
  queuedResolutions: number;
  lastIndexedAt: string | null;
  extractorVersion: string;
  resolverVersion: string;
  lastError: string | null;
}

export type ReferenceFilter =
  "all" | "local" | "external" | "ambiguous" | "unresolved";

export interface CitationCoverage {
  itemId: string;
  extractionStatus: string;
  totalReferences: number;
  resolvedLocal: number;
  external: number;
  ambiguous: number;
  unresolved: number;
  citedByLocal: number;
}

export interface CitationItemSummary {
  id: string;
  title: string;
  firstAuthor: string | null;
  year: number | null;
  venue: string | null;
  doi: string | null;
}

export interface ExternalWorkSummary {
  id: string;
  identityKind: string;
  title: string | null;
  firstAuthor: string | null;
  year: number | null;
  venue: string | null;
  doi: string | null;
  referencedByCount: number;
}

export interface ResolutionCandidate {
  item: CitationItemSummary;
  category: string;
  score: number;
  evidence: string[];
}

export interface BibliographicReferenceView {
  id: string;
  ordinal: number;
  rawText: string;
  originalDoi: string | null;
  normalizedDoi: string | null;
  parsedTitle: string | null;
  parsedYear: number | null;
  firstAuthor: string | null;
  parsedVenue: string | null;
  extractionConfidence: string;
  pageNumber: number | null;
  sourceStart: number | null;
  sourceEnd: number | null;
  resolutionStatus: string;
  resolutionMethod: string;
  confidenceCategory: string;
  resolutionEvidence: string[];
  matchedItem: CitationItemSummary | null;
  externalWork: ExternalWorkSummary | null;
  candidates: ResolutionCandidate[];
  overrideStale: boolean;
}

export interface ReferencePage {
  coverage: CitationCoverage;
  references: BibliographicReferenceView[];
  total: number;
  offset: number;
  limit: number;
}

export interface CitationItemPage {
  items: CitationItemSummary[];
  total: number;
  offset: number;
  limit: number;
}

export interface StructuralPaperLink {
  item: CitationItemSummary;
  count: number;
  normalizedStrength: number;
  label: string;
}

export interface StructuralPaperPage {
  links: StructuralPaperLink[];
  total: number;
}

export interface CoCitedWorkLink {
  localItem: CitationItemSummary | null;
  externalWork: ExternalWorkSummary | null;
  count: number;
  label: string;
}

export interface CoCitedWorkPage {
  links: CoCitedWorkLink[];
  total: number;
}

export type NeighborhoodMode =
  | "references"
  | "cited-by"
  | "citation"
  | "shared-references"
  | "co-cited"
  | "semantic";

export interface ResearchNeighborhood {
  projection: GraphProjection;
  mode: NeighborhoodMode;
  totalRelationships: number;
  hiddenRelationships: number;
}

export interface CitationEvidenceView {
  referenceId: string;
  ordinal: number;
  rawText: string;
  resolutionMethod: string;
  confidenceCategory: string;
  sourceDocumentTitle: string;
  sourceDocumentId: string;
  pageNumber: number | null;
  extractorProvider: string;
  extractorVersion: string;
  resolverVersion: string;
}

export interface RelationshipEvidence {
  edgeId: string;
  relation: Exclude<GraphRelationType, "semantically-similar">;
  details?: string[];
  directed?: boolean;
  source: CitationItemSummary;
  target: CitationItemSummary;
  evidenceLabel: string;
  evidence: CitationEvidenceView[];
}

export interface ExternalWorkPage {
  works: ExternalWorkSummary[];
  total: number;
  offset: number;
  limit: number;
}

export interface CitationService {
  index(
    priorityItemId?: string,
    maxDocuments?: number,
  ): Promise<WorkspacePayload>;
  rebuild(): Promise<WorkspacePayload>;
  reresolve(): Promise<WorkspacePayload>;
  references(
    itemId: string,
    filter?: ReferenceFilter,
    limit?: number,
    offset?: number,
  ): Promise<ReferencePage>;
  citedBy(
    itemId: string,
    limit?: number,
    offset?: number,
  ): Promise<CitationItemPage>;
  sharedReferences(
    itemId: string,
    limit?: number,
  ): Promise<StructuralPaperPage>;
  coCited(itemId: string, limit?: number): Promise<CoCitedWorkPage>;
  neighborhood(
    seedItemIds: string[],
    mode: NeighborhoodMode,
    maxNodes?: number,
    offset?: number,
  ): Promise<ResearchNeighborhood>;
  evidence(edgeId: string): Promise<RelationshipEvidence | null>;
  connectionDetails(edge: GraphEdge): Promise<string[]>;
  review(
    referenceId: string,
    decision: "accept" | "reject" | "unresolved",
    targetItemId?: string,
  ): Promise<void>;
  missingWorks(limit?: number, offset?: number): Promise<ExternalWorkPage>;
}

export interface SimilarPaper {
  item: ResearchItem;
  similarityScore: number;
  similarityLabel: string;
  sourceRepresentation: string;
  targetRepresentation: string;
  embeddingProfile: EmbeddingProfileSummary;
}

export interface SimilarPaperPage {
  results: SimilarPaper[];
  seedItemIds: string[];
  indexedPapers: number;
  eligiblePapers: number;
}

export interface PaperSemanticCoverage {
  itemId: string;
  paperStatus: string;
  paperRepresentation: string | null;
  paperCoverage: string | null;
  documentCount: number;
  chunkCount: number;
  indexedChunkCount: number;
  embeddingProfile: EmbeddingProfileSummary | null;
}

export interface SemanticRelationshipEvidence {
  edgeId: string;
  relation: "semantically-similar";
  source: CitationItemSummary;
  target: CitationItemSummary;
  sourceRepresentation: string;
  targetRepresentation: string;
  similarityMetric: string;
  similarityScore: number;
  similarityLabel: string;
  mutual: boolean;
  embeddingProfile: EmbeddingProfileSummary;
  passages: SemanticPassage[];
}

export interface SemanticService {
  installModel(): Promise<WorkspacePayload>;
  index(maxPapers?: number, maxDocuments?: number): Promise<WorkspacePayload>;
  rebuild(): Promise<WorkspacePayload>;
  cancel(): Promise<void>;
  similar(
    seedItemIds: string[],
    collectionId?: string,
    limit?: number,
    minimumSimilarity?: number,
  ): Promise<SimilarPaperPage>;
  coverage(itemId: string): Promise<PaperSemanticCoverage>;
  neighborhood(
    seedItemIds: string[],
    maxNodes?: number,
    collectionId?: string,
  ): Promise<ResearchNeighborhood>;
  evidence(edgeId: string): Promise<SemanticRelationshipEvidence | null>;
}

export interface WorkspaceService {
  load(force?: boolean): Promise<WorkspacePayload>;
  sync(): Promise<WorkspacePayload>;
  rebuild(): Promise<WorkspacePayload>;
  syncCorpus(): Promise<WorkspacePayload>;
  rebuildCorpus(): Promise<WorkspacePayload>;
  setActiveLibrary(libraryId: string): Promise<WorkspacePayload>;
  setMode(mode: "demo" | "zotero"): void;
}

export type AgentProviderKind = "graphite-managed" | "openai-compatible";
export type AgentDetail = "concise" | "detailed";
export type AgentIntent =
  | "paper-discovery"
  | "current-paper-qa"
  | "multi-paper-comparison"
  | "structured-extraction"
  | "citation-structure"
  | "library-synthesis";

export interface AgentContext {
  selectedOnly?: boolean;
  lensOnly?: boolean;
  currentItemId?: string;
  currentDocumentId?: string;
  currentPage?: number;
  selectedItemIds: string[];
  activeCollectionId?: string;
  activeLensId?: string;
  activeLensItemIds: string[];
  graphNeighborhoodItemIds: string[];
  removedContextKinds: string[];
}

export interface ExternalProviderConfig {
  endpoint: string;
  model: string;
  contextTokens: number;
  apiKey?: string;
}

export type InferenceBackend = "auto" | "cpu" | "gpu";
export type ContextWindow = "low" | "medium" | "high" | "extra-high";

export interface AgentRequest {
  question: string;
  conversationId?: string;
  providerKind: AgentProviderKind;
  modelProfile: string;
  inferenceBackend?: InferenceBackend;
  contextWindow?: ContextWindow;
  detail: AgentDetail;
  context: AgentContext;
  externalProvider?: ExternalProviderConfig;
}

export interface AgentSettings {
  providerKind: AgentProviderKind;
  modelProfile: string;
  inferenceBackend?: InferenceBackend;
  contextWindow?: ContextWindow;
  externalEndpoint: string | null;
  externalModel: string | null;
  externalContextTokens: number | null;
  answerDetail: AgentDetail;
}

export interface ComputeDevice {
  index: number;
  name: string;
  backend: string;
  kind: "discrete" | "integrated";
  totalMemoryBytes: number;
  availableMemoryBytes: number;
}

export interface HardwareProfile {
  cpuName?: string;
  compiledBackend?: string;
  executionStatus?: string;
  gpuDevices?: ComputeDevice[];
  graphicsAdapters?: string[];
  logicalCpus: number;
  totalMemoryBytes: number;
  availableMemoryBytes: number;
  operatingSystem: string;
  architecture: string;
  accelerationBackend: string;
  accelerationAvailable: boolean;
  recommendedProfile: string;
  note: string;
}

export interface ModelProfile {
  id: string;
  label: string;
  modelIdentifier: string;
  modelRevision: string;
  artifactFilename: string;
  artifactIdentity: string;
  quantization: string;
  expectedBytes: number;
  sha256: string;
  contextCapacity: number;
  license: string;
  installed: boolean;
  status: string;
  installedBytes: number;
  lastError: string | null;
}

export interface AgentEnvironment {
  settings: AgentSettings;
  hardware: HardwareProfile;
  modelProfiles: ModelProfile[];
  runtimeName: string;
  runtimeVersion: string;
  trustStatement: string;
  toolBudget: number;
  contextBudgetTokens: number;
  rerankerStatus: string;
}

export interface AgentEvidence {
  sourceId: string;
  kind: string;
  itemId: string | null;
  itemTitle: string;
  documentId: string | null;
  documentRevisionId: number | null;
  chunkId: string | null;
  excerpt: string | null;
  characterStart: number | null;
  characterEnd: number | null;
  pageStart: number | null;
  pageEnd: number | null;
  retrievalModes: string[];
  lexicalRank: number | null;
  semanticRank: number | null;
  provider: string;
  sourceFingerprint: string;
  eligibleForScientificClaim: boolean;
  stale: boolean;
}

export interface AgentCitation {
  sourceId: string;
  marker: string;
  claimIndex: number;
}

export interface AgentAction {
  kind: string;
  label: string;
  query: string | null;
  itemIds: string[];
}

export interface AgentToolCall {
  tool: string;
  purpose: string;
  resultCount: number;
  durationMs: number;
}

export interface AgentTimings {
  retrievalMs: number;
  contextMs: number;
  timeToFirstTokenMs: number | null;
  generationMs: number;
  totalMs: number;
}

export interface AgentRunResult {
  runId: string;
  conversationId: string;
  question: string;
  intent: AgentIntent;
  answerMarkdown: string;
  status: string;
  providerLabel: string;
  modelLabel: string;
  contextLabel: string;
  contextTokens: number;
  generationTokens: number;
  sources: AgentEvidence[];
  citations: AgentCitation[];
  limitations: string[];
  actions: AgentAction[];
  toolCalls: AgentToolCall[];
  promptVersions: string[];
  timings: AgentTimings;
}

export interface AgentStreamEvent {
  kind: "stage" | "delta" | "reset" | "complete" | "cancelled";
  stage: string | null;
  message: string | null;
  delta: string | null;
  result: AgentRunResult | null;
}

export interface ModelInstallEvent {
  kind: string;
  profileId: string;
  message: string;
}

export interface ConversationSummary {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  turnCount: number;
}

export interface ConversationTurn {
  id: string;
  runId: string | null;
  role: "user" | "assistant";
  content: string;
  status: string;
  createdAt: string;
}

export interface ConversationDetail {
  summary: ConversationSummary;
  turns: ConversationTurn[];
  runs: AgentRunResult[];
}

export interface ResearchAgentService {
  environment(): Promise<AgentEnvironment>;
  saveSettings(settings: AgentSettings): Promise<AgentEnvironment>;
  installModel(
    profileId: string,
    onEvent: (event: ModelInstallEvent) => void,
  ): Promise<AgentEnvironment>;
  unloadModel(): Promise<void>;
  checkExternal(config: ExternalProviderConfig): Promise<string>;
  run(
    request: AgentRequest,
    onEvent: (event: AgentStreamEvent) => void,
  ): Promise<AgentRunResult>;
  cancel(): Promise<void>;
  conversations(): Promise<ConversationSummary[]>;
  conversation(conversationId: string): Promise<ConversationDetail>;
  deleteConversation(conversationId: string): Promise<void>;
  exportMarkdown(conversationId: string): Promise<string>;
}

export interface SaveLensRequest {
  id?: string;
  revision?: number;
  libraryId: string;
  name: string;
  description: string;
  paperIds: string[];
}

export interface LensService {
  list(libraryId: string): Promise<ResearchLens[]>;
  save(request: SaveLensRequest): Promise<ResearchLens>;
  delete(libraryId: string, id: string, revision: number): Promise<void>;
}
