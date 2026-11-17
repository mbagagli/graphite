//! Check extraction revision isolation and real offline conversion.

use super::*;
use crate::extraction::coordinator::tests::{full_text, prepared_database};
use crate::extraction::ExtractionCoordinator;
use crate::model::FullTextSyncDelta;
use crate::semantic::chunking::{DocumentSource, ScientificChunk};

fn sync(database: &ResearchDatabase, source: &str, library: &str, text: &str) {
    database
        .apply_fulltext_delta(
            source,
            library,
            &FullTextSyncDelta {
                successful_version: 3,
                records: vec![full_text("PDF1", text)],
                removed_attachment_keys: Vec::new(),
                warning_count: 0,
                status: "ready".into(),
                last_error: None,
            },
        )
        .unwrap();
}

fn identity(database: &ResearchDatabase) -> String {
    database
        .connect()
        .unwrap()
        .query_row(
            "SELECT id FROM documents WHERE zotero_attachment_key='PDF1'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn attempt(document: &str, revision: i64) -> ExtractionAttempt {
    let canonical = canonical_document(
        WorkerResult {
            schema_version: 1,
            input_sha256: "pdf-hash".into(),
            page_count: 2,
            blocks: vec![
                WorkerBlock {
                    id: "first".into(),
                    text: "Reliable page one.".into(),
                    label: "text".into(),
                    pages: vec![1],
                },
                WorkerBlock {
                    id: "table".into(),
                    text: "| value | unit |\n| --- | --- |\n| 10 | m |".into(),
                    label: "table-markdown".into(),
                    pages: vec![2],
                },
            ],
        },
        document,
        revision,
        "pdf-hash",
    )
    .unwrap();
    ExtractionAttempt {
        provider: descriptor(),
        canonical_document: Some(canonical),
        provenance: BTreeMap::new(),
        observations: ExtractionObservations::unreported(PROFILE, document, revision, "pdf-hash"),
        warnings: Vec::new(),
        error: None,
    }
}

#[tokio::test]
async fn alternatives_keep_revisions_pages_and_survive_source_sync() {
    let (db, path, source, library) = prepared_database();
    sync(&db, &source, &library, "Original source text.");
    let document = identity(&db);
    ExtractionCoordinator::new(db.clone())
        .canonicalize_document(&document)
        .await
        .unwrap();
    let cache = path.with_extension("docling");
    let coordinator = DoclingCoordinator::new(db.clone(), cache.clone()).unwrap();
    let original = coordinator.status(&document).unwrap().choices[0].id.clone();
    let revision: i64 = db
        .connect()
        .unwrap()
        .query_row(
            "SELECT id FROM document_revisions WHERE document_id=?1",
            [&document],
            |r| r.get(0),
        )
        .unwrap();
    persist_attempt(&db, attempt(&document, revision)).unwrap();
    let active = db.document_text(&document).unwrap().unwrap();
    assert!(active.provider.starts_with("canonical:docling"));
    assert!(active.content.contains("| 10 | m |"));
    let version: i64 = db
        .connect()
        .unwrap()
        .query_row(
            "SELECT source_version FROM document_texts WHERE document_id=?1 AND is_active=1",
            [&document],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, 4);
    let source_document = DocumentSource {
        item_id: "unused".into(),
        library_id: library.clone(),
        document_id: document.clone(),
        document_revision_id: revision,
        provider: active.provider.clone(),
        body_end_byte: active.content.len(),
        text: active.content.clone(),
        bibliography_excluded: false,
    };
    let spans =
        crate::extraction::pages::active_spans(&db.connect().unwrap(), &source_document).unwrap();
    let chunk = ScientificChunk {
        id: "chunk".into(),
        ordinal: 0,
        text: "Reliable page one.".into(),
        character_start: 0,
        character_end: 18,
        token_count: 3,
        fingerprint: "hash".into(),
        vector_key: 1,
    };
    assert_eq!(
        crate::extraction::pages::chunk_location(&source_document, &chunk, &spans)
            .unwrap()
            .2,
        Some((1, 1))
    );
    sync(
        &db,
        &source,
        &library,
        "New Zotero text must not replace the manual selection.",
    );
    assert_eq!(
        db.document_text(&document).unwrap().unwrap().content,
        active.content
    );
    assert!(!ExtractionCoordinator::new(db.clone())
        .canonicalize_document(&document)
        .await
        .unwrap());
    assert_eq!(coordinator.status(&document).unwrap().choices.len(), 2);
    coordinator.select(&document, &original).unwrap();
    assert_eq!(
        db.document_text(&document).unwrap().unwrap().content,
        "Original source text."
    );
    coordinator
        .update(&document, "running", "Extracting")
        .unwrap();
    let reopened = DoclingCoordinator::new(db.clone(), cache).unwrap();
    assert_eq!(
        reopened.status(&document).unwrap().job.status,
        "interrupted"
    );
    assert!(reopened.select("another-document", &original).is_err());
}

#[test]
fn rejects_wrong_identity_empty_text_and_invalid_pages() {
    for (hash, pages, text) in [
        ("wrong", vec![1], "text"),
        ("expected", vec![3], "text"),
        ("expected", vec![1], ""),
    ] {
        let result = WorkerResult {
            schema_version: 1,
            input_sha256: hash.into(),
            page_count: 2,
            blocks: vec![WorkerBlock {
                id: "b".into(),
                text: text.into(),
                label: "text".into(),
                pages,
            }],
        };
        assert!(canonical_document(result, "document", 1, "expected").is_err());
    }
}

#[tokio::test]
#[ignore = "requires existing local Docling and embedding environments"]
async fn fresh_papers_use_real_app_extraction_and_page_indexing() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let runtime = root.join(".cache/rag-comparison");
    let output = PathBuf::from(
        std::env::var("GRAPHITE_DOCLING_TEST_OUTPUT").expect("fresh output directory"),
    );
    std::fs::create_dir(&output).unwrap();
    let mut reports = Vec::new();
    for paper in ["paper13", "paper17", "paper25", "paper31"] {
        eprintln!("Fresh app extraction: {paper}");
        let (db, path, source, library) = prepared_database();
        let baseline = root.join(format!(
            "tools/pdf-benchmark/corpus/local/{paper}.zotero-fulltext.json"
        ));
        if baseline.is_file() {
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&baseline).unwrap()).unwrap();
            sync(&db, &source, &library, value["content"].as_str().unwrap());
            ExtractionCoordinator::new(db.clone())
                .canonicalize_document(&identity(&db))
                .await
                .unwrap();
        }
        let document = identity(&db);
        let pdf = root
            .join(format!("tools/pdf-benchmark/corpus/local/{paper}.pdf"))
            .canonicalize()
            .unwrap();
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(&pdf).unwrap();
        let hash = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        db.store_document_locator(
            &document,
            &pdf,
            &format!("pdf:{hash}"),
            &hash,
            bytes.len() as u64,
            None,
        )
        .unwrap();
        let cache = output.join(paper);
        let coordinator = DoclingCoordinator::new(db.clone(), cache.clone()).unwrap();
        coordinator.extract(&document).unwrap();
        let semantic = crate::semantic::SemanticCoordinator::new(
            db.clone(),
            runtime.join("graphite-semantic-models"),
            cache.join("vectors"),
        )
        .unwrap();
        semantic
            .install_model()
            .expect("reuse already cached assets under network denial");
        semantic.index_document(&document).unwrap();
        let connection = db.connect().unwrap();
        let (chunks, pages): (i64,i64) = connection.query_row("SELECT count(*),count(page_start) FROM semantic_chunks WHERE document_id=?1 AND status='active'", [&document], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert!(chunks > 0);
        assert_eq!(chunks, pages);
        sync(
            &db,
            &source,
            &library,
            "Inactive source text updated during Docling use.",
        );
        let preserved: i64 = connection
            .query_row(
                "SELECT count(*) FROM semantic_chunks WHERE document_id=?1 AND status='active'",
                [&document],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preserved, chunks);
        assert!(!ExtractionCoordinator::new(db.clone())
            .canonicalize_document(&document)
            .await
            .unwrap());
        let text = db.document_text(&document).unwrap().unwrap();
        std::fs::write(cache.join("active-text.txt"), &text.content).unwrap();
        reports.push(serde_json::json!({"documentId":paper,"pdfSha256":hash,"fixtureDatabase":path,"documentIdentity":document,"mappedChunks":chunks,"status":coordinator.status(&document).unwrap(),"textCharacters":text.content.chars().count()}));
        std::fs::write(
            output.join("results.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
    }
}

#[tokio::test]
async fn identical_text_with_new_page_provenance_gets_a_new_revision() {
    let (db, _, source, library) = prepared_database();
    sync(&db, &source, &library, "Reliable page one.");
    let document = identity(&db);
    ExtractionCoordinator::new(db.clone())
        .canonicalize_document(&document)
        .await
        .unwrap();
    let revision: i64 = db
        .connect()
        .unwrap()
        .query_row(
            "SELECT id FROM document_revisions WHERE document_id=?1",
            [&document],
            |row| row.get(0),
        )
        .unwrap();
    let mut converted = attempt(&document, revision);
    let canonical = converted.canonical_document.as_mut().unwrap();
    canonical.blocks.truncate(1);
    canonical.reading_order.truncate(1);
    persist_attempt(&db, converted.clone()).unwrap();
    let read =
        || {
            db.connect().unwrap().query_row(
        "SELECT source_version FROM document_texts WHERE document_id=?1 AND is_active=1",
        [&document], |row| row.get::<_, i64>(0),
    ).unwrap()
        };
    assert_eq!(read(), 4);
    assert!(db
        .document_text(&document)
        .unwrap()
        .unwrap()
        .provider
        .starts_with("canonical:docling"));
    persist_attempt(&db, converted).unwrap();
    assert_eq!(read(), 4);
}

#[test]
fn worker_guard_reaps_an_interrupted_process() {
    let process = Command::new("sleep").arg("30").spawn().unwrap();
    let pid = process.id();
    drop(WorkerChild(process));
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
}

#[test]
#[ignore = "requires completed fresh runtime fixture"]
fn real_worker_cancellation_retains_active_text() {
    let output = PathBuf::from(std::env::var("GRAPHITE_DOCLING_TEST_OUTPUT").unwrap());
    let reports: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("results.json")).unwrap()).unwrap();
    let first = &reports[0];
    let db =
        ResearchDatabase::initialize(PathBuf::from(first["fixtureDatabase"].as_str().unwrap()))
            .unwrap();
    let document = first["documentIdentity"].as_str().unwrap().to_owned();
    let before = db.document_text(&document).unwrap().unwrap().content;
    let coordinator = DoclingCoordinator::new(db.clone(), output.join("cancellation")).unwrap();
    let worker = coordinator.clone();
    let worker_document = document.clone();
    let handle = std::thread::spawn(move || worker.extract(&worker_document));
    let started = Instant::now();
    while coordinator.status(&document).unwrap().job.status != "running" {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    coordinator.cancel(&document).unwrap();
    let error = handle.join().unwrap().unwrap_err().to_string();
    assert!(error.starts_with("Local PDF extraction:"));
    assert!(error.contains("cancelled"));
    assert!(error.contains("Saved text was retained"));
    assert!(!error.contains("data directory"));
    assert_eq!(
        coordinator.status(&document).unwrap().job.status,
        "cancelled"
    );
    assert_eq!(
        db.document_text(&document).unwrap().unwrap().content,
        before
    );
}

#[test]
fn malformed_job_status_does_not_prevent_opening_the_app() {
    let (db, path, _, _) = prepared_database();
    let cache = path.with_extension("invalid-job");
    std::fs::create_dir(&cache).unwrap();
    std::fs::write(cache.join("job.json"), "interrupted-write").unwrap();
    let coordinator = DoclingCoordinator::new(db, cache).unwrap();
    assert_eq!(coordinator.status("document").unwrap().job.status, "failed");
}

#[test]
#[ignore = "requires completed fresh runtime fixtures"]
fn fresh_extractions_reindex_with_current_page_mapping() {
    let output = PathBuf::from(std::env::var("GRAPHITE_DOCLING_TEST_OUTPUT").unwrap());
    let reports: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("results.json")).unwrap()).unwrap();
    let runtime = PathBuf::from(std::env::var("GRAPHITE_DOCLING_RUNTIME").unwrap());
    for report in reports.as_array().unwrap() {
        let db = ResearchDatabase::initialize(PathBuf::from(
            report["fixtureDatabase"].as_str().unwrap(),
        ))
        .unwrap();
        let document = report["documentIdentity"].as_str().unwrap();
        let connection = db.connect().unwrap();
        connection
            .execute(
                "DELETE FROM semantic_document_states WHERE document_id=?1",
                [document],
            )
            .unwrap();
        let cache = output.join(report["documentId"].as_str().unwrap());
        let semantic = crate::semantic::SemanticCoordinator::new(
            db,
            runtime.join("graphite-semantic-models"),
            cache.join("vectors"),
        )
        .unwrap();
        semantic.index_document(document).unwrap();
        let count: i64 = connection.query_row(
            "SELECT count(*) FROM semantic_chunks WHERE document_id=?1 AND status='active' AND page_start IS NOT NULL",
            [document], |row| row.get(0),
        ).unwrap();
        assert_eq!(count, report["mappedChunks"].as_i64().unwrap());
    }
}

#[tokio::test]
async fn visibility_profile_retains_the_original_extraction() {
    let (db, path, source, library) = prepared_database();
    sync(&db, &source, &library, "Original source text.");
    let document = identity(&db);
    let revision: i64 = db
        .connect()
        .unwrap()
        .query_row(
            "SELECT id FROM document_revisions WHERE document_id=?1",
            [&document],
            |row| row.get(0),
        )
        .unwrap();
    let mut previous = attempt(&document, revision);
    let old_profile = "docling-local-2.142.1-v1";
    previous.provider.profile_id = old_profile.into();
    previous.provider.mapping_profile = "native-text-and-markdown-tables-v1".into();
    previous
        .canonical_document
        .as_mut()
        .unwrap()
        .extraction_profile_id = old_profile.into();
    previous.canonical_document.as_mut().unwrap().blocks[0].content =
        CanonicalBlockContent::Other {
            label: "text".into(),
            text: "Hidden table from the original extraction.".into(),
        };
    persist_attempt(&db, previous).unwrap();
    let coordinator =
        DoclingCoordinator::new(db.clone(), path.with_extension("visibility")).unwrap();
    let old_id = coordinator.status(&document).unwrap().choices[0].id.clone();
    persist_attempt(&db, attempt(&document, revision)).unwrap();
    let status = coordinator.status(&document).unwrap();
    assert_eq!(status.choices.len(), 2);
    assert!(status.choices.iter().any(|choice| choice.id == old_id));
    let original: String = db
        .connect()
        .unwrap()
        .query_row(
            "SELECT canonical_json FROM document_extractions WHERE id=?1",
            [&old_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(original.contains("Hidden table from the original extraction."));
    let active = db.document_text(&document).unwrap().unwrap();
    assert!(active.content.contains("Reliable page one."));
    assert!(!active.content.contains("Hidden table"));
}

#[test]
fn extraction_errors_do_not_claim_the_app_directory_is_unavailable() {
    let error = failure("Extraction cancelled. Saved text was retained.");
    assert!(matches!(error, AppError::Extraction(_)));
    assert_eq!(
        error.to_string(),
        "Local PDF extraction: Extraction cancelled. Saved text was retained."
    );
}
