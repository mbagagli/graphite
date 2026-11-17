use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::database::ResearchDatabase;
use crate::extraction::model::ExtractionObservations;
use crate::extraction::shadow::{
    ShadowBatchSummary, ShadowExtractionCoordinator, ShadowRunRequest, SHADOW_ROUTING_PROFILE_ID,
};
use crate::extraction::ExtractionCoordinator;
use crate::model::{
    FullTextRecord, FullTextSyncDelta, LibraryDelta, RawObject, RemoteLibrary, SourceProbe,
};

use super::config::ShadowPilotConfig;
use super::model::ShadowComparison;

const REPRESENTATIVE_MANIFEST: &str = "representative-manifest.json";
const TARGETED_MANIFEST: &str = "targeted-manifest.json";
const OBSERVATION_REPORT: &str = "m6b-shadow-observation.json";
const STRUCTURE_DOCUMENTS: &[&str] = &["paper16", "paper21", "paper33"];
const CANCELLATION_DOCUMENT: &str = "paper32";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObservationManifest {
    documents: Vec<ObservationDocument>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObservationDocument {
    id: String,
    file: String,
    sha256: String,
    categories: Vec<String>,
    expected_pages: u32,
    text_layer: String,
    #[serde(default)]
    expected_ocr_pages: Vec<u32>,
    zotero_baseline: Option<String>,
    zotero_baseline_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservedDocument {
    id: String,
    sha256: String,
    categories: Vec<String>,
    expected_pages: u32,
    text_layer: String,
    expected_ocr_pages: Vec<u32>,
    zotero_text_available: bool,
}

impl From<&ObservationDocument> for ObservedDocument {
    fn from(document: &ObservationDocument) -> Self {
        Self {
            id: document.id.clone(),
            sha256: document.sha256.clone(),
            categories: document.categories.clone(),
            expected_pages: document.expected_pages,
            text_layer: document.text_layer.clone(),
            expected_ocr_pages: document.expected_ocr_pages.clone(),
            zotero_text_available: document.zotero_baseline.is_some(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ZoteroSidecar {
    source_version: i64,
    content: String,
    indexed_pages: Option<u32>,
    total_pages: Option<u32>,
    indexed_characters: Option<u64>,
    total_characters: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponsivenessEvidence {
    probe_count: u64,
    maximum_fts_latency_ms: u64,
    parent_rss_after_batch_bytes: Vec<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SchedulingEvidence {
    duplicate_summary: ShadowBatchSummary,
    attempts_before_duplicate: usize,
    attempts_after_duplicate: usize,
    cancellation_requested_jobs: u32,
    cancellation_summary: ShadowBatchSummary,
    interrupted_jobs_recovered: u32,
    recovered_jobs_cancelled: u32,
    final_pending_jobs: u64,
    final_running_jobs: u64,
    leaked_sidecar_processes: usize,
    parent_rss_before_bytes: u64,
    parent_rss_after_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationAttempt {
    document_id: String,
    request_kind: String,
    provider_profile: String,
    provider_role: String,
    outcome: String,
    quality: String,
    duration_ms: Option<u64>,
    peak_rss_bytes: Option<u64>,
    peak_thread_count: Option<u64>,
    zotero_text_available: bool,
    active_zotero_quality: Option<String>,
    candidate_to_zotero_character_ratio: Option<f64>,
    reported_page_count_difference: Option<i64>,
    tail_token_overlap: Option<f64>,
    truncation_suspected: bool,
    reported_page_count: Option<u32>,
    extracted_page_count: Option<u32>,
    empty_pages: Vec<u32>,
    suspicious_pages: Vec<u32>,
    ocr_used: Option<bool>,
    ocr_pages: Vec<u32>,
    encoding_warning_count: usize,
    layout_warning_count: usize,
    replacement_character_count: Option<u64>,
    warning_count: usize,
    has_error: bool,
    routing_reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct TableDigest {
    rows: u64,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActiveRagSnapshot {
    active_selections: TableDigest,
    document_texts: TableDigest,
    search_content: TableDigest,
    research_fts: TableDigest,
    semantic_chunks: TableDigest,
    vector_registrations: TableDigest,
    graph_relations: TableDigest,
    citation_edges: TableDigest,
    semantic_neighbors: TableDigest,
    reader_state: TableDigest,
    tyrion_evidence: TableDigest,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationReport {
    schema_version: u32,
    routing_profile: String,
    documents: Vec<ObservedDocument>,
    zotero_document_count: usize,
    content_batches: Vec<ShadowBatchSummary>,
    structure_summary: ShadowBatchSummary,
    responsiveness: ResponsivenessEvidence,
    scheduling: SchedulingEvidence,
    active_rag_unchanged: bool,
    active_rag_before: ActiveRagSnapshot,
    active_rag_after: ActiveRagSnapshot,
    attempts: Vec<ObservationAttempt>,
}

#[tokio::test]
#[ignore = "requires configured release sidecars and the local 25-document observation corpus"]
async fn runs_extended_real_linux_shadow_observation_without_active_rag_changes() {
    let corpus_root = corpus_root();
    let documents = load_observation_documents(&corpus_root);
    assert_eq!(documents.len(), 25, "extended observation document count");
    let parent_rss_before_bytes = current_process_rss_bytes();
    let (database, database_path, document_ids) =
        seed_observation_database(&corpus_root, &documents).await;
    seed_active_rag_sentinels(&database, &document_ids);
    let before = active_rag_snapshot(&database);
    let coordinator =
        ShadowExtractionCoordinator::new(database.clone()).expect("shadow coordinator");

    let ordered_ids = documents
        .iter()
        .map(|document| document_ids[&document.id].clone())
        .collect::<Vec<_>>();
    let mut content_batches = Vec::new();
    let mut responsiveness = ResponsivenessEvidence::default();
    for batch in ordered_ids.chunks(10) {
        let (summary, evidence) = run_responsive_batch(
            coordinator.clone(),
            batch.to_vec(),
            documents.len() as u64,
            &database,
        )
        .await;
        content_batches.push(summary);
        responsiveness.probe_count += evidence.probe_count;
        responsiveness.maximum_fts_latency_ms = responsiveness
            .maximum_fts_latency_ms
            .max(evidence.maximum_fts_latency_ms);
        responsiveness
            .parent_rss_after_batch_bytes
            .push(current_process_rss_bytes());
    }

    let structure_summary = coordinator
        .run_selected(ShadowRunRequest {
            document_ids: STRUCTURE_DOCUMENTS
                .iter()
                .map(|id| document_ids[*id].clone())
                .collect(),
            request_structure: true,
        })
        .await
        .expect("explicit structure observation");

    let attempts_before_duplicate = observation_attempts(&database).len();
    let duplicate_summary = coordinator
        .run_selected(ShadowRunRequest {
            document_ids: ordered_ids[..10].to_vec(),
            request_structure: false,
        })
        .await
        .expect("duplicate observation batch");
    let attempts_after_duplicate = observation_attempts(&database).len();
    assert_eq!(attempts_after_duplicate, attempts_before_duplicate);
    assert_eq!(duplicate_summary.queued, 0);

    let cancellation_coordinator = coordinator.clone();
    let cancellation_document_id = document_ids[CANCELLATION_DOCUMENT].clone();
    let cancellation_task = tokio::spawn(async move {
        cancellation_coordinator
            .run_selected(ShadowRunRequest {
                document_ids: vec![cancellation_document_id],
                request_structure: true,
            })
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let cancellation_requested_jobs = coordinator.cancel().expect("cancel shadow batch");
    let cancellation_summary = cancellation_task
        .await
        .expect("cancellation task")
        .expect("cancellation result");
    assert_eq!(cancellation_summary.cancelled, 1);

    let restart_probe_id = seed_interrupted_job(
        &database,
        &document_ids["paper35"],
        "m6b-observation-restart-probe",
    );
    let recovered_coordinator =
        ShadowExtractionCoordinator::new(database.clone()).expect("recovery coordinator");
    let interrupted_jobs_recovered =
        u32::from(job_state(&database, &restart_probe_id) == "pending");
    assert_eq!(interrupted_jobs_recovered, 1);
    let recovered_jobs_cancelled = recovered_coordinator
        .cancel()
        .expect("cancel recovered pending job");
    assert_eq!(job_state(&database, &restart_probe_id), "cancelled");

    wait_for_sidecars_to_exit().await;
    let leaked_sidecar_processes = sidecar_process_count();
    assert_eq!(leaked_sidecar_processes, 0);
    let diagnostics = recovered_coordinator
        .diagnostics()
        .expect("final shadow diagnostics");
    assert_eq!(diagnostics.pending, 0);
    assert_eq!(diagnostics.running, 0);
    let after = active_rag_snapshot(&database);
    assert_eq!(after, before);

    let attempts = observation_attempts(&database);
    let report = ObservationReport {
        schema_version: 1,
        routing_profile: SHADOW_ROUTING_PROFILE_ID.to_owned(),
        zotero_document_count: documents
            .iter()
            .filter(|document| document.zotero_baseline.is_some())
            .count(),
        documents: documents.iter().map(ObservedDocument::from).collect(),
        content_batches,
        structure_summary,
        responsiveness,
        scheduling: SchedulingEvidence {
            duplicate_summary,
            attempts_before_duplicate,
            attempts_after_duplicate,
            cancellation_requested_jobs,
            cancellation_summary,
            interrupted_jobs_recovered,
            recovered_jobs_cancelled,
            final_pending_jobs: diagnostics.pending,
            final_running_jobs: diagnostics.running,
            leaked_sidecar_processes,
            parent_rss_before_bytes,
            parent_rss_after_bytes: current_process_rss_bytes(),
        },
        active_rag_unchanged: true,
        active_rag_before: before,
        active_rag_after: after,
        attempts,
    };
    let report_path = corpus_root
        .parent()
        .expect("benchmark root")
        .join("results")
        .join(OBSERVATION_REPORT);
    fs::write(
        report_path,
        serde_json::to_vec_pretty(&report).expect("serialize observation report"),
    )
    .expect("write local observation report");
    fs::remove_file(database_path).expect("remove observation database");
}

#[tokio::test]
#[ignore = "requires the configured Xberg release sidecar and local representative corpus"]
async fn failed_real_shadow_attempt_is_inert_and_new_revision_recovers() {
    let corpus_root = corpus_root();
    let document = load_observation_documents(&corpus_root)
        .into_iter()
        .find(|document| document.id == "paper20")
        .expect("failure recovery document");
    let (database, database_path, document_ids) =
        seed_observation_database(&corpus_root, std::slice::from_ref(&document)).await;
    let before = active_rag_snapshot(&database);
    let mut unavailable_config = ShadowPilotConfig::from_environment();
    unavailable_config.xberg_binary = Some(PathBuf::from("/bin/false"));
    let failing = ShadowExtractionCoordinator::with_config(database.clone(), unavailable_config)
        .expect("failing shadow coordinator");
    let document_id = document_ids[&document.id].clone();
    let failed = failing
        .run_selected(ShadowRunRequest {
            document_ids: vec![document_id.clone()],
            request_structure: false,
        })
        .await
        .expect("failed shadow batch");
    assert_eq!(failed.failed, 1);
    assert_eq!(active_rag_snapshot(&database), before);

    let pdf = corpus_root.join(&document.file);
    let file_size = fs::metadata(&pdf).expect("recovery PDF metadata").len();
    database
        .store_document_locator(
            &document_id,
            &pdf,
            "m6b-failure-recovery-revision",
            &document.sha256,
            file_size,
            None,
        )
        .expect("recovery artifact revision");
    let recovered = ShadowExtractionCoordinator::new(database.clone())
        .expect("recovered shadow coordinator")
        .run_selected(ShadowRunRequest {
            document_ids: vec![document_id],
            request_structure: false,
        })
        .await
        .expect("recovered shadow batch");
    assert_eq!(recovered.completed, 1);
    assert_eq!(active_rag_snapshot(&database), before);
    let attempts = observation_attempts(&database);
    assert_eq!(attempts.len(), 3);
    assert!(attempts.iter().any(|attempt| attempt.outcome == "crashed"));
    assert_eq!(
        attempts
            .iter()
            .filter(|attempt| attempt.outcome == "succeeded")
            .count(),
        2
    );
    assert!(attempts
        .iter()
        .any(|attempt| attempt.provider_role == "ocr-fallback"));
    wait_for_sidecars_to_exit().await;
    assert_eq!(sidecar_process_count(), 0);
    fs::remove_file(database_path).expect("remove recovery database");
}

async fn run_responsive_batch(
    coordinator: ShadowExtractionCoordinator,
    document_ids: Vec<String>,
    expected_fts_hits: u64,
    database: &ResearchDatabase,
) -> (ShadowBatchSummary, ResponsivenessEvidence) {
    let task = tokio::spawn(async move {
        coordinator
            .run_selected(ShadowRunRequest {
                document_ids,
                request_structure: false,
            })
            .await
    });
    let mut evidence = ResponsivenessEvidence::default();
    loop {
        let started = Instant::now();
        assert_eq!(fts_pilot_hits(database), expected_fts_hits);
        evidence.probe_count += 1;
        evidence.maximum_fts_latency_ms = evidence
            .maximum_fts_latency_ms
            .max(started.elapsed().as_millis() as u64);
        if task.is_finished() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    (
        task.await
            .expect("responsive shadow batch task")
            .expect("responsive shadow batch"),
        evidence,
    )
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tools/pdf-benchmark/corpus")
}

fn load_observation_documents(corpus_root: &Path) -> Vec<ObservationDocument> {
    let mut documents = Vec::new();
    for name in [REPRESENTATIVE_MANIFEST, TARGETED_MANIFEST] {
        let manifest: ObservationManifest = serde_json::from_slice(
            &fs::read(corpus_root.join(name)).expect("observation manifest"),
        )
        .expect("observation manifest JSON");
        documents.extend(manifest.documents);
    }
    documents.sort_by(|left, right| left.id.cmp(&right.id));
    for document in &documents {
        let pdf_bytes = fs::read(corpus_root.join(&document.file)).expect("observation PDF");
        assert_eq!(
            hex_sha256(&pdf_bytes),
            document.sha256,
            "{} PDF",
            document.id
        );
        if let (Some(sidecar), Some(expected_hash)) = (
            document.zotero_baseline.as_ref(),
            document.zotero_baseline_sha256.as_ref(),
        ) {
            let sidecar_bytes = fs::read(corpus_root.join(sidecar)).expect("Zotero sidecar");
            assert_eq!(
                hex_sha256(&sidecar_bytes),
                *expected_hash,
                "{} Zotero sidecar",
                document.id
            );
        }
    }
    documents
}

async fn seed_observation_database(
    corpus_root: &Path,
    documents: &[ObservationDocument],
) -> (ResearchDatabase, PathBuf, BTreeMap<String, String>) {
    let database_path = std::env::temp_dir().join(format!(
        "graphite-m6b-shadow-observation-{}-{}.sqlite3",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    let database =
        ResearchDatabase::initialize(database_path.clone()).expect("observation database");
    let source = database
        .register_source(&SourceProbe {
            server_id: "m6b-shadow-observation".to_owned(),
            api_version: "3".to_owned(),
            schema_version: None,
        })
        .expect("observation source");
    let library = database
        .register_libraries(
            &source.id,
            &[RemoteLibrary {
                kind: "user".to_owned(),
                zotero_id: "0".to_owned(),
                name: "M6-B local observation".to_owned(),
            }],
        )
        .expect("observation library")
        .remove(0);
    let objects = documents
        .iter()
        .flat_map(|document| {
            [
                observation_item(&format!("{}-item", document.id), None),
                observation_item(
                    &format!("{}-pdf", document.id),
                    Some(&format!("{}-item", document.id)),
                ),
            ]
        })
        .collect::<Vec<_>>();
    database
        .apply_delta(
            &source.id,
            &library.id,
            &LibraryDelta {
                target_version: 1,
                objects,
                deleted_items: Vec::new(),
                deleted_collections: Vec::new(),
            },
        )
        .expect("observation metadata");
    let mut full_text_records = Vec::new();
    let mut document_ids = BTreeMap::new();
    for document in documents {
        let pdf = corpus_root.join(&document.file);
        let bytes = fs::read(&pdf).expect("observation PDF");
        let graphite_document_id = document_for_key(&database, &format!("{}-pdf", document.id));
        database
            .store_document_locator(
                &graphite_document_id,
                &pdf,
                &format!("observation:{}", document.id),
                &document.sha256,
                bytes.len() as u64,
                None,
            )
            .expect("observation locator");
        document_ids.insert(document.id.clone(), graphite_document_id);
        if let Some(sidecar) = &document.zotero_baseline {
            let sidecar: ZoteroSidecar = serde_json::from_slice(
                &fs::read(corpus_root.join(sidecar)).expect("Zotero sidecar"),
            )
            .expect("Zotero sidecar JSON");
            full_text_records.push(FullTextRecord {
                attachment_key: format!("{}-pdf", document.id),
                version: sidecar.source_version,
                content: sidecar.content,
                indexed_pages: sidecar.indexed_pages,
                total_pages: sidecar.total_pages,
                indexed_chars: sidecar.indexed_characters,
                total_chars: sidecar.total_characters,
            });
        }
    }
    database
        .apply_fulltext_delta(
            &source.id,
            &library.id,
            &FullTextSyncDelta {
                successful_version: 1,
                records: full_text_records,
                removed_attachment_keys: Vec::new(),
                warning_count: 0,
                status: "ready".to_owned(),
                last_error: None,
            },
        )
        .expect("observation Zotero text");
    ExtractionCoordinator::new(database.clone())
        .backfill(100)
        .await
        .expect("observation canonical Zotero baseline");
    (database, database_path, document_ids)
}

fn observation_item(key: &str, parent: Option<&str>) -> RawObject {
    let data = parent.map_or_else(
        || {
            serde_json::json!({
                "key": key,
                "version": 1,
                "itemType": "journalArticle",
                "title": format!("Observation {key}"),
                "creators": [],
                "tags": [],
                "collections": []
            })
        },
        |parent| {
            serde_json::json!({
                "key": key,
                "version": 1,
                "itemType": "attachment",
                "parentItem": parent,
                "title": "PDF",
                "contentType": "application/pdf",
                "filename": format!("{key}.pdf"),
                "linkMode": "imported_file"
            })
        },
    );
    RawObject {
        key: key.to_owned(),
        version: 1,
        object_type: "items".to_owned(),
        json: serde_json::json!({"key": key, "version": 1, "data": data}),
    }
}

fn seed_active_rag_sentinels(database: &ResearchDatabase, document_ids: &BTreeMap<String, String>) {
    let connection = database.connect().expect("sentinel connection");
    let document_id = &document_ids["paper11"];
    let (item_id, library_id, revision_id): (String, String, i64) = connection
        .query_row(
            "SELECT document.research_item_id, document.library_id, revision.id
             FROM documents document
             JOIN document_locators locator ON locator.document_id = document.id
             JOIN document_revisions revision
               ON revision.document_id = document.id
              AND revision.revision_key = locator.revision_key
             WHERE document.id = ?1",
            [document_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("sentinel identities");
    let second_item_id: String = connection
        .query_row(
            "SELECT research_item_id FROM documents WHERE id = ?1",
            [&document_ids["paper12"]],
            |row| row.get(0),
        )
        .expect("second sentinel item");
    connection
        .execute(
            "INSERT INTO item_relations(item_id, predicate, target)
             VALUES (?1, 'related', 'observation-sentinel')",
            [&item_id],
        )
        .expect("graph sentinel");
    connection
        .execute(
            "INSERT INTO citation_edges (
                id, source_item_id, target_item_id, evidence_count,
                strongest_category, created_at, updated_at
             ) VALUES (
                'm6b-observation-citation', ?1, ?2, 1, 'fixture',
                CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
             )",
            params![item_id, second_item_id],
        )
        .expect("citation sentinel");
    connection
        .execute(
            "INSERT INTO embedding_profiles (
                id, provider, model_identifier, model_revision,
                artifact_identity, embedding_dimension, similarity_metric,
                query_template, document_template, normalization,
                runtime_version, vector_backend, vector_index_format_version,
                active, created_at
             ) VALUES (
                'm6b-observation-embedding', 'fixture', 'fixture', '1',
                'fixture', 2, 'cosine', '', '', 'l2', 'fixture',
                'usearch', '1', 1, CURRENT_TIMESTAMP
             )",
            [],
        )
        .expect("embedding sentinel profile");
    connection
        .execute(
            "INSERT INTO semantic_vector_entries (
                embedding_profile_id, library_id, index_kind, target_id,
                active, created_at, updated_at
             ) VALUES (
                'm6b-observation-embedding', ?1, 'chunk',
                'm6b-observation-chunk', 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
             )",
            [&library_id],
        )
        .expect("vector sentinel");
    let vector_key = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO semantic_chunks (
                id, item_id, library_id, document_id, document_revision_id,
                embedding_profile_id, chunking_profile_id, source_provider,
                character_start, character_end, page_start, page_end,
                section_heading, ordinal, token_count, text, text_fingerprint,
                vector_key, status, created_at, updated_at
             ) VALUES (
                'm6b-observation-chunk', ?1, ?2, ?3, ?4,
                'm6b-observation-embedding', 'scientific-paragraph-v1',
                'zotero-indexed', 0, 29, 1, 1, 'Observation', 0, 4,
                'stable observation evidence', 'm6b-observation-fingerprint',
                ?5, 'active', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
             )",
            params![item_id, library_id, document_id, revision_id, vector_key],
        )
        .expect("semantic chunk sentinel");
    let (neighbor_source, neighbor_target) = if item_id < second_item_id {
        (&item_id, &second_item_id)
    } else {
        (&second_item_id, &item_id)
    };
    connection
        .execute(
            "INSERT INTO semantic_neighbor_cache (
                edge_id, embedding_profile_id, source_item_id, target_item_id,
                representation_version, source_representation,
                target_representation, similarity_metric, similarity_score,
                mutual, created_at
             ) VALUES (
                'm6b-observation-neighbor', 'm6b-observation-embedding',
                ?1, ?2, 'fixture', 'fixture-a', 'fixture-b', 'cosine',
                0.5, 1, CURRENT_TIMESTAMP
             )",
            params![neighbor_source, neighbor_target],
        )
        .expect("semantic neighbor sentinel");
    connection
        .execute(
            "INSERT INTO reader_states (
                document_id, page_number, zoom, fit_mode,
                outline_open, context_open, updated_at
             ) VALUES (?1, 3, 1.25, 'width', 1, 1, CURRENT_TIMESTAMP)",
            [document_id],
        )
        .expect("reader sentinel");
    connection
        .execute(
            "INSERT INTO agent_conversations (
                id, library_id, title, created_at, updated_at
             ) VALUES (
                'm6b-observation-conversation', ?1, 'Observation sentinel',
                CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
             )",
            [&library_id],
        )
        .expect("conversation sentinel");
    connection
        .execute(
            "INSERT INTO agent_runs (
                id, conversation_id, question, intent, scope_json,
                priority_context_json, provider_json, generation_json,
                prompt_versions_json, tool_calls_json, retrieval_json,
                context_json, completion_state, created_at
             ) VALUES (
                'm6b-observation-run', 'm6b-observation-conversation',
                'Observation sentinel?', 'research', '{}', '{}', '{}', '{}',
                '{}', '[]', '{}', '{}', 'completed', CURRENT_TIMESTAMP
             )",
            [],
        )
        .expect("agent run sentinel");
    connection
        .execute(
            "INSERT INTO agent_sources (
                run_id, source_id, evidence_kind, item_id, item_title,
                document_id, document_revision_id, chunk_id, passage_text,
                character_start, character_end, page_start, page_end,
                retrieval_modes_json, rank_json, provider,
                source_fingerprint, eligible_for_scientific_claim
             ) VALUES (
                'm6b-observation-run', 'm6b-observation-source', 'passage',
                ?1, 'Observation sentinel', ?2, ?3,
                'm6b-observation-chunk', 'stable observation evidence',
                0, 29, 1, 1, '[\"lexical\"]', '{}', 'zotero-indexed',
                'm6b-observation-source-fingerprint', 1
             )",
            params![item_id, document_id, revision_id],
        )
        .expect("Tyrion evidence sentinel");
}

fn active_rag_snapshot(database: &ResearchDatabase) -> ActiveRagSnapshot {
    let connection = database.connect().expect("snapshot connection");
    ActiveRagSnapshot {
        active_selections: digest_query(
            &connection,
            "SELECT artifact_id, extraction_id, quality_report_id,
                    routing_profile_id, selection_reason, selected_at, updated_at
             FROM active_extraction_selections ORDER BY artifact_id",
        ),
        document_texts: digest_query(
            &connection,
            "SELECT id, document_id, provider, source_version, content,
                    indexed_pages, total_pages, indexed_chars, total_chars,
                    completeness, extraction_id, projection_profile_id,
                    content_hash, is_active
             FROM document_texts ORDER BY id",
        ),
        search_content: digest_query(
            &connection,
            "SELECT rowid, item_id, library_id, title, creators, tags,
                    abstract_text, venue, body
             FROM search_content ORDER BY rowid",
        ),
        research_fts: digest_query(
            &connection,
            "SELECT rowid, title, creators, tags, abstract_text, venue, body
             FROM research_fts ORDER BY rowid",
        ),
        semantic_chunks: digest_query(
            &connection,
            "SELECT id, item_id, library_id, document_id,
                    document_revision_id, embedding_profile_id,
                    chunking_profile_id, source_provider, character_start,
                    character_end, page_start, page_end, section_heading,
                    ordinal, token_count, text, text_fingerprint,
                    vector_key, status
             FROM semantic_chunks ORDER BY id",
        ),
        vector_registrations: digest_query(
            &connection,
            "SELECT vector_key, embedding_profile_id, library_id,
                    index_kind, target_id, active
             FROM semantic_vector_entries ORDER BY vector_key",
        ),
        graph_relations: digest_query(
            &connection,
            "SELECT item_id, predicate, target
             FROM item_relations ORDER BY item_id, predicate, target",
        ),
        citation_edges: digest_query(
            &connection,
            "SELECT id, source_item_id, target_item_id, evidence_count,
                    strongest_category, created_at, updated_at
             FROM citation_edges ORDER BY id",
        ),
        semantic_neighbors: digest_query(
            &connection,
            "SELECT edge_id, embedding_profile_id, source_item_id,
                    target_item_id, representation_version,
                    source_representation, target_representation,
                    similarity_metric, similarity_score, mutual, created_at
             FROM semantic_neighbor_cache ORDER BY edge_id",
        ),
        reader_state: digest_query(
            &connection,
            "SELECT document_id, page_number, zoom, fit_mode,
                    outline_open, context_open, updated_at
             FROM reader_states ORDER BY document_id",
        ),
        tyrion_evidence: digest_query(
            &connection,
            "SELECT run_id, source_id, evidence_kind, item_id, item_title,
                    document_id, document_revision_id, chunk_id, passage_text,
                    character_start, character_end, page_start, page_end,
                    retrieval_modes_json, rank_json, provider,
                    source_fingerprint, eligible_for_scientific_claim
             FROM agent_sources ORDER BY run_id, source_id",
        ),
    }
}

fn digest_query(connection: &rusqlite::Connection, sql: &str) -> TableDigest {
    use rusqlite::types::ValueRef;
    let mut statement = connection.prepare(sql).expect("snapshot statement");
    let column_count = statement.column_count();
    let mut rows = statement.query([]).expect("snapshot rows");
    let mut digest = Sha256::new();
    let mut row_count = 0_u64;
    while let Some(row) = rows.next().expect("snapshot row") {
        row_count += 1;
        for index in 0..column_count {
            match row.get_ref(index).expect("snapshot value") {
                ValueRef::Null => digest.update([0]),
                ValueRef::Integer(value) => {
                    digest.update([1]);
                    digest.update(value.to_le_bytes());
                }
                ValueRef::Real(value) => {
                    digest.update([2]);
                    digest.update(value.to_le_bytes());
                }
                ValueRef::Text(value) => {
                    digest.update([3]);
                    digest.update((value.len() as u64).to_le_bytes());
                    digest.update(value);
                }
                ValueRef::Blob(value) => {
                    digest.update([4]);
                    digest.update((value.len() as u64).to_le_bytes());
                    digest.update(value);
                }
            }
        }
    }
    TableDigest {
        rows: row_count,
        sha256: hex_sha256(&digest.finalize()),
    }
}

fn observation_attempts(database: &ResearchDatabase) -> Vec<ObservationAttempt> {
    let connection = database.connect().expect("observation metrics connection");
    let mut statement = connection
        .prepare(
            "SELECT binding.external_artifact_id, job.request_kind,
                    extraction.provider_profile_id, attempt.provider_role,
                    attempt.attempt_outcome, quality.quality_status,
                    attempt.duration_ms, attempt.peak_rss_bytes,
                    attempt.peak_thread_count, attempt.comparison_json,
                    extraction.observations_json, extraction.warnings_json,
                    extraction.error IS NOT NULL, attempt.routing_reason,
                    active_quality.quality_status
             FROM shadow_extraction_attempts attempt
             JOIN shadow_extraction_jobs job ON job.id = attempt.job_id
             JOIN document_extractions extraction
               ON extraction.id = attempt.extraction_id
             JOIN extraction_quality_reports quality
               ON quality.id = attempt.quality_report_id
             JOIN document_artifact_source_bindings binding
               ON binding.artifact_id = extraction.artifact_id
              AND binding.state = 'active'
             LEFT JOIN active_extraction_selections active
               ON active.artifact_id = extraction.artifact_id
             LEFT JOIN extraction_quality_reports active_quality
               ON active_quality.id = active.quality_report_id
             ORDER BY binding.external_artifact_id,
                      job.request_kind, extraction.provider_profile_id",
        )
        .expect("observation metrics statement");
    statement
        .query_map([], |row| {
            let external_id: String = row.get(0)?;
            let comparison_json: String = row.get(9)?;
            let observations_json: String = row.get(10)?;
            let warnings_json: String = row.get(11)?;
            let comparison: ShadowComparison =
                serde_json::from_str(&comparison_json).expect("comparison JSON");
            let observations: ExtractionObservations =
                serde_json::from_str(&observations_json).expect("observations JSON");
            let warnings: Vec<String> =
                serde_json::from_str(&warnings_json).expect("warnings JSON");
            Ok(ObservationAttempt {
                document_id: external_id
                    .strip_suffix("-pdf")
                    .unwrap_or(&external_id)
                    .to_owned(),
                request_kind: row.get(1)?,
                provider_profile: row.get(2)?,
                provider_role: row.get(3)?,
                outcome: row.get(4)?,
                quality: row.get(5)?,
                duration_ms: row.get::<_, Option<i64>>(6)?.map(|value| value as u64),
                peak_rss_bytes: row.get::<_, Option<i64>>(7)?.map(|value| value as u64),
                peak_thread_count: row.get::<_, Option<i64>>(8)?.map(|value| value as u64),
                zotero_text_available: comparison.zotero_text_available,
                active_zotero_quality: row.get(14)?,
                candidate_to_zotero_character_ratio: comparison.candidate_to_zotero_character_ratio,
                reported_page_count_difference: comparison.reported_page_count_difference,
                tail_token_overlap: comparison.tail_token_overlap,
                truncation_suspected: comparison.truncation_suspected,
                reported_page_count: observations.reported_page_count.value,
                extracted_page_count: observations.extracted_page_count.value,
                empty_pages: observations.empty_page_indices.value.unwrap_or_default(),
                suspicious_pages: observations
                    .suspicious_page_indices
                    .value
                    .unwrap_or_default(),
                ocr_used: observations.ocr_used.value,
                ocr_pages: observations.ocr_page_indices.value.unwrap_or_default(),
                encoding_warning_count: observations
                    .encoding_warnings
                    .value
                    .as_ref()
                    .map_or(0, Vec::len),
                layout_warning_count: observations
                    .layout_warnings
                    .value
                    .as_ref()
                    .map_or(0, Vec::len),
                replacement_character_count: observations.replacement_character_count.value,
                warning_count: warnings.len(),
                has_error: row.get(12)?,
                routing_reason: row.get(13)?,
            })
        })
        .expect("observation metric rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("observation metrics")
}

fn fts_pilot_hits(database: &ResearchDatabase) -> u64 {
    database
        .connect()
        .expect("FTS responsiveness connection")
        .query_row(
            "SELECT COUNT(*) FROM research_fts WHERE research_fts MATCH 'Observation'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("FTS responsiveness query") as u64
}

fn seed_interrupted_job(database: &ResearchDatabase, document_id: &str, job_id: &str) -> String {
    let connection = database.connect().expect("restart probe connection");
    let revision_id: i64 = connection
        .query_row(
            "SELECT revision.id
             FROM document_locators locator
             JOIN document_revisions revision
               ON revision.document_id = locator.document_id
              AND revision.revision_key = locator.revision_key
             WHERE locator.document_id = ?1",
            [document_id],
            |row| row.get(0),
        )
        .expect("restart probe revision");
    connection
        .execute(
            "INSERT INTO shadow_extraction_jobs (
                id, artifact_id, artifact_revision_id, routing_profile_id,
                request_kind, execution_state, routing_reason,
                cancellation_requested, created_at, started_at
             ) VALUES (
                ?1, ?2, ?3, ?4, 'structure-demand', 'running',
                'restart/resume observation probe', 0,
                CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
             )",
            params![job_id, document_id, revision_id, SHADOW_ROUTING_PROFILE_ID],
        )
        .expect("restart probe job");
    job_id.to_owned()
}

fn job_state(database: &ResearchDatabase, job_id: &str) -> String {
    database
        .connect()
        .expect("job state connection")
        .query_row(
            "SELECT execution_state FROM shadow_extraction_jobs WHERE id = ?1",
            [job_id],
            |row| row.get(0),
        )
        .expect("job state")
}

fn document_for_key(database: &ResearchDatabase, attachment_key: &str) -> String {
    database
        .connect()
        .expect("document connection")
        .query_row(
            "SELECT id FROM documents WHERE zotero_attachment_key = ?1",
            [attachment_key],
            |row| row.get(0),
        )
        .expect("observation document")
}

fn current_process_rss_bytes() -> u64 {
    let status = fs::read_to_string("/proc/self/status").expect("process status");
    status
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
        .unwrap_or_default()
        * 1024
}

async fn wait_for_sidecars_to_exit() {
    for _ in 0..40 {
        if sidecar_process_count() == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn sidecar_process_count() -> usize {
    let names = [
        "graphite-pdf-benchmark-xberg",
        "graphite-pdf-benchmark-pdf-inspector",
        "graphite-pdf-benchmark-docling",
    ];
    fs::read_dir("/proc")
        .expect("process directory")
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|process_id| *process_id != std::process::id())
        .filter(|process_id| {
            fs::read(format!("/proc/{process_id}/cmdline"))
                .ok()
                .is_some_and(|command| {
                    let executable = command.split(|byte| *byte == 0).next().unwrap_or_default();
                    Path::new(std::str::from_utf8(executable).unwrap_or_default())
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| names.contains(&name))
                })
        })
        .count()
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
