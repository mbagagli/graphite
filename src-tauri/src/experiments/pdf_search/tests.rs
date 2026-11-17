use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::database::ResearchDatabase;
use crate::error::{AppError, AppResult};
use crate::model::{
    FullTextRecord, FullTextSyncDelta, LibraryDelta, RawObject, RemoteLibrary, SearchPage,
    SearchRequest, SemanticIndexRequest, SourceProbe,
};
use crate::semantic::chunking::{scientific_chunks, DocumentSource};
use crate::semantic::vector_index::UsearchVectorIndex;
use crate::semantic::{
    embedding_profile_id, EmbeddingProvider, EmbeddingProviderMetadata, LocalFastEmbedProvider,
    SemanticCoordinator, EMBEDDING_DIMENSION,
};

use super::coordinator::PdfSearchExperimentCoordinator;
use super::index::ExperimentIndexer;
use super::model::{
    sha256, CharacterizationQuery, ExperimentCancellationToken, ExperimentDefinition,
    ExperimentRetrievalKind, ProductionSnapshot, SnapshotChunk, SnapshotEmbeddingProfile,
    SnapshotItem, SnapshotPaperVector,
};
use super::runtime::ExperimentRuntime;
use super::snapshot::ReadOnlyProductionSnapshot;
use super::store::ExperimentStore;

mod upstream_comparison;

#[derive(Clone)]
struct DeterministicEmbeddingProvider;

#[derive(Clone)]
struct SlowEmbeddingProvider;

impl EmbeddingProvider for DeterministicEmbeddingProvider {
    fn installed_metadata(&self) -> AppResult<Option<EmbeddingProviderMetadata>> {
        Ok(Some(fake_metadata()))
    }

    fn install(&self) -> AppResult<EmbeddingProviderMetadata> {
        Ok(fake_metadata())
    }

    fn embed_documents(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|text| fake_vector(text)).collect())
    }

    fn embed_query(&self, query: &str) -> AppResult<Vec<f32>> {
        Ok(fake_vector(query))
    }

    fn token_count(&self, text: &str) -> AppResult<usize> {
        Ok(text.split_whitespace().count().max(1))
    }
}

impl EmbeddingProvider for SlowEmbeddingProvider {
    fn installed_metadata(&self) -> AppResult<Option<EmbeddingProviderMetadata>> {
        DeterministicEmbeddingProvider.installed_metadata()
    }

    fn install(&self) -> AppResult<EmbeddingProviderMetadata> {
        DeterministicEmbeddingProvider.install()
    }

    fn embed_documents(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        DeterministicEmbeddingProvider.embed_documents(texts)
    }

    fn embed_query(&self, query: &str) -> AppResult<Vec<f32>> {
        DeterministicEmbeddingProvider.embed_query(query)
    }

    fn token_count(&self, text: &str) -> AppResult<usize> {
        std::thread::sleep(std::time::Duration::from_millis(75));
        DeterministicEmbeddingProvider.token_count(text)
    }
}

#[test]
fn cancellation_restart_idempotency_and_exact_cleanup_are_safe() {
    let provider: Arc<dyn EmbeddingProvider> = Arc::new(DeterministicEmbeddingProvider);
    let root = temporary_root("lifecycle");
    let definition = definition();
    let snapshot = synthetic_snapshot(provider.as_ref());
    let runtime = ExperimentRuntime::prepare(&root, &definition).expect("runtime");
    let cancellation = ExperimentCancellationToken::default();
    cancellation.cancel();
    let cancelled = ExperimentIndexer::new(provider.clone()).index_zotero_mirror(
        &runtime,
        &definition,
        &snapshot,
        &cancellation,
    );
    assert!(matches!(cancelled, Err(AppError::Semantic(_))));
    assert!(!runtime.database_path().exists());
    assert!(!runtime.manifest_path().exists());
    std::fs::write(runtime.database_path(), b"interrupted publication")
        .expect("partial publication fixture");

    let coordinator = PdfSearchExperimentCoordinator::with_provider(root.clone(), provider);
    let first = coordinator
        .build_zotero_mirror(&definition, &snapshot)
        .expect("restart build");
    assert_eq!(first.item_count, 2);
    assert!(first.chunk_count > 0);
    assert!(!first.idempotent_reuse);
    let second = coordinator
        .build_zotero_mirror(&definition, &snapshot)
        .expect("idempotent build");
    assert_eq!(second.chunk_count, first.chunk_count);
    assert_eq!(second.provenance_count, first.provenance_count);
    assert!(second.idempotent_reuse);
    let published =
        ExperimentStore::open_read_only(&runtime.database_path()).expect("published executions");
    let execution_count: i64 = published
        .connection()
        .query_row("SELECT COUNT(*) FROM experiment_executions", [], |row| {
            row.get(0)
        })
        .expect("execution history");
    assert_eq!(execution_count, 2);
    drop(published);

    let query = CharacterizationQuery {
        id: "synthetic-method".to_owned(),
        query: "method measurements".to_owned(),
        item_ids: snapshot
            .items
            .iter()
            .map(|item| item.item_id.clone())
            .collect(),
    };
    let page = coordinator
        .search_zotero_mirror(&definition, &query, ExperimentRetrievalKind::Hybrid, 20)
        .expect("experimental query");
    assert_eq!(page.mode.as_str(), "experimental-zotero-mirror");
    assert!(!page.results.is_empty());

    let other = ExperimentDefinition::frozen(
        "other-corpus",
        definition.embedding_profile_id.clone(),
        definition.embedding_artifact_identity.clone(),
    );
    let other_runtime = ExperimentRuntime::prepare(&root, &other).expect("other runtime");
    let other_marker = other_runtime.definition_root().join("results").join("keep");
    std::fs::write(&other_marker, b"keep").expect("other marker");
    coordinator
        .cleanup(&definition.definition_id)
        .expect("exact cleanup");
    assert!(!runtime.definition_root().exists());
    assert!(other_marker.is_file());
    other_runtime.cleanup().expect("other cleanup");
    remove_empty_test_root(&root);
}

#[test]
fn published_store_is_read_only_for_queries_and_has_separate_schema() {
    let provider: Arc<dyn EmbeddingProvider> = Arc::new(DeterministicEmbeddingProvider);
    let root = temporary_root("read-only");
    let definition = definition();
    let snapshot = synthetic_snapshot(provider.as_ref());
    let coordinator = PdfSearchExperimentCoordinator::with_provider(root.clone(), provider);
    coordinator
        .build_zotero_mirror(&definition, &snapshot)
        .expect("mirror build");
    let runtime = ExperimentRuntime::existing(&root, &definition.definition_id).expect("runtime");
    let store = ExperimentStore::open_read_only(&runtime.database_path()).expect("read-only store");
    assert!(store
        .connection()
        .execute("CREATE TABLE production_write_probe(id INTEGER)", [])
        .is_err());
    let experiment_tables: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name LIKE 'experiment_%'",
            [],
            |row| row.get(0),
        )
        .expect("experiment schema");
    assert!(experiment_tables >= 5);
    drop(store);
    coordinator
        .cleanup(&definition.definition_id)
        .expect("cleanup");
    remove_empty_test_root(&root);
}

#[test]
fn running_build_can_be_cancelled_without_publication_or_leaks() {
    let root = temporary_root("running-cancellation");
    let definition = definition();
    let snapshot = synthetic_snapshot(&DeterministicEmbeddingProvider);
    let coordinator = PdfSearchExperimentCoordinator::with_provider(
        root.clone(),
        Arc::new(SlowEmbeddingProvider),
    );
    let worker = coordinator.clone();
    let worker_definition = definition.clone();
    let handle =
        std::thread::spawn(move || worker.build_zotero_mirror(&worker_definition, &snapshot));
    let mut cancellation_observed = false;
    for _ in 0..100 {
        if coordinator
            .cancel(&definition.definition_id)
            .expect("request cancellation")
        {
            cancellation_observed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(cancellation_observed);
    let outcome = handle.join().expect("experiment worker");
    assert!(matches!(outcome, Err(AppError::Semantic(_))));
    let runtime =
        ExperimentRuntime::existing(&root, &definition.definition_id).expect("cancelled runtime");
    assert!(!runtime.database_path().exists());
    assert!(!runtime.manifest_path().exists());
    coordinator
        .cleanup(&definition.definition_id)
        .expect("cancelled cleanup");
    remove_empty_test_root(&root);
}

#[test]
fn normal_semantic_search_cannot_activate_an_experimental_mode() {
    let root = temporary_root("normal-search-isolation");
    let database_path = root.join("production.sqlite3");
    let vector_root = root.join("production-vectors");
    let database = ResearchDatabase::initialize(database_path.clone()).expect("production DB");
    let semantic = SemanticCoordinator::with_components(
        database.clone(),
        Arc::new(DeterministicEmbeddingProvider),
        Arc::new(UsearchVectorIndex::new(vector_root).expect("production vectors")),
    );
    let page = semantic
        .search(&SearchRequest {
            query: "probe".to_owned(),
            mode: Some("experimental-zotero-mirror".to_owned()),
            limit: None,
            offset: None,
            item_ids: Vec::new(),
            collection_id: None,
            citation_filter: None,
            year_from: None,
            year_to: None,
            item_type: None,
        })
        .expect("normal search");
    assert_eq!(page.retrieval_mode, "keyword");
    let connection = database.connect().expect("production schema");
    let experiment_tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name LIKE 'experiment_%'",
            [],
            |row| row.get(0),
        )
        .expect("production schema query");
    assert_eq!(experiment_tables, 0);
    drop(connection);
    drop(semantic);
    drop(database);
    std::fs::remove_file(database_path).expect("remove production DB fixture");
    std::fs::remove_dir_all(root).expect("remove normal search fixture");
}

fn definition() -> ExperimentDefinition {
    let metadata = fake_metadata();
    ExperimentDefinition::frozen(
        "synthetic-corpus-manifest",
        embedding_profile_id(&metadata),
        metadata.artifact_identity,
    )
}

fn fake_metadata() -> EmbeddingProviderMetadata {
    EmbeddingProviderMetadata {
        provider: "deterministic-experiment-test".to_owned(),
        model_identifier: "deterministic-384".to_owned(),
        model_revision: "1".to_owned(),
        artifact_identity: "deterministic-artifact-v1".to_owned(),
        dimension: EMBEDDING_DIMENSION,
        metric: "cosine".to_owned(),
        query_template: "{query}".to_owned(),
        document_template: "{document}".to_owned(),
        normalization: "l2-unit".to_owned(),
        runtime_version: "deterministic-test-v1".to_owned(),
        installed_bytes: 0,
    }
}

fn synthetic_snapshot(provider: &dyn EmbeddingProvider) -> ProductionSnapshot {
    let metadata = fake_metadata();
    let profile_id = embedding_profile_id(&metadata);
    let items = vec![
        snapshot_item(
            "item-a",
            "document-a",
            "Alpha measurements",
            "The method uses calibrated measurements.\n\nThe results confirm the alpha model.",
        ),
        snapshot_item(
            "item-b",
            "document-b",
            "Beta observations",
            "Observations constrain the beta model.\n\nThe analysis reports stable results.",
        ),
    ];
    let mut vector_key = 1_u64;
    let mut chunks = Vec::new();
    for item in &items {
        let source = DocumentSource {
            item_id: item.item_id.clone(),
            library_id: item.library_id.clone(),
            document_id: item.document_id.clone(),
            document_revision_id: item.document_revision_id,
            provider: "zotero-indexed".to_owned(),
            text: item.body.clone(),
            body_end_byte: item.body_end_byte,
            bibliography_excluded: false,
        };
        for chunk in scientific_chunks(provider, &source).expect("scientific chunks") {
            let vector = provider
                .embed_documents(std::slice::from_ref(&chunk.text))
                .expect("chunk vector")
                .remove(0);
            chunks.push(SnapshotChunk {
                id: chunk.id,
                item_id: item.item_id.clone(),
                document_id: item.document_id.clone(),
                document_revision_id: item.document_revision_id,
                ordinal: chunk.ordinal as u32,
                text: chunk.text,
                character_start: chunk.character_start as u64,
                character_end: chunk.character_end as u64,
                page_start: None,
                page_end: None,
                fingerprint: chunk.fingerprint,
                vector_key,
                vector,
            });
            vector_key += 1;
        }
    }
    let paper_vectors = items
        .iter()
        .map(|item| {
            let key = vector_key;
            vector_key += 1;
            SnapshotPaperVector {
                item_id: item.item_id.clone(),
                representation_kind: "title-only".to_owned(),
                vector_key: key,
                vector: fake_vector(&item.title),
            }
        })
        .collect();
    ProductionSnapshot {
        embedding_profile: SnapshotEmbeddingProfile {
            id: profile_id,
            artifact_identity: metadata.artifact_identity,
            dimension: EMBEDDING_DIMENSION,
        },
        items,
        chunks,
        paper_vectors,
        shadow_projections: Vec::new(),
    }
}

fn snapshot_item(item_id: &str, document_id: &str, title: &str, body: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.to_owned(),
        library_id: "library".to_owned(),
        title: title.to_owned(),
        creators: String::new(),
        tags: String::new(),
        abstract_text: String::new(),
        venue: String::new(),
        document_id: document_id.to_owned(),
        document_revision_id: 1,
        source_version: 1,
        body: body.to_owned(),
        body_end_byte: body.len(),
    }
}

fn fake_vector(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; EMBEDDING_DIMENSION];
    for (index, byte) in text.bytes().enumerate() {
        vector[index % EMBEDDING_DIMENSION] += f32::from(byte) / 255.0;
    }
    let norm = vector
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt()
        .max(f32::EPSILON);
    for value in &mut vector {
        *value /= norm;
    }
    vector
}

fn temporary_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "graphite-pdf-search-{label}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir(&root).expect("test root");
    root
}

fn remove_empty_test_root(root: &PathBuf) {
    let experiments = root.join("experiments").join("pdf-search-v1");
    if experiments.is_dir() {
        std::fs::remove_dir(&experiments).expect("empty experiment profile root");
    }
    let parent = root.join("experiments");
    if parent.is_dir() {
        std::fs::remove_dir(&parent).expect("empty experiment root");
    }
    std::fs::remove_dir(root).expect("empty test root");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepresentativeManifest {
    documents: Vec<RepresentativeDocument>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepresentativeDocument {
    id: String,
    zotero_baseline: String,
    zotero_baseline_sha256: String,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RealMirrorEvidence {
    definition_id: String,
    corpus_manifest_sha256: String,
    document_count: usize,
    characterization_query_count: usize,
    comparison_count: u32,
    equivalent: bool,
    differences: Vec<super::model::MirrorEquivalenceDifference>,
    item_count: u64,
    chunk_count: u64,
    paper_vector_count: u64,
    embedded_chunk_count: u64,
    sqlite_bytes: u64,
    vector_bytes: u64,
    production_database_unchanged: bool,
    production_vectors_unchanged: bool,
    cleanup_passed: bool,
}

#[test]
#[ignore = "requires frozen local Zotero sidecars and the installed production BGE assets"]
fn real_twenty_document_zotero_mirror_is_retrieval_equivalent() {
    let corpus_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tools/pdf-benchmark/corpus");
    let manifest_path = corpus_root.join("representative-manifest.json");
    let manifest_bytes = std::fs::read(&manifest_path).expect("representative manifest");
    let manifest: RepresentativeManifest =
        serde_json::from_slice(&manifest_bytes).expect("representative manifest JSON");
    assert_eq!(manifest.documents.len(), 20);
    let model_root = std::env::var_os("GRAPHITE_M6C_MODEL_ROOT")
        .map(PathBuf::from)
        .expect("GRAPHITE_M6C_MODEL_ROOT must name the installed model root");
    let fixture = seed_representative_database(&corpus_root, &manifest);
    let production_vector_root = fixture.root.join("production-vectors");
    let provider: Arc<dyn EmbeddingProvider> =
        Arc::new(LocalFastEmbedProvider::new(model_root).expect("installed embedding provider"));
    let vectors = Arc::new(
        UsearchVectorIndex::new(production_vector_root.clone())
            .expect("production fixture vector index"),
    );
    let semantic =
        SemanticCoordinator::with_components(fixture.database.clone(), provider.clone(), vectors);
    semantic.install_model().expect("activate installed model");
    for _ in 0..3 {
        semantic
            .index(&SemanticIndexRequest {
                max_papers: Some(64),
                max_documents: Some(16),
            })
            .expect("index representative corpus");
    }
    let snapshot = ReadOnlyProductionSnapshot::capture(
        &fixture.database_path,
        &production_vector_root,
        &fixture.item_ids,
    )
    .expect("read-only production snapshot");
    assert_eq!(snapshot.items.len(), 20);
    assert!(!snapshot.chunks.is_empty());
    let corpus_hash = sha256(&manifest_bytes);
    let experiment_root = fixture.root.join("experiment-app-data");
    std::fs::create_dir(&experiment_root).expect("experiment app-data root");
    let coordinator =
        PdfSearchExperimentCoordinator::with_provider(experiment_root.clone(), provider);
    let definition = coordinator.definition(&corpus_hash).expect("definition");
    let database_before = file_sha256(&fixture.database_path);
    let vectors_before = vector_file_hashes(&production_vector_root);
    let summary = coordinator
        .build_zotero_mirror(&definition, &snapshot)
        .expect("real Zotero mirror");
    let queries: Vec<CharacterizationQuery> =
        serde_json::from_slice(include_bytes!("characterization_queries.json"))
            .expect("characterization query JSON");
    let mut observations =
        Vec::<(CharacterizationQuery, ExperimentRetrievalKind, SearchPage)>::new();
    for query in &queries {
        let scoped_query = CharacterizationQuery {
            item_ids: fixture.item_ids.clone(),
            ..query.clone()
        };
        for kind in [
            ExperimentRetrievalKind::Lexical,
            ExperimentRetrievalKind::Semantic,
            ExperimentRetrievalKind::Hybrid,
        ] {
            let production = semantic
                .search(&SearchRequest {
                    query: scoped_query.query.clone(),
                    mode: Some(
                        match kind {
                            ExperimentRetrievalKind::Lexical => "keyword",
                            ExperimentRetrievalKind::Semantic => "semantic",
                            ExperimentRetrievalKind::Hybrid => "hybrid",
                        }
                        .to_owned(),
                    ),
                    limit: Some(20),
                    offset: Some(0),
                    item_ids: fixture.item_ids.clone(),
                    collection_id: None,
                    citation_filter: None,
                    year_from: None,
                    year_to: None,
                    item_type: None,
                })
                .expect("production characterization search");
            observations.push((scoped_query.clone(), kind, production));
        }
    }
    let equivalence = coordinator
        .compare_zotero_mirror(&definition, &observations)
        .expect("mirror comparison");
    let database_after = file_sha256(&fixture.database_path);
    let vectors_after = vector_file_hashes(&production_vector_root);
    let production_database_unchanged = database_before == database_after;
    let production_vectors_unchanged = vectors_before == vectors_after;
    assert!(production_database_unchanged);
    assert!(production_vectors_unchanged);
    assert!(
        equivalence.equivalent,
        "unexplained mirror differences: {:?}",
        equivalence.differences
    );
    coordinator
        .cleanup(&definition.definition_id)
        .expect("experiment cleanup");
    assert_eq!(database_before, file_sha256(&fixture.database_path));
    assert_eq!(vectors_before, vector_file_hashes(&production_vector_root));
    let cleanup_passed = !experiment_root
        .join("experiments/pdf-search-v1")
        .join(&definition.definition_id)
        .exists();
    assert!(cleanup_passed);
    let evidence = RealMirrorEvidence {
        definition_id: definition.definition_id,
        corpus_manifest_sha256: corpus_hash,
        document_count: fixture.item_ids.len(),
        characterization_query_count: queries.len(),
        comparison_count: equivalence.comparison_count,
        equivalent: equivalence.equivalent,
        differences: equivalence.differences,
        item_count: summary.item_count,
        chunk_count: summary.chunk_count,
        paper_vector_count: summary.paper_vector_count,
        embedded_chunk_count: summary.embedded_chunk_count,
        sqlite_bytes: summary.sqlite_bytes,
        vector_bytes: summary.vector_bytes,
        production_database_unchanged,
        production_vectors_unchanged,
        cleanup_passed,
    };
    let results_root = corpus_root.join("../results");
    std::fs::create_dir_all(&results_root).expect("local results root");
    std::fs::write(
        results_root.join("m6c1-zotero-mirror-equivalence.json"),
        serde_json::to_vec_pretty(&evidence).expect("equivalence evidence JSON"),
    )
    .expect("equivalence evidence");
    fixture.cleanup();
}

struct RepresentativeFixture {
    database: ResearchDatabase,
    database_path: PathBuf,
    root: PathBuf,
    item_ids: Vec<String>,
}

impl RepresentativeFixture {
    fn cleanup(self) {
        drop(self.database);
        std::fs::remove_dir_all(&self.root).expect("remove representative fixture");
    }
}

fn seed_representative_database(
    corpus_root: &std::path::Path,
    manifest: &RepresentativeManifest,
) -> RepresentativeFixture {
    let root = temporary_root("real-mirror");
    let database_path = root.join("production.sqlite3");
    let database = ResearchDatabase::initialize(database_path.clone()).expect("production fixture");
    let source = database
        .register_source(&SourceProbe {
            server_id: "m6c1-zotero-mirror".to_owned(),
            api_version: "3".to_owned(),
            schema_version: None,
        })
        .expect("fixture source");
    let library = database
        .register_libraries(
            &source.id,
            &[RemoteLibrary {
                kind: "user".to_owned(),
                zotero_id: "0".to_owned(),
                name: "M6-C1 representative mirror".to_owned(),
            }],
        )
        .expect("fixture library")
        .remove(0);
    let objects = manifest
        .documents
        .iter()
        .flat_map(|document| {
            [
                representative_item(&format!("{}-item", document.id), None),
                representative_item(
                    &format!("{}-pdf", document.id),
                    Some(&format!("{}-item", document.id)),
                ),
            ]
        })
        .collect();
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
        .expect("fixture metadata");
    let mut records = Vec::new();
    for document in &manifest.documents {
        let sidecar_path = corpus_root.join(&document.zotero_baseline);
        let bytes = std::fs::read(&sidecar_path).expect("local Zotero sidecar");
        assert_eq!(sha256(&bytes), document.zotero_baseline_sha256);
        let sidecar: ZoteroSidecar =
            serde_json::from_slice(&bytes).expect("local Zotero sidecar JSON");
        records.push(FullTextRecord {
            attachment_key: format!("{}-pdf", document.id),
            version: sidecar.source_version,
            content: sidecar.content,
            indexed_pages: sidecar.indexed_pages,
            total_pages: sidecar.total_pages,
            indexed_chars: sidecar.indexed_characters,
            total_chars: sidecar.total_characters,
        });
    }
    database
        .apply_fulltext_delta(
            &source.id,
            &library.id,
            &FullTextSyncDelta {
                successful_version: 1,
                records,
                removed_attachment_keys: Vec::new(),
                warning_count: 0,
                status: "ready".to_owned(),
                last_error: None,
            },
        )
        .expect("fixture Zotero text");
    let connection = database.connect().expect("fixture identities");
    let item_ids = manifest
        .documents
        .iter()
        .map(|document| {
            connection
                .query_row(
                    "SELECT id FROM research_items WHERE zotero_key = ?1",
                    [format!("{}-item", document.id)],
                    |row| row.get::<_, String>(0),
                )
                .expect("fixture item identity")
        })
        .collect();
    drop(connection);
    RepresentativeFixture {
        database,
        database_path,
        root,
        item_ids,
    }
}

fn representative_item(key: &str, parent: Option<&str>) -> RawObject {
    let data = parent.map_or_else(
        || {
            serde_json::json!({
                "key": key,
                "version": 1,
                "itemType": "journalArticle",
                "title": format!("Representative {key}"),
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

fn file_sha256(path: &std::path::Path) -> String {
    sha256(&std::fs::read(path).expect("hash file"))
}

fn vector_file_hashes(root: &std::path::Path) -> Vec<(String, String)> {
    let mut values = std::fs::read_dir(root)
        .expect("vector root")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                file_sha256(&entry.path()),
            )
        })
        .collect::<Vec<_>>();
    values.sort();
    values
}
