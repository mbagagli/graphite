use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePayload {
    pub snapshot: LibrarySnapshot,
    pub graph: ResearchGraph,
    pub analytics: AnalyticsSnapshot,
    pub source: SourceDiagnostics,
    pub corpus: CorpusDiagnostics,
    pub extraction: ExtractionDiagnostics,
    pub citations: CitationDiagnostics,
    pub semantic: SemanticDiagnostics,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySnapshot {
    pub items: Vec<ResearchItem>,
    pub collections: Vec<ResearchCollection>,
    pub topics: Vec<Topic>,
    pub lenses: Vec<ResearchLens>,
    pub libraries: Vec<LibraryDescriptor>,
    pub active_library_id: Option<String>,
    pub provenance: String,
    pub total_item_count: u64,
    pub loaded_item_count: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDescriptor {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub item_count: u64,
    pub last_synced_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchCollection {
    pub id: String,
    pub name: String,
    pub description: String,
    pub parent_id: Option<String>,
    pub library_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Topic {
    pub id: String,
    pub name: String,
    pub short_name: String,
    pub color: String,
    pub origin: String,
    #[serde(default)]
    pub analyzed: Option<usize>,
    #[serde(default)]
    pub eligible: Option<usize>,
    #[serde(default)]
    pub representatives: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub id: String,
    pub name: String,
    pub creator_type: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentAttachment {
    pub id: String,
    pub document_id: String,
    pub media_type: String,
    pub page_count: Option<u32>,
    pub availability: String,
    pub title: String,
    pub filename: Option<String>,
    pub link_mode: Option<String>,
    pub source_version: i64,
    pub text: Option<DocumentTextSummary>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTextSummary {
    pub provider: String,
    pub status: String,
    pub indexed_pages: Option<u32>,
    pub total_pages: Option<u32>,
    pub indexed_chars: Option<u64>,
    pub total_chars: Option<u64>,
    pub source_version: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchNote {
    pub id: String,
    pub title: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemProvenance {
    pub source_id: String,
    pub library_id: String,
    pub zotero_key: String,
    pub zotero_item_type: String,
    pub source_version: i64,
    pub date_added: Option<String>,
    pub date_modified: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchItem {
    pub id: String,
    pub title: String,
    pub authors: Vec<Author>,
    pub year: Option<i32>,
    pub date: Option<String>,
    pub venue: Option<String>,
    pub abstract_text: Option<String>,
    pub topics: Vec<String>,
    pub methods: Vec<String>,
    pub tags: Vec<String>,
    pub collection_ids: Vec<String>,
    pub citation_count: u32,
    pub importance: f64,
    pub attachments: Vec<DocumentAttachment>,
    pub notes: Vec<ResearchNote>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub pages: Option<String>,
    pub publisher: Option<String>,
    pub doi: Option<String>,
    pub isbn: Option<String>,
    pub issn: Option<String>,
    pub url: Option<String>,
    pub provenance: ItemProvenance,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchLens {
    pub library_id: String,
    pub revision: i64,
    pub unavailable_paper_ids: Vec<String>,
    pub paper_titles: std::collections::HashMap<String, String>,
    pub id: String,
    pub name: String,
    pub description: String,
    pub paper_ids: Vec<String>,
    pub groups: Vec<String>,
    pub saved: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub paper_id: Option<String>,
    pub entity_kind: String,
    pub label: String,
    pub detail: Option<String>,
    pub topic_id: String,
    pub x: f64,
    pub y: f64,
    pub importance: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
    pub weight: f64,
    pub directed: bool,
    pub evidence_count: u32,
    pub evidence_label: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub available_relations: Vec<String>,
    pub total_node_count: u64,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicMetric {
    pub topic_id: String,
    pub count: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YearMetric {
    pub year: i32,
    pub count: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsSnapshot {
    pub by_topic: Vec<TopicMetric>,
    pub by_year: Vec<YearMetric>,
    pub most_connected: Vec<ResearchItem>,
    pub pdf_coverage: f64,
    pub document_count: u64,
    pub resolved_pdf_count: u64,
    pub full_text_count: u64,
    pub partial_text_count: u64,
    pub indexed_pages: u64,
    pub citation: CitationAnalytics,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationAnalytics {
    pub most_cited_local: Vec<CitationPaperMetric>,
    pub most_referencing: Vec<CitationPaperMetric>,
    pub frequently_referenced_external: Vec<ExternalWorkMetric>,
    pub resolution_coverage: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationPaperMetric {
    pub item_id: String,
    pub title: String,
    pub count: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalWorkMetric {
    pub work_id: String,
    pub title: String,
    pub year: Option<i32>,
    pub doi: Option<String>,
    pub referenced_by_count: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusDiagnostics {
    pub status: String,
    pub document_count: u64,
    pub pdf_count: u64,
    pub resolved_pdf_count: u64,
    pub full_text_count: u64,
    pub partial_text_count: u64,
    pub indexed_pages: u64,
    pub successful_version: i64,
    pub last_synced_at: Option<String>,
    pub changed_documents: u64,
    pub warning_count: u64,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionDiagnostics {
    pub status: String,
    pub pending: u64,
    pub good: u64,
    pub degraded: u64,
    pub failed: u64,
    pub fallback: u64,
    pub unsupported: u64,
    pub unavailable: u64,
    pub active_provider_profile: Option<String>,
    pub active_routing_profile: String,
    pub active_quality_profile: String,
    pub shadow: crate::extraction::shadow::ShadowPilotDiagnostics,
}

impl ExtractionDiagnostics {
    pub fn empty() -> Self {
        Self {
            status: "not-indexed".to_owned(),
            pending: 0,
            good: 0,
            degraded: 0,
            failed: 0,
            fallback: 0,
            unsupported: 0,
            unavailable: 0,
            active_provider_profile: None,
            active_routing_profile: "source-text-fallback-v1".to_owned(),
            active_quality_profile: "extraction-quality-v1.1".to_owned(),
            shadow: crate::extraction::shadow::ShadowPilotDiagnostics::disabled("disabled"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationDiagnostics {
    pub status: String,
    pub eligible_documents: u64,
    pub indexed_documents: u64,
    pub no_reference_section: u64,
    pub partial_documents: u64,
    pub failed_documents: u64,
    pub references_extracted: u64,
    pub references_with_doi: u64,
    pub resolved_local: u64,
    pub external_works: u64,
    pub ambiguous_references: u64,
    pub unresolved_references: u64,
    pub citation_edges: u64,
    pub queued_documents: u64,
    pub queued_resolutions: u64,
    pub last_indexed_at: Option<String>,
    pub extractor_version: String,
    pub resolver_version: String,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticDiagnostics {
    pub status: String,
    pub indexing: bool,
    pub model_installed: bool,
    pub model_identifier: String,
    pub model_display_name: String,
    pub model_source: String,
    pub model_license: String,
    pub model_download_bytes: u64,
    pub model_storage_bytes: u64,
    pub model_revision: Option<String>,
    pub active_profile: Option<EmbeddingProfileSummary>,
    pub active_chunking_profile: ChunkingProfileSummary,
    pub active_retrieval_profile: RetrievalProfileSummary,
    pub vector_backend: String,
    pub eligible_papers: u64,
    pub indexed_papers: u64,
    pub stale_papers: u64,
    pub eligible_documents: u64,
    pub chunked_documents: u64,
    pub generated_chunks: u64,
    pub indexed_chunks: u64,
    pub stale_chunks: u64,
    pub current_document_revisions_indexed: u64,
    pub stale_document_revisions_indexed: u64,
    pub maximum_observed_chunks_per_document: u64,
    pub documents_at_chunk_limit: u64,
    pub registered_paper_vectors: u64,
    pub physical_paper_vectors: Option<u64>,
    pub paper_vector_count_matches: Option<bool>,
    pub registered_chunk_vectors: u64,
    pub physical_chunk_vectors: Option<u64>,
    pub chunk_vector_count_matches: Option<bool>,
    pub metadata_only_papers: u64,
    pub vector_index_bytes: u64,
    pub chunk_metadata_bytes: u64,
    pub last_indexed_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkingProfileSummary {
    pub id: String,
    pub strategy: String,
    pub version: String,
    pub target_tokens: u32,
    pub overlap_tokens: u32,
    pub maximum_tokens: u32,
    pub maximum_chunks_per_document: u32,
    pub boundary_policy: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalProfileSummary {
    pub id: String,
    pub maximum_candidates_per_vector_index: u32,
    pub reciprocal_rank_fusion_k: f64,
    pub lexical_weight: f64,
    pub semantic_weight: f64,
    pub maximum_passages_per_paper: u32,
    pub scoped_path: String,
    pub scoped_chunk_limit_per_paper: Option<u32>,
    pub context_assembler: String,
    pub maximum_evidence_sources: u32,
    pub compact_evidence_budget_tokens: u32,
    pub large_evidence_budget_tokens: u32,
    pub token_accounting: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingProfileSummary {
    pub id: String,
    pub provider: String,
    pub model_identifier: String,
    pub model_revision: String,
    pub artifact_identity: String,
    pub embedding_dimension: u32,
    pub similarity_metric: String,
    pub query_template: String,
    pub document_template: String,
    pub normalization: String,
    pub runtime_version: String,
    pub vector_backend: String,
    pub vector_index_format_version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDiagnostics {
    pub mode: String,
    pub health: String,
    pub connection_status: String,
    pub server_id: Option<String>,
    pub api_version: Option<String>,
    pub schema_version: Option<String>,
    pub last_synced_at: Option<String>,
    pub last_error: Option<String>,
    pub active_library_id: Option<String>,
    pub source_changed: bool,
    pub sync_summary: Option<SyncSummary>,
}

impl SourceDiagnostics {
    pub fn empty() -> Self {
        Self {
            mode: "zotero".to_owned(),
            health: "not-synced".to_owned(),
            connection_status: "unknown".to_owned(),
            server_id: None,
            api_version: None,
            schema_version: None,
            last_synced_at: None,
            last_error: None,
            active_library_id: None,
            source_changed: false,
            sync_summary: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub status: String,
    pub libraries_scanned: u32,
    pub objects_changed: u64,
    pub objects_deleted: u64,
    pub normalization_warnings: u64,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    pub mode: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    #[serde(default)]
    pub item_ids: Vec<String>,
    pub collection_id: Option<String>,
    pub citation_filter: Option<String>,
    pub year_from: Option<i32>,
    pub year_to: Option<i32>,
    pub item_type: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub item: ResearchItem,
    pub score: f64,
    pub reasons: Vec<String>,
    pub snippet: Option<String>,
    pub match_source: String,
    pub retrieval_modes: Vec<String>,
    pub ranking: RetrievalScoreBreakdown,
    pub passages: Vec<SemanticPassage>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub results: Vec<SearchResult>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
    pub corpus_status: String,
    pub retrieval_mode: String,
    pub semantic_indexed_papers: u64,
    pub semantic_eligible_papers: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalScoreBreakdown {
    pub lexical_rank: Option<u32>,
    pub semantic_rank: Option<u32>,
    pub lexical_score: Option<f64>,
    pub semantic_score: Option<f64>,
    pub fusion_score: Option<f64>,
    pub exact_match: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticPassage {
    // Full evidence stays in the backend; serialized search previews stay short.
    #[serde(skip)]
    pub(crate) text: String,
    #[serde(skip)]
    pub(crate) lexical_rank: Option<u32>,
    #[serde(skip)]
    pub(crate) semantic_rank: Option<u32>,
    pub chunk_id: String,
    pub document_id: String,
    pub document_revision_id: i64,
    pub snippet: String,
    pub character_start: u64,
    pub character_end: u64,
    pub page_start: Option<u32>,
    pub page_end: Option<u32>,
    pub semantic_score: f64,
    pub embedding_profile_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticIndexRequest {
    pub max_papers: Option<u32>,
    pub max_documents: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarPapersRequest {
    pub seed_item_ids: Vec<String>,
    pub collection_id: Option<String>,
    pub limit: Option<u32>,
    pub minimum_similarity: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarPaper {
    pub item: ResearchItem,
    pub similarity_score: f64,
    pub similarity_label: String,
    pub source_representation: String,
    pub target_representation: String,
    pub embedding_profile: EmbeddingProfileSummary,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarPaperPage {
    pub results: Vec<SimilarPaper>,
    pub seed_item_ids: Vec<String>,
    pub indexed_papers: u64,
    pub eligible_papers: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperSemanticCoverage {
    pub item_id: String,
    pub paper_status: String,
    pub paper_representation: Option<String>,
    pub paper_coverage: Option<String>,
    pub document_count: u64,
    pub chunk_count: u64,
    pub indexed_chunk_count: u64,
    pub embedding_profile: Option<EmbeddingProfileSummary>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticRelationshipEvidence {
    pub edge_id: String,
    pub relation: String,
    pub source: CitationItemSummary,
    pub target: CitationItemSummary,
    pub source_representation: String,
    pub target_representation: String,
    pub similarity_metric: String,
    pub similarity_score: f64,
    pub similarity_label: String,
    pub mutual: bool,
    pub embedding_profile: EmbeddingProfileSummary,
    pub passages: Vec<SemanticPassage>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemQueryRequest {
    pub collection_id: Option<String>,
    pub item_type: Option<String>,
    pub year_from: Option<i32>,
    pub year_to: Option<i32>,
    pub tag: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemPage {
    pub items: Vec<ResearchItem>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphProjectionRequest {
    pub collection_id: Option<String>,
    pub seed_item_ids: Vec<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphProjectionResult {
    pub items: Vec<ResearchItem>,
    pub topics: Vec<Topic>,
    pub graph: ResearchGraph,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationIndexRequest {
    pub priority_item_id: Option<String>,
    pub max_documents: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceQueryRequest {
    pub item_id: String,
    pub filter: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationCoverage {
    pub item_id: String,
    pub extraction_status: String,
    pub total_references: u64,
    pub resolved_local: u64,
    pub external: u64,
    pub ambiguous: u64,
    pub unresolved: u64,
    pub cited_by_local: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationItemSummary {
    pub id: String,
    pub title: String,
    pub first_author: Option<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub doi: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalWorkSummary {
    pub id: String,
    pub identity_kind: String,
    pub title: Option<String>,
    pub first_author: Option<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub doi: Option<String>,
    pub referenced_by_count: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionCandidate {
    pub item: CitationItemSummary,
    pub category: String,
    pub score: f64,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BibliographicReferenceView {
    pub id: String,
    pub ordinal: u32,
    pub raw_text: String,
    pub original_doi: Option<String>,
    pub normalized_doi: Option<String>,
    pub parsed_title: Option<String>,
    pub parsed_year: Option<i32>,
    pub first_author: Option<String>,
    pub parsed_venue: Option<String>,
    pub extraction_confidence: String,
    pub page_number: Option<u32>,
    pub source_start: Option<u64>,
    pub source_end: Option<u64>,
    pub resolution_status: String,
    pub resolution_method: String,
    pub confidence_category: String,
    pub resolution_evidence: Vec<String>,
    pub matched_item: Option<CitationItemSummary>,
    pub external_work: Option<ExternalWorkSummary>,
    pub candidates: Vec<ResolutionCandidate>,
    pub override_stale: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferencePage {
    pub coverage: CitationCoverage,
    pub references: Vec<BibliographicReferenceView>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationListRequest {
    pub item_id: String,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationItemPage {
    pub items: Vec<CitationItemSummary>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuralPaperLink {
    pub item: CitationItemSummary,
    pub count: u64,
    pub normalized_strength: f64,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuralPaperPage {
    pub links: Vec<StructuralPaperLink>,
    pub total: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoCitedWorkLink {
    pub local_item: Option<CitationItemSummary>,
    pub external_work: Option<ExternalWorkSummary>,
    pub count: u64,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoCitedWorkPage {
    pub links: Vec<CoCitedWorkLink>,
    pub total: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchNeighborhoodRequest {
    pub seed_item_ids: Vec<String>,
    pub mode: String,
    pub max_nodes: Option<u32>,
    pub max_edges: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchNeighborhood {
    pub projection: GraphProjectionResult,
    pub mode: String,
    pub total_relationships: u64,
    pub hidden_relationships: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeEvidenceRequest {
    pub edge_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationEvidenceView {
    pub reference_id: String,
    pub ordinal: u32,
    pub raw_text: String,
    pub resolution_method: String,
    pub confidence_category: String,
    pub source_document_title: String,
    pub source_document_id: String,
    pub page_number: Option<u32>,
    pub extractor_provider: String,
    pub extractor_version: String,
    pub resolver_version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipEvidence {
    pub edge_id: String,
    pub relation: String,
    pub source: CitationItemSummary,
    pub target: CitationItemSummary,
    pub evidence_label: String,
    pub evidence: Vec<CitationEvidenceView>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionOverrideRequest {
    pub reference_id: String,
    pub decision: String,
    pub target_item_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingWorksRequest {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalWorkPage {
    pub works: Vec<ExternalWorkSummary>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentOpenResult {
    pub document_id: String,
    pub status: String,
    pub media_type: String,
    pub content_length: Option<u64>,
    pub revision_key: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTextPayload {
    pub document_id: String,
    pub provider: String,
    pub content: String,
    pub indexed_pages: Option<u32>,
    pub total_pages: Option<u32>,
    pub indexed_chars: Option<u64>,
    pub total_chars: Option<u64>,
    pub completeness: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderState {
    pub document_id: String,
    pub page_number: u32,
    pub zoom: f64,
    pub fit_mode: String,
    pub outline_open: bool,
    pub context_open: bool,
}

#[derive(Clone, Debug)]
pub struct FullTextInventory {
    pub versions: Vec<(String, i64)>,
    pub library_version: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct FullTextRecord {
    pub attachment_key: String,
    pub version: i64,
    pub content: String,
    pub indexed_pages: Option<u32>,
    pub total_pages: Option<u32>,
    pub indexed_chars: Option<u64>,
    pub total_chars: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct FullTextSyncDelta {
    pub successful_version: i64,
    pub records: Vec<FullTextRecord>,
    pub removed_attachment_keys: Vec<String>,
    pub warning_count: u64,
    pub status: String,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProbe {
    pub server_id: String,
    pub api_version: String,
    pub schema_version: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RemoteLibrary {
    pub kind: String,
    pub zotero_id: String,
    pub name: String,
}

impl RemoteLibrary {
    pub fn api_prefix(&self) -> String {
        if self.kind == "user" {
            "/users/0".to_owned()
        } else {
            format!("/groups/{}", self.zotero_id)
        }
    }
}

#[derive(Clone, Debug)]
pub struct VersionMap {
    pub versions: Vec<(String, i64)>,
    pub library_version: i64,
}

#[derive(Clone, Debug)]
pub struct RawObject {
    pub key: String,
    pub version: i64,
    pub object_type: String,
    pub json: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct LibraryDelta {
    pub target_version: i64,
    pub objects: Vec<RawObject>,
    pub deleted_items: Vec<String>,
    pub deleted_collections: Vec<String>,
}
