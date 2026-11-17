//! Run both comparison arms through the actual local agent coordinator.
//!
//! This opt-in harness imports only corpus identities, queries and parser
//! outputs. Reference answers are deliberately unavailable to the runner.

use std::io::Read;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use rusqlite::{params, params_from_iter, types::Value as SqlValue, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::agent::grounding::is_abstention;
use crate::agent::model::{AgentEvidence, AgentRunResult};
use crate::agent::AgentCoordinator;

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageSpan {
    start: usize,
    end: usize,
    page_start: Option<u32>,
    page_end: Option<u32>,
}

/// Resolve a chunk's pages conservatively from overlapping source blocks.
fn chunk_pages(spans: &[PageSpan], start: usize, end: usize) -> Option<(u32, u32)> {
    let overlapping: Vec<_> = spans
        .iter()
        .filter(|span| span.start < end && span.end > start)
        .collect();
    if overlapping.is_empty() {
        return None;
    }
    let mut low = u32::MAX;
    let mut high = 0;
    for span in overlapping {
        low = low.min(span.page_start?);
        high = high.max(span.page_end?);
    }
    Some((low, high))
}

/// Compensate only for leading whitespace trimmed by the existing chunker.
fn exact_chunk_offsets(characters: &[char], start: usize, text: &str) -> (usize, usize) {
    let mut start = start;
    while characters
        .get(start)
        .is_some_and(|value| value.is_whitespace())
    {
        start += 1;
    }
    let end = start + text.chars().count();
    assert!(start < end && end <= characters.len());
    assert_eq!(
        characters[start..end].iter().collect::<String>(),
        text,
        "never map pages onto transformed or stale chunk text"
    );
    (start, end)
}

/// Map only exact source substrings, before the experiment becomes queryable.
fn attach_pages(
    fixture: &RepresentativeFixture,
    input: &Path,
    manifest: &RepresentativeManifest,
) -> usize {
    let connection = fixture.database.connect().expect("page connection");
    let mut mapped = 0;
    for (document, item_id) in manifest.documents.iter().zip(&fixture.item_ids) {
        let payload: Value = serde_json::from_slice(
            &std::fs::read(input.join(&document.zotero_baseline)).expect("adapter bytes"),
        )
        .expect("adapter schema");
        let spans: Vec<PageSpan> =
            serde_json::from_value(payload["spans"].clone()).expect("page spans");
        let characters: Vec<char> = payload["content"]
            .as_str()
            .expect("source text")
            .chars()
            .collect();
        let mut statement = connection
            .prepare(
                "SELECT id, character_start, character_end, text FROM semantic_chunks
             WHERE item_id=?1 AND status='active'",
            )
            .expect("chunk query");
        let chunks: Vec<(String, i64, i64, String)> = statement
            .query_map([item_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .expect("chunks")
            .collect::<Result<_, _>>()
            .expect("chunk rows");
        for (id, start, end, text) in chunks {
            let start = usize::try_from(start).expect("nonnegative offset");
            let end = usize::try_from(end).expect("nonnegative offset");
            assert!(end <= characters.len() && start < end);
            let (start, end) = exact_chunk_offsets(&characters, start, &text);
            let pages = chunk_pages(&spans, start, end);
            connection
                .execute(
                    "UPDATE semantic_chunks SET page_start=?2, page_end=?3,
                 character_start=?4, character_end=?5,
                 source_provider='docling-local-comparison' WHERE id=?1",
                    params![
                        id,
                        pages.map(|p| p.0),
                        pages.map(|p| p.1),
                        start as i64,
                        end as i64
                    ],
                )
                .expect("persist page provenance");
            mapped += usize::from(pages.is_some());
        }
    }
    mapped
}

/// Verify the installed GGUF and copy only its registry row into the fixture.
fn register_model(fixture: &RepresentativeFixture, app: &Path) -> String {
    let source = rusqlite::Connection::open_with_flags(
        app.join("graphite-research.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("read-only installed model registry");
    let (relative, expected): (String, String) = source
        .query_row(
            "SELECT cache_relative_path, sha256 FROM agent_model_installations
         WHERE profile_id='balanced' AND status='ready'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("already installed balanced model");
    let model_root = app.join("agent-models").canonicalize().expect("model root");
    let model_path = model_root
        .join(relative)
        .canonicalize()
        .expect("local GGUF");
    assert!(model_path.starts_with(&model_root));
    let mut model = std::fs::File::open(model_path).expect("read local model");
    let mut hash = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let size = model.read(&mut buffer).expect("model hash read");
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    assert_eq!(
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        expected,
        "installed model hash"
    );
    let mut statement = source
        .prepare("SELECT * FROM agent_model_installations WHERE profile_id='balanced'")
        .expect("registry schema");
    let count = statement.column_count();
    let values = statement
        .query_row([], |row| {
            (0..count)
                .map(|index| row.get::<_, SqlValue>(index))
                .collect::<Result<Vec<_>, _>>()
        })
        .expect("model registry row");
    fixture
        .database
        .connect()
        .expect("fixture registry")
        .execute(
            &format!(
                "INSERT INTO agent_model_installations VALUES ({})",
                vec!["?"; count].join(",")
            ),
            params_from_iter(values),
        )
        .expect("fixture model registration");
    expected
}

/// Preserve every retrieved source and map displayed citation IDs to it.
fn observed_result(
    result: &AgentRunResult,
    raw: &[AgentEvidence],
    documents: &HashMap<String, (String, String)>,
) -> Value {
    let mut evidence = Vec::new();
    let mut fingerprints = HashMap::new();
    let mut context_ids = Vec::new();
    for source in result.sources.iter().chain(raw) {
        if fingerprints.contains_key(&source.source_fingerprint) {
            continue;
        }
        let id = if source.source_id.is_empty() {
            format!("R{}", evidence.len() + 1)
        } else {
            source.source_id.clone()
        };
        let (document, hash) = &documents[source.item_id.as_ref().expect("source paper")];
        evidence.push(json!({"id": id, "documentId": document,
            "documentSha256": hash, "pageStart": source.page_start,
            "pageEnd": source.page_end, "passage": source.excerpt,
            "chunkId": source.chunk_id, "sourceFingerprint": source.source_fingerprint}));
        fingerprints.insert(source.source_fingerprint.clone(), id);
    }
    for source in &result.sources {
        assert!(
            raw.iter()
                .any(|r| r.source_fingerprint == source.source_fingerprint),
            "actual context must come from observed retrieval"
        );
        context_ids.push(fingerprints[&source.source_fingerprint].clone());
    }
    let mut retrieved: Vec<_> = raw
        .iter()
        .map(|source| fingerprints[&source.source_fingerprint].clone())
        .collect();
    retrieved.sort();
    retrieved.dedup();
    let mut citations: Vec<_> = result
        .citations
        .iter()
        .map(|citation| citation.source_id.clone())
        .collect();
    citations.sort();
    citations.dedup();
    json!({"status": "succeeded", "answer": result.answer_markdown,
        "abstained": is_abstention(&result.answer_markdown), "evidence": evidence,
        "retrievedEvidenceIds": retrieved, "contextEvidenceIds": context_ids,
        "citationEvidenceIds": citations, "selectionReindexCount": 0,
        "agentResult": result, "rawRetrieval": raw})
}

/// Snapshot complete active chunk rows to detect unrelated reindexing.
fn active_chunk_snapshot(fixture: &RepresentativeFixture) -> Vec<Vec<SqlValue>> {
    let connection = fixture.database.connect().expect("snapshot connection");
    let mut statement = connection
        .prepare("SELECT * FROM semantic_chunks WHERE status='active' ORDER BY id")
        .expect("snapshot query");
    let count = statement.column_count();
    statement
        .query_map([], |row| (0..count).map(|index| row.get(index)).collect())
        .expect("snapshot rows")
        .collect::<Result<_, _>>()
        .expect("snapshot values")
}

/// Apply changed sidecars through the existing incremental ingestion path.
fn apply_repaired_inputs(
    fixture: &RepresentativeFixture,
    input: &Path,
    baseline: &RepresentativeManifest,
    revised: &RepresentativeManifest,
) -> Vec<String> {
    let connection = fixture.database.connect().expect("revision connection");
    let mut changed = Vec::new();
    for (old, new) in baseline.documents.iter().zip(&revised.documents) {
        assert_eq!(old.id, new.id);
        if old.zotero_baseline_sha256 == new.zotero_baseline_sha256 {
            continue;
        }
        let bytes = std::fs::read(input.join(&new.zotero_baseline)).expect("repaired input");
        assert_eq!(sha256(&bytes), new.zotero_baseline_sha256);
        let payload: Value = serde_json::from_slice(&bytes).expect("repair metadata");
        assert_eq!(
            payload["repair"]["parentSidecarSha256"],
            old.zotero_baseline_sha256
        );
        let sidecar: ZoteroSidecar = serde_json::from_slice(&bytes).expect("repaired sidecar");
        let (source, library, version): (String, String, i64) = connection
            .query_row(
                "SELECT source_id, library_id, source_version FROM document_texts
             WHERE zotero_attachment_key=?1 AND is_active=1",
                [format!("{}-pdf", new.id)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("original text identity");
        assert!(sidecar.source_version > version);
        fixture
            .database
            .apply_fulltext_delta(
                &source,
                &library,
                &FullTextSyncDelta {
                    successful_version: sidecar.source_version,
                    records: vec![FullTextRecord {
                        attachment_key: format!("{}-pdf", new.id),
                        version: sidecar.source_version,
                        content: sidecar.content,
                        indexed_pages: sidecar.indexed_pages,
                        total_pages: sidecar.total_pages,
                        indexed_chars: sidecar.indexed_characters,
                        total_chars: sidecar.total_characters,
                    }],
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("incremental repaired extraction");
        changed.push(new.id.clone());
    }
    assert!(!changed.is_empty(), "repair must change an extraction");
    changed
}

/// Exercise the app's optional parser without exposing reference answers.
fn extract_fresh_app_documents(
    fixture: &RepresentativeFixture,
    corpus_root: &Path,
    corpus: &Value,
    output: &Path,
) {
    let coordinator = crate::extraction::docling::DoclingCoordinator::new(
        fixture.database.clone(),
        output.join("extraction"),
    )
    .expect("local extraction coordinator");
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("local extraction executor");
    for entry in corpus["documents"].as_array().expect("corpus") {
        let paper = entry["id"].as_str().expect("paper identity");
        let document: String = fixture
            .database
            .connect()
            .unwrap()
            .query_row(
                "SELECT id FROM documents WHERE zotero_attachment_key=?1",
                [format!("{paper}-pdf")],
                |row| row.get(0),
            )
            .expect("seeded document");
        executor
            .block_on(
                crate::extraction::ExtractionCoordinator::new(fixture.database.clone())
                    .canonicalize_document(&document),
            )
            .expect("retain original extraction");
        let pdf = corpus_root
            .join("local")
            .join(format!("{paper}.pdf"))
            .canonicalize()
            .expect("existing local PDF");
        let bytes = std::fs::read(&pdf).expect("PDF bytes");
        let hash = sha256(&bytes);
        assert_eq!(hash, entry["pdfSha256"].as_str().unwrap());
        fixture
            .database
            .store_document_locator(
                &document,
                &pdf,
                &format!("pdf:{hash}"),
                &hash,
                bytes.len() as u64,
                None,
            )
            .expect("verified fixture locator");
        eprintln!("fresh app extraction {paper}");
        coordinator.extract(&document).expect("real app extraction");
    }
}

/// Generate real answers without giving the model any approved reference text.
#[test]
#[ignore = "requires prepared local corpus and installed balanced GGUF"]
fn approved_questions_local_answers() {
    let arm = std::env::var("GRAPHITE_COMPARISON_ARM").expect("set comparison arm");
    assert!(["zotero", "docling"].contains(&arm.as_str()));
    let app = PathBuf::from(
        std::env::var("GRAPHITE_COMPARISON_APP_ROOT")
            .expect("set read-only installed model source"),
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let runtime = root.join(".cache/rag-comparison");
    let run_id =
        std::env::var("GRAPHITE_COMPARISON_RUN").unwrap_or_else(|_| format!("answers-{arm}-v7"));
    assert!(
        !run_id.is_empty()
            && run_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
    );
    let output = runtime.join("runs").join(run_id);
    std::fs::create_dir(&output).expect("fresh output directory required");
    let fresh_app = std::env::var("GRAPHITE_COMPARISON_FRESH_APP").as_deref() == Ok("1");
    let evaluation_id = if fresh_app {
        "docling-app-v1"
    } else {
        "upstream-rag-v1"
    };
    let bundle = root.join("evaluation").join(evaluation_id);
    let query_bytes = std::fs::read(bundle.join("queries.json")).expect("queries");
    let queries: ProbeQueries = serde_json::from_slice(&query_bytes).expect("query schema");
    let corpus: Value = serde_json::from_slice(
        &std::fs::read(bundle.join("corpus.json")).expect("corpus identities"),
    )
    .expect("corpus schema");
    let prepared_docling = arm == "docling" && !fresh_app;
    let input = if prepared_docling {
        runtime.join("prepared-docling-v1")
    } else {
        root.join("tools/pdf-benchmark/corpus")
    };
    let manifest_path = input.join(if prepared_docling {
        "manifest.json"
    } else {
        "representative-manifest.json"
    });
    let manifest_bytes = std::fs::read(manifest_path).expect("source manifest");
    let mut manifest: RepresentativeManifest =
        serde_json::from_slice(&manifest_bytes).expect("ingestion manifest");
    manifest.documents.retain(|d| {
        corpus["documents"]
            .as_array()
            .expect("corpus")
            .iter()
            .any(|entry| entry["id"] == d.id)
    });
    let mut fixture = seed_representative_database(&input, &manifest);
    let missing_baselines: Vec<String> = corpus["documents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| {
            let id = entry["id"].as_str().unwrap();
            (!manifest.documents.iter().any(|document| document.id == id)).then(|| id.to_owned())
        })
        .collect();
    if fresh_app && arm == "docling" {
        // A scanned PDF can exist without any Zotero text. Seed identities only.
        let connection = fixture.database.connect().unwrap();
        let (source, library): (String, String) = connection
            .query_row("SELECT source_id, id FROM libraries LIMIT 1", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        for id in &missing_baselines {
            fixture
                .database
                .apply_delta(
                    &source,
                    &library,
                    &LibraryDelta {
                        target_version: 1,
                        objects: vec![
                            representative_item(&format!("{id}-item"), None),
                            representative_item(&format!("{id}-pdf"), Some(&format!("{id}-item"))),
                        ],
                        deleted_items: Vec::new(),
                        deleted_collections: Vec::new(),
                    },
                )
                .expect("PDF-only identity");
            fixture.item_ids.push(
                connection
                    .query_row(
                        "SELECT id FROM research_items WHERE zotero_key=?1",
                        [format!("{id}-item")],
                        |row| row.get(0),
                    )
                    .unwrap(),
            );
            manifest.documents.push(RepresentativeDocument {
                id: id.clone(),
                zotero_baseline: String::new(),
                zotero_baseline_sha256: String::new(),
            });
        }
    }
    let ids: HashMap<_, _> = manifest
        .documents
        .iter()
        .zip(&fixture.item_ids)
        .map(|(doc, id)| (doc.id.clone(), id.clone()))
        .collect();
    let documents: HashMap<_, _> = ids
        .iter()
        .map(|(doc, item)| {
            let identity = corpus["documents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["id"] == *doc)
                .expect("PDF identity");
            (
                item.clone(),
                (
                    doc.clone(),
                    identity["pdfSha256"].as_str().unwrap().to_owned(),
                ),
            )
        })
        .collect();
    if fresh_app && arm == "docling" {
        extract_fresh_app_documents(&fixture, &input, &corpus, &output);
    }
    let semantic = SemanticCoordinator::new(
        fixture.database.clone(),
        runtime.join("graphite-semantic-models"),
        fixture.root.join("vectors"),
    )
    .expect("actual semantic coordinator");
    semantic
        .install_model()
        .expect("activate existing embedding assets");
    for _ in 0..3 {
        semantic
            .index(&SemanticIndexRequest {
                max_papers: Some(64),
                max_documents: Some(16),
            })
            .expect("index comparison corpus");
    }
    let baseline_mapped = if prepared_docling {
        attach_pages(&fixture, &input, &manifest)
    } else if fresh_app && arm == "docling" {
        fixture.database.connect().unwrap().query_row(
            "SELECT count(*) FROM semantic_chunks WHERE status='active' AND page_start IS NOT NULL",
            [], |row| row.get::<_, i64>(0),
        ).unwrap() as usize
    } else {
        0
    };
    let mut revision_audit = json!(null);
    let mut mapped = baseline_mapped;
    if let Ok(repaired_input) = std::env::var("GRAPHITE_COMPARISON_REPAIRED_INPUT") {
        assert_eq!(arm, "docling");
        let repaired_input = PathBuf::from(repaired_input);
        let bytes = std::fs::read(repaired_input.join("manifest.json")).expect("repair manifest");
        let revised: RepresentativeManifest =
            serde_json::from_slice(&bytes).expect("repair schema");
        assert_eq!(manifest.documents.len(), revised.documents.len());
        let before = active_chunk_snapshot(&fixture);
        let changed = apply_repaired_inputs(&fixture, &repaired_input, &manifest, &revised);
        for _ in 0..3 {
            semantic
                .index(&SemanticIndexRequest {
                    max_papers: Some(64),
                    max_documents: Some(16),
                })
                .expect("index repaired revision");
        }
        mapped = attach_pages(&fixture, &repaired_input, &revised);
        let after = active_chunk_snapshot(&fixture);
        let changed_ids: Vec<_> = changed
            .iter()
            .map(|id| SqlValue::Text(ids[id].clone()))
            .collect();
        let unaffected_before: Vec<_> = before
            .iter()
            .filter(|r| !changed_ids.contains(&r[1]))
            .collect();
        let unaffected_after: Vec<_> = after
            .iter()
            .filter(|r| !changed_ids.contains(&r[1]))
            .collect();
        assert_eq!(
            unaffected_before, unaffected_after,
            "unrelated chunks must remain unchanged"
        );
        let connection = fixture.database.connect().expect("revision audit");
        let mut revisions = Vec::new();
        for document in &changed {
            let item = &ids[document];
            let count: i64 = connection.query_row(
                "SELECT count(*) FROM document_revisions r JOIN documents d ON d.id=r.document_id
                 WHERE d.research_item_id=?1 AND r.revision_kind='source-provided-text'",
                [item], |row| row.get(0),
            ).expect("retained revisions");
            assert_eq!(count, 2);
            let active_versions: Vec<i64> = connection
                .prepare(
                    "SELECT DISTINCT r.source_version FROM semantic_chunks c
                 JOIN document_revisions r ON r.id=c.document_revision_id
                 WHERE c.item_id=?1 AND c.status='active'",
                )
                .expect("active version query")
                .query_map([item], |row| row.get(0))
                .expect("active versions")
                .collect::<Result<_, _>>()
                .expect("version rows");
            assert_eq!(active_versions, vec![2]);
            revisions.push(
                json!({"documentId": document, "retainedRevisionCount": count,
                "activeSourceVersions": active_versions}),
            );
        }
        revision_audit = json!({"changedDocuments": changed, "revisions": revisions,
            "unchangedOtherChunkRows": unaffected_after.len(),
            "baselineMappedChunks": baseline_mapped,
            "repairedManifestSha256": sha256(&bytes)});
    }
    let selection_snapshot = active_chunk_snapshot(&fixture);
    let model_hash = register_model(&fixture, &app);
    let agent = AgentCoordinator::new(fixture.database.clone(), semantic, app.join("agent-models"))
        .expect("actual local agent");
    let hardware = agent
        .environment()
        .expect("actual hardware profile")
        .hardware;
    let mut report = json!({"evaluationId": evaluation_id, "executionKind": "measured",
        "usesAppDoclingIntegration": fresh_app && arm == "docling",
        "papersWithoutZoteroBaseline": missing_baselines,
        "corpusSha256": sha256(&std::fs::read(bundle.join("corpus.json")).unwrap()),
        "profile": format!("{arm}-graphite-selected-only-balanced-concise-v7"),
        "softwareIdentity": sha256(include_bytes!("generated_answers.rs")),
        "hardware": hardware,
        "modelSha256": model_hash, "querySha256": sha256(&query_bytes),
        "inputManifestSha256": sha256(&manifest_bytes), "mappedChunks": mapped,
        "revisionAudit": revision_audit, "fixtureRoot": fixture.root, "networkPolicy": "linux-seccomp-network-denied",
        "limitations": ["Identifier-only paper metadata", "Concise 420-token production output budget",
            "Multi-page source blocks retain conservative page ranges", "No image interpretation model"],
        "results": []});
    let mut conversation_id = None;
    let query_filter = std::env::var("GRAPHITE_COMPARISON_QUERY").ok();
    for query in queries.questions {
        if query_filter
            .as_ref()
            .is_some_and(|id| !id.split(',').any(|selected| selected == query.id))
        {
            continue;
        }
        eprintln!("comparison arm={arm} query={} started", query.id);
        let row = if query
            .selected_document_ids
            .iter()
            .any(|id| !ids.contains_key(id))
        {
            json!({"status":"unavailable", "reason":"A selected paper has no text baseline in this arm"})
        } else {
            let selected: Vec<_> = query
                .selected_document_ids
                .iter()
                .map(|id| ids[id].clone())
                .collect();
            let request = AgentRequest {
                question: query.question,
                conversation_id: conversation_id.clone(),
                provider_kind: AgentProviderKind::GraphiteManaged,
                model_profile: "balanced".to_owned(),
                inference_backend: Default::default(),
                context_window: Default::default(),
                detail: AgentDetail::Concise,
                external_provider: None,
                context: AgentContext {
                    selected_only: true,
                    selected_item_ids: selected,
                    ..AgentContext::default()
                },
            };
            let events = Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed = events.clone();
            let cancel_agent = agent.clone();
            let timed_out = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let timer_timeout = timed_out.clone();
            let query_started = std::time::Instant::now();
            let (finished, waiting) = mpsc::channel();
            let timer = std::thread::spawn(move || {
                if waiting.recv_timeout(Duration::from_secs(240)).is_err() {
                    timer_timeout.store(true, std::sync::atomic::Ordering::SeqCst);
                    cancel_agent.cancel();
                }
            });
            let result = agent.run(
                request,
                Arc::new(move |event| {
                    observed.lock().expect("event trace").push(event);
                }),
            );
            let _ = finished.send(());
            timer.join().expect("timeout guard");
            let raw = agent
                .observed_retrieval
                .lock()
                .expect("retrieval trace")
                .clone();
            let mut row = match result {
                Ok(result) => {
                    conversation_id = Some(result.conversation_id.clone());
                    observed_result(&result, &raw, &documents)
                }
                Err(error) => {
                    json!({"status":"failed", "reason":error.to_string(), "rawRetrieval":raw})
                }
            };
            row["timedOut"] = json!(timed_out.load(std::sync::atomic::Ordering::SeqCst));
            row["elapsedSeconds"] = json!(query_started.elapsed().as_secs_f64());
            std::fs::write(
                output.join(format!("{}-events.json", query.id)),
                serde_json::to_vec(&*events.lock().expect("events")).expect("event JSON"),
            )
            .expect("retain generation trace");
            row
        };
        let mut row = row;
        row["queryId"] = json!(query.id);
        eprintln!(
            "comparison arm={arm} query={} status={}",
            query.id, row["status"]
        );
        report["results"]
            .as_array_mut()
            .expect("result array")
            .push(row);
        std::fs::write(
            output.join("results.json"),
            serde_json::to_vec_pretty(&report).expect("report JSON"),
        )
        .expect("checkpoint each answer");
    }
    assert_eq!(
        selection_snapshot,
        active_chunk_snapshot(&fixture),
        "selection must not reindex"
    );
    report["indexUnchangedAcrossSelectionSwitch"] = json!(true);
    report["hardwareAfter"] = json!(
        agent
            .environment()
            .expect("final runtime hardware")
            .hardware
    );
    std::fs::write(
        output.join("results.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .expect("final audit checkpoint");
    agent.unload_model().expect("unload model");
    // Keep the disposable database for failed-answer and provenance inspection.
}

#[test]
fn chunk_page_ranges_keep_unknowns_and_cross_page_evidence() {
    let spans = [
        PageSpan {
            start: 0,
            end: 3,
            page_start: Some(2),
            page_end: Some(2),
        },
        PageSpan {
            start: 5,
            end: 15,
            page_start: Some(3),
            page_end: Some(4),
        },
        PageSpan {
            start: 17,
            end: 24,
            page_start: None,
            page_end: None,
        },
    ];
    assert_eq!(chunk_pages(&spans, 1, 3), Some((2, 2)));
    assert_eq!(chunk_pages(&spans, 1, 8), Some((2, 4)));
    assert_eq!(chunk_pages(&spans, 6, 15), Some((3, 4)));
    assert_eq!(chunk_pages(&spans, 12, 20), None);
    assert_eq!(chunk_pages(&spans, 25, 30), None);
    let characters: Vec<char> = "\u{03bd}\n| value |".chars().collect();
    assert_eq!(exact_chunk_offsets(&characters, 1, "| value |"), (2, 11));
    assert!(std::panic::catch_unwind(|| exact_chunk_offsets(&characters, 0, "wrong")).is_err());
}

/// Recheck saved visible answers through the current production acceptance gate.
#[test]
#[ignore = "requires a saved local answer run"]
fn replay_saved_answer_requirements() {
    let path = std::env::var("GRAPHITE_COMPARISON_REPLAY_RESULTS").expect("set saved result path");
    let run: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut validated = 0;
    for row in run["results"].as_array().unwrap() {
        if row["status"] != "succeeded" {
            continue;
        }
        let result = &row["agentResult"];
        let evidence: Vec<crate::agent::model::AgentEvidence> =
            serde_json::from_value(result["sources"].clone()).unwrap();
        let intent: crate::agent::model::AgentIntent =
            serde_json::from_value(result["intent"].clone()).unwrap();
        crate::agent::grounding::validate_answer_for_question(
            result["question"].as_str().unwrap(),
            row["answer"].as_str().unwrap(),
            &intent,
            &evidence,
        )
        .unwrap_or_else(|issues| panic!("{}: {issues:?}", row["queryId"]));
        validated += 1;
        eprintln!("Validated saved answer {}", row["queryId"]);
    }
    assert!(validated > 0, "No successful answers to validate");
}
