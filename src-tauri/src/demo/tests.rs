//! Exercise source isolation, integrity failures, and optional real files.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::citation::{reference_page, CitationCoordinator};
use crate::corpus::CorpusCoordinator;
use crate::document::DocumentCoordinator;
use crate::model::{ReferenceQueryRequest, RemoteLibrary, SearchRequest, SourceProbe};
use crate::semantic::SemanticCoordinator;
use crate::sync::SyncCoordinator;
use crate::zotero::ZoteroReadClient;

use super::*;

fn fixture() -> (tempfile::TempDir, ResearchDatabase, DemoCoordinator) {
    let directory = tempfile::tempdir().expect("isolated test directory");
    let database =
        ResearchDatabase::initialize(directory.path().join("research.sqlite3")).expect("database");
    let coordinator = DemoCoordinator::new(
        database.clone(),
        directory.path().join("pdfs"),
        Arc::new(tokio::sync::Mutex::new(())),
    );
    (directory, database, coordinator)
}

fn original_source(database: &ResearchDatabase) -> String {
    let source = database
        .register_source(&SourceProbe {
            server_id: "fixture-original-source".into(),
            api_version: "3".into(),
            schema_version: None,
        })
        .expect("source");
    let library = database
        .register_libraries(
            &source.id,
            &[RemoteLibrary {
                kind: "user".into(),
                zotero_id: "0".into(),
                name: "Existing cache".into(),
            }],
        )
        .expect("library")
        .remove(0);
    database
        .connect()
        .expect("connection")
        .execute(
            "INSERT INTO research_items
        (id,source_id,library_id,zotero_key,item_type,title,source_version,is_trashed)
        VALUES ('existing-paper',?1,?2,'EXISTING','journalArticle','Existing cached paper',7,0)",
            params![source.id, library.id],
        )
        .expect("cached record");
    database
        .connect()
        .expect("connection")
        .execute(
            "INSERT INTO app_meta(key,value) VALUES ('fixture-preference','retained')",
            [],
        )
        .expect("preference");
    library.id
}

#[test]
fn metadata_is_real_unique_and_restores_the_original_partition() {
    let (_directory, database, coordinator) = fixture();
    let original = original_source(&database);
    let workspace = coordinator.activate(true).expect("activate demo");
    assert_eq!(workspace.source.mode, "demo");
    assert_eq!(workspace.snapshot.provenance, "real-demo");
    assert_eq!(workspace.snapshot.total_item_count, 60);
    assert_eq!(
        workspace
            .snapshot
            .items
            .iter()
            .map(|item| item.doi.as_deref().expect("real DOI"))
            .collect::<HashSet<_>>()
            .len(),
        60
    );
    assert!(workspace
        .snapshot
        .items
        .iter()
        .all(|item| !item.authors.is_empty() && item.attachments.len() == 1));
    let ids: HashSet<_> = workspace
        .snapshot
        .items
        .into_iter()
        .map(|item| item.id)
        .collect();
    let connection = database.connect().expect("connection");
    connection
        .execute(
            "UPDATE libraries SET name='Previous demo title' WHERE id=?1",
            [LIBRARY_ID],
        )
        .expect("previous library name");
    let mut retired = catalog::DemoSourceProvider
        .delta()
        .expect("catalogue delta");
    retired.knowledge_items.truncate(1);
    retired.knowledge_items[0].external_id = "retired-demo-paper".into();
    retired.artifacts.clear();
    crate::knowledge::store::KnowledgeStore::new(database.clone())
        .apply_source_delta(catalog::SOURCE_ID, LIBRARY_ID, &retired)
        .expect("previous catalogue record");
    assert_eq!(
        database
            .load_workspace()
            .expect("previous catalogue")
            .snapshot
            .total_item_count,
        61
    );
    let repeated = coordinator.activate(true).expect("repeat demo activation");
    let name: String = connection
        .query_row(
            "SELECT name FROM libraries WHERE id=?1",
            [LIBRARY_ID],
            |row| row.get(0),
        )
        .expect("current demo name");
    assert_eq!(name, "Machine learning in seismology: 60 real papers");
    assert_eq!(
        ids,
        repeated
            .snapshot
            .items
            .into_iter()
            .map(|item| item.id)
            .collect()
    );
    let restored = coordinator
        .activate(false)
        .expect("restore original source");
    assert_eq!(
        restored.snapshot.active_library_id.as_deref(),
        Some(original.as_str())
    );
    assert_eq!(restored.snapshot.total_item_count, 1);
    assert_eq!(restored.snapshot.items[0].id, "existing-paper");
    assert_eq!(
        database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT value FROM app_meta WHERE key='fixture-preference'",
                [],
                |r| r.get::<_, String>(0)
            )
            .expect("retained preference"),
        "retained"
    );
}

#[test]
fn missing_or_modified_pdfs_fail_without_touching_the_original_cache() {
    let (directory, database, coordinator) = fixture();
    let original = original_source(&database);
    coordinator.activate(true).expect("activate demo");
    let empty = directory.path().join("empty");
    fs::create_dir(&empty).expect("empty folder");
    assert!(coordinator
        .acquire(Some(&empty), Arc::new(|_| {}))
        .expect_err("missing PDFs")
        .to_string()
        .contains("Missing PDF"));
    assert_eq!(coordinator.status().expect("status").pdf_count, 0);
    let paper = manifest().expect("manifest").papers.remove(0);
    assert!(validate_bytes(&paper, b"%PDF-invalid").is_err());
    assert!(bounded_read(
        &Paper {
            pdf_bytes: 5,
            ..paper.clone()
        },
        &b"%PDF-too-long"[..],
        &|_| {},
        &|| Ok(())
    )
    .is_err());
    assert!(
        bounded_read(&paper, &b"%PDF-partial"[..], &|_| {}, &|| Err(failure(
            "Demo preparation cancelled."
        )))
        .expect_err("cancellation")
        .to_string()
        .contains("cancelled")
    );
    assert!(text::store_pages(&database, "unknown", &paper, vec![String::new()]).is_err());
    assert_eq!(
        coordinator
            .activate(false)
            .expect("restore")
            .snapshot
            .active_library_id
            .as_deref(),
        Some(original.as_str())
    );
}

#[tokio::test]
async fn demo_document_and_sync_paths_never_fall_back_to_zotero() {
    let (_directory, database, coordinator) = fixture();
    let workspace = tokio::task::spawn_blocking(move || coordinator.activate(true))
        .await
        .expect("worker")
        .expect("demo");
    let client = ZoteroReadClient::with_root("http://127.0.0.1:9/api".into())
        .expect("unreachable fixture client");
    let documents = DocumentCoordinator::with_client(database.clone(), client.clone());
    let result = documents
        .resolve(&workspace.snapshot.items[0].attachments[0].document_id)
        .await
        .expect("local missing-PDF result");
    assert_eq!(result.status, "unavailable");
    let corpus = CorpusCoordinator::with_client(database.clone(), client.clone());
    assert_eq!(
        corpus
            .synchronize()
            .await
            .expect("local corpus")
            .snapshot
            .total_item_count,
        60
    );
    let sync = SyncCoordinator::with_client(database, client);
    assert!(sync
        .probe()
        .await
        .expect_err("guarded probe")
        .to_string()
        .contains("Switch to Zotero"));
    assert!(sync
        .synchronize()
        .await
        .expect_err("guarded sync")
        .to_string()
        .contains("Switch to Zotero"));
    assert!(sync
        .rebuild()
        .await
        .expect_err("guarded rebuild")
        .to_string()
        .contains("Switch to Zotero"));
}

fn store_real_texts(
    database: &ResearchDatabase,
    coordinator: &DemoCoordinator,
    status: &DemoStatus,
    text_root: &Path,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let documents = DocumentCoordinator::with_client(
        database.clone(),
        ZoteroReadClient::with_root("http://127.0.0.1:9/api".into())
            .expect("unreachable fixture client"),
    );
    for document in &status.documents {
        assert_eq!(
            runtime
                .block_on(documents.resolve(&document.document_id))
                .expect("resolve own cached PDF")
                .status,
            "ready"
        );
        let pages: Vec<_> =
            fs::read_to_string(text_root.join(format!("{}.txt", document.paper_id)))
                .expect("PDF.js extracted text")
                .split("\n\u{c}\n")
                .map(str::to_owned)
                .collect();
        assert_eq!(pages.len(), document.pages as usize);
        assert!(
            pages.iter().any(|page| page.lines().count() > 5),
            "PDF.js text must preserve line endings for bibliography extraction"
        );
        coordinator
            .store_pages(&document.document_id, pages)
            .expect("store canonical page text");
        let stored = database
            .document_text(&document.document_id)
            .expect("read stored document")
            .expect("active canonical text");
        assert!(stored.content.chars().count() > 100);
    }
}

#[test]
#[ignore = "requires verified public PDFs, PDF.js text and a fresh explicit output directory"]
fn real_collection_citations_connect_new_papers_and_export_workspace() {
    let pdfs = PathBuf::from(std::env::var("GRAPHITE_DEMO_PDFS").expect("verified PDF directory"));
    let text_root =
        PathBuf::from(std::env::var("GRAPHITE_DEMO_TEXT").expect("verified PDF.js text directory"));
    let output = PathBuf::from(
        std::env::var("GRAPHITE_DEMO_OUTPUT").expect("fresh explicit output directory"),
    );
    fs::create_dir(&output).expect("output directory must not already exist");
    let database =
        ResearchDatabase::initialize(output.join("demo.sqlite3")).expect("fresh demo database");
    let coordinator = DemoCoordinator::new(
        database.clone(),
        output.join("pdfs"),
        Arc::new(tokio::sync::Mutex::new(())),
    );
    let original = original_source(&database);
    let catalogue = manifest().expect("catalogue");
    assert_eq!(catalogue.papers.len(), 60);
    let initial = coordinator.activate(true).expect("activate real demo");
    assert_eq!(initial.snapshot.total_item_count, 60);
    assert_eq!(initial.citations.citation_edges, 0);
    let status = coordinator
        .acquire(Some(&pdfs), Arc::new(|_| {}))
        .expect("import all verified PDFs");
    assert_eq!(status.pdf_count, 60);
    let originals = if pdfs.join("pdfs").is_dir() {
        pdfs.join("pdfs")
    } else {
        pdfs
    };
    for paper in &catalogue.papers {
        validate_file(paper, &originals.join(format!("{}.pdf", paper.id)))
            .expect("unchanged original PDF");
        validate_file(paper, &coordinator.root.join(format!("{}.pdf", paper.id)))
            .expect("verified cached PDF");
    }
    store_real_texts(&database, &coordinator, &status, &text_root);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("citation runtime");
    let citations =
        CitationCoordinator::new(database.clone(), Arc::new(tokio::sync::Mutex::new(())));
    let mut indexed = runtime
        .block_on(citations.index(None, Some(100)))
        .expect("index actual PDF bibliographies");
    for _ in 0..10 {
        if indexed.citations.queued_resolutions == 0 {
            break;
        }
        indexed = runtime
            .block_on(citations.reresolve())
            .expect("finish bibliography resolution");
    }
    assert_eq!(indexed.corpus.full_text_count, 60);
    assert_eq!(indexed.corpus.resolved_pdf_count, 60);
    assert_eq!(indexed.citations.indexed_documents, 60);
    assert_eq!(indexed.citations.queued_documents, 0);
    assert_eq!(indexed.citations.queued_resolutions, 0);
    assert_eq!(indexed.citations.failed_documents, 0);
    let keys: BTreeMap<_, _> = indexed
        .snapshot
        .items
        .iter()
        .map(|item| (item.id.clone(), item.provenance.zotero_key.clone()))
        .collect();
    let mut reference_pairs = BTreeSet::new();
    let mut self_matches = 0;
    let mut references = Vec::new();
    let mut coverage = Vec::new();
    for item in &indexed.snapshot.items {
        let mut offset = 0;
        loop {
            let page = reference_page(
                &database,
                &ReferenceQueryRequest {
                    item_id: item.id.clone(),
                    filter: Some("local".into()),
                    limit: Some(100),
                    offset: Some(offset),
                },
            )
            .expect("inspect extracted local reference evidence");
            if offset == 0 {
                coverage.push(serde_json::json!({
                    "paperId": keys[&item.id], "coverage": page.coverage,
                }));
            }
            for reference in &page.references {
                let target = reference.matched_item.as_ref().expect("local target");
                assert!(!reference.raw_text.trim().is_empty());
                assert!(keys.contains_key(&target.id));
                if target.id == item.id {
                    self_matches += 1;
                } else {
                    reference_pairs.insert((item.id.clone(), target.id.clone()));
                }
                references.push(serde_json::json!({
                    "source": keys[&item.id], "target": keys[&target.id],
                    "reference": reference,
                }));
            }
            offset += page.references.len() as u32;
            if u64::from(offset) >= page.total {
                break;
            }
            assert!(!page.references.is_empty());
        }
    }
    let connection = database
        .connect()
        .expect("inspect persisted citation edges");
    let pairs: BTreeSet<(String, String)> = connection
        .prepare(
            "SELECT ce.source_item_id, ce.target_item_id FROM citation_edges ce
         JOIN research_items source ON source.id=ce.source_item_id
         WHERE source.library_id=?1",
        )
        .expect("native citation edge query")
        .query_map([LIBRARY_ID], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("native citation pairs")
        .collect::<Result<_, _>>()
        .expect("valid native citation pairs");
    assert!(pairs.is_subset(&reference_pairs));
    assert!(pairs.iter().all(|(source, target)| source != target));
    assert_eq!(pairs.len() as u64, indexed.citations.citation_edges);
    let displayed: BTreeSet<_> = indexed
        .graph
        .edges
        .iter()
        .filter(|edge| edge.relation == "citation")
        .map(|edge| {
            assert!(edge.directed && edge.evidence_count > 0);
            (edge.source.clone(), edge.target.clone())
        })
        .collect();
    assert!(displayed.is_subset(&pairs));
    let isolates: Vec<_> = keys
        .iter()
        .filter(|(id, _)| {
            !pairs
                .iter()
                .any(|(source, target)| source == *id || target == *id)
        })
        .map(|(_, key)| key.clone())
        .collect();
    let displayed_isolates: Vec<_> = keys
        .iter()
        .filter(|(id, _)| {
            !displayed
                .iter()
                .any(|(source, target)| source == *id || target == *id)
        })
        .map(|(_, key)| key.clone())
        .collect();
    let default_edges: Vec<_> = indexed
        .graph
        .edges
        .iter()
        .filter(|edge| {
            matches!(
                edge.relation.as_str(),
                "citation" | "bibliographic-coupling" | "co-cited"
            )
        })
        .collect();
    for edge in &default_edges {
        if edge.relation == "shared-tag" {
            let source = indexed
                .snapshot
                .items
                .iter()
                .find(|item| item.id == edge.source)
                .expect("tag source");
            let target = indexed
                .snapshot
                .items
                .iter()
                .find(|item| item.id == edge.target)
                .expect("tag target");
            assert!(source.tags.iter().any(|tag| target.tags.contains(tag)));
        }
    }
    let graph_isolates: Vec<_> = keys
        .iter()
        .filter(|(id, _)| {
            !default_edges
                .iter()
                .any(|edge| &edge.source == *id || &edge.target == *id)
        })
        .map(|(_, key)| key.clone())
        .collect();
    let reviewed: serde_json::Value =
        serde_json::from_str(include_str!("../../../demo/connections.json"))
            .expect("audited real bibliography connections");
    assert_eq!(reviewed["manifestId"], catalogue.id);
    let reviewed_edges = reviewed["edges"].as_array().expect("reviewed edges");
    let original_ids: BTreeSet<_> = reviewed["originalPaperIds"]
        .as_array()
        .expect("original paper identities")
        .iter()
        .map(|id| id.as_str().expect("original identity"))
        .collect();
    let audited_pairs: BTreeSet<_> = reviewed_edges
        .iter()
        .map(|edge| {
            (
                edge["source"].as_str().expect("reviewed source"),
                edge["target"].as_str().expect("reviewed target"),
            )
        })
        .collect();
    for edge in reviewed_edges {
        let source = catalogue
            .papers
            .iter()
            .find(|paper| paper.id == edge["source"])
            .expect("audited source in real manifest");
        assert_eq!(source.sha256, edge["sourcePdfSha256"]);
    }

    let key_pairs: BTreeSet<_> = pairs
        .iter()
        .map(|(source, target)| (keys[source].as_str(), keys[target].as_str()))
        .collect();
    let missing_reviewed: Vec<_> = reviewed_edges
        .iter()
        .filter(|edge| {
            !key_pairs.contains(&(
                edge["source"].as_str().expect("reviewed source"),
                edge["target"].as_str().expect("reviewed target"),
            ))
        })
        .cloned()
        .collect();
    let new_papers: Vec<_> = catalogue.papers.iter().skip(30).map(|paper| {
        let id = keys.iter().find(|(_, key)| *key == &paper.id)
            .expect("new real paper in workspace").0;
        serde_json::json!({
            "paperId": paper.id,
            "nativeCitationDegree": pairs.iter().filter(|(a, b)| a == id || b == id).count(),
            "displayedRelationshipDegree": default_edges.iter()
                .filter(|edge| &edge.source == id || &edge.target == id).count(),
            "auditedLinksToOriginalPapers": audited_pairs.iter().filter(|(a, b)| {
                (*a == paper.id && original_ids.contains(b))
                    || (*b == paper.id && original_ids.contains(a))
            }).count(),
            "displayedCitationDegree": displayed.iter().filter(|(a, b)| a == id || b == id).count(),
        })
    }).collect();
    coordinator.activate(true).expect("repeat activation");
    let repeated = runtime
        .block_on(citations.index(None, Some(100)))
        .expect("idempotent citation indexing");
    assert_eq!(repeated.snapshot.total_item_count, 60);
    assert_eq!(repeated.corpus.full_text_count, 60);
    assert_eq!(
        indexed.citations.references_extracted,
        repeated.citations.references_extracted
    );
    assert_eq!(
        indexed.citations.citation_edges,
        repeated.citations.citation_edges
    );
    assert_eq!(
        keys.keys().collect::<BTreeSet<_>>(),
        repeated
            .snapshot
            .items
            .iter()
            .map(|item| &item.id)
            .collect::<BTreeSet<_>>()
    );
    let restored = coordinator
        .activate(false)
        .expect("restore previous source");
    assert_eq!(
        restored.snapshot.active_library_id.as_deref(),
        Some(original.as_str())
    );
    assert_eq!(restored.snapshot.items.len(), 1);
    assert_eq!(restored.snapshot.items[0].id, "existing-paper");
    coordinator
        .activate(true)
        .expect("restore exported demo workspace");
    fs::write(
        output.join("workspace.json"),
        serde_json::to_vec_pretty(&repeated).expect("serialize native workspace"),
    )
    .expect("export native workspace");
    fs::write(
        output.join("references.json"),
        serde_json::to_vec_pretty(&references).expect("serialize reference evidence"),
    )
    .expect("export native reference evidence");
    let mut evidence = serde_json::json!({
        "acceptancePassed": false,
        "manifestId": catalogue.id, "papers": 60,
        "pdfsResolvedAndChecksummed": 60, "fullTextRecords": 60,
        "pages": catalogue.papers.iter().map(|paper| paper.pdf_pages).sum::<u32>(),
        "duplicateSyncPreservedIds": true, "sourceRestored": true,
        "generatedAnswerVerified": false, "networkUsed": false,
        "citationDiagnostics": repeated.citations,
        "nativeCitationEdges": pairs.len(), "displayedCitationEdges": displayed.len(),
        "totalGraphEdges": indexed.graph.edges.len(),
        "defaultVisibleEdges": default_edges.len(),
        "displayedSharedTagEdges": 0,
        "nativeCitationIsolates": isolates, "displayedCitationIsolates": displayed_isolates,
        "graphIsolates": graph_isolates, "selfReferenceRowsExcludedFromEdges": self_matches,
        "newPapers": new_papers, "referenceCoverage": coverage,
        "auditedConnectionCount": reviewed_edges.len(),
        "auditedConnectionsRecovered": reviewed_edges.len() - missing_reviewed.len(),
        "missingAuditedConnections": missing_reviewed,
    });
    fs::write(
        output.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence).expect("serialize integration evidence"),
    )
    .expect("export integration evidence");
    println!("Real demo: {} extracted references, {} native citation edges, {} displayed edges, {} native isolates, {} displayed isolates",
        repeated.citations.references_extracted, pairs.len(), displayed.len(),
        isolates.len(), displayed_isolates.len());
    assert!(
        key_pairs.is_subset(&audited_pairs),
        "every native citation must match independently reviewed PDF evidence"
    );
    // Scientific coverage may leave isolates, just as in a Zotero library.
    assert_eq!(indexed.graph.nodes.len(), catalogue.papers.len());
    for paper in &new_papers {
        assert!(
            paper["auditedLinksToOriginalPapers"].as_u64().unwrap() > 0,
            "new paper lacks reviewed bibliography evidence linking an original: {}",
            paper["paperId"]
        );
    }
    evidence["acceptancePassed"] = serde_json::json!(true);
    fs::write(
        output.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence).expect("serialize passing evidence"),
    )
    .expect("record passed native acceptance");
}

#[test]
#[ignore = "requires verified public demo PDFs, PDF.js text and an existing embedding model cache"]
fn real_collection_imports_indexes_and_restores_without_zotero() {
    let pdfs = PathBuf::from(std::env::var("GRAPHITE_DEMO_PDFS").expect("verified PDF directory"));
    let original_pdfs = if pdfs.join("pdfs").is_dir() {
        pdfs.join("pdfs")
    } else {
        pdfs.clone()
    };
    let text_root =
        PathBuf::from(std::env::var("GRAPHITE_DEMO_TEXT").expect("verified PDF.js text directory"));
    let models = PathBuf::from(
        std::env::var("GRAPHITE_DEMO_MODEL_CACHE").expect("existing embedding model cache"),
    );
    assert!(models.join("graphite-semantic-model.json").is_file());
    let (directory, database, coordinator) = fixture();
    let original = original_source(&database);
    let initial = coordinator
        .activate(true)
        .expect("activate real collection");
    assert_eq!(initial.snapshot.total_item_count, 60);
    let cancelled = coordinator.clone();
    let partial = coordinator.acquire(
        Some(&pdfs),
        Arc::new(move |event| {
            if event.completed >= 1 && event.paper_id.is_some() {
                cancelled.cancel();
            }
        }),
    );
    assert!(partial
        .expect_err("cancel between documents")
        .to_string()
        .contains("cancelled"));
    assert_eq!(coordinator.status().expect("partial status").pdf_count, 1);
    assert_eq!(
        coordinator
            .status()
            .expect("cancelled status")
            .progress
            .expect("progress")
            .phase,
        "cancelled"
    );
    let first = manifest().expect("manifest").papers.remove(0);
    let cached = coordinator.root.join(format!("{}.pdf", first.id));
    let mut corrupted = fs::read(&cached).expect("cached PDF");
    let last = corrupted.len() - 1;
    corrupted[last] ^= 1;
    fs::write(&cached, corrupted).expect("simulate damaged app cache");
    let status = coordinator
        .acquire(
            Some(&pdfs),
            Arc::new(|event| {
                if event.paper_id.is_none() {
                    println!(
                        "Demo {}: {}/{} PDFs",
                        event.phase, event.completed, event.total
                    );
                }
            }),
        )
        .expect("verified folder import");
    assert_eq!(status.pdf_count, 60);
    validate_file(&first, &cached).expect("repaired cache");
    validate_file(&first, &original_pdfs.join(format!("{}.pdf", first.id)))
        .expect("original folder unchanged");
    store_real_texts(&database, &coordinator, &status, &text_root);
    let workspace = database.load_workspace().expect("real workspace");
    assert_eq!(workspace.corpus.full_text_count, 60);
    let wave = workspace
        .snapshot
        .items
        .iter()
        .find(|item| item.doi.as_deref() == Some("10.5194/se-11-1527-2020"))
        .expect("wave paper");
    let semantic =
        SemanticCoordinator::new(database.clone(), models, directory.path().join("vectors"))
            .expect("semantic service");
    semantic
        .install_model()
        .expect("register existing cached model");
    semantic
        .index_document(&wave.attachments[0].document_id)
        .expect("index actual PDF passages");
    let result = semantic
        .search(&SearchRequest {
            query: "WaveNet conditional autoencoder seismic waves".into(),
            mode: Some("hybrid".into()),
            limit: Some(5),
            offset: None,
            item_ids: vec![wave.id.clone()],
            collection_id: None,
            citation_filter: None,
            year_from: None,
            year_to: None,
            item_type: None,
        })
        .expect("actual passage retrieval");
    assert!(!result.results.is_empty());
    let connection = database.connect().expect("connection");
    let pages:i64=connection.query_row("SELECT COUNT(*) FROM semantic_chunks WHERE document_id=?1 AND page_start IS NOT NULL AND page_end IS NOT NULL",[&wave.attachments[0].document_id],|r|r.get(0)).expect("page-aware passages");
    assert!(pages > 0);
    let text_before = database
        .document_text(&wave.attachments[0].document_id)
        .expect("text")
        .expect("active text")
        .content;
    let path_before = database
        .document_path(&wave.attachments[0].document_id)
        .expect("locator");
    let repeated = coordinator
        .activate(true)
        .expect("reactivate prepared collection");
    assert_eq!(repeated.corpus.full_text_count, 60);
    assert_eq!(
        database
            .document_text(&wave.attachments[0].document_id)
            .expect("text after switch")
            .expect("active text after switch")
            .content,
        text_before
    );
    assert_eq!(
        database
            .document_path(&wave.attachments[0].document_id)
            .expect("locator after switch"),
        path_before
    );
    let preserved:i64=connection.query_row("SELECT COUNT(*) FROM semantic_chunks WHERE document_id=?1 AND page_start IS NOT NULL AND page_end IS NOT NULL",[&wave.attachments[0].document_id],|r|r.get(0)).expect("preserved semantic passages");
    assert_eq!(preserved, pages);
    assert_eq!(
        coordinator
            .activate(false)
            .expect("restore source")
            .snapshot
            .active_library_id
            .as_deref(),
        Some(original.as_str())
    );
    assert_eq!(
        database
            .load_workspace()
            .expect("original workspace")
            .snapshot
            .total_item_count,
        1
    );
    println!("Verified 60 PDFs, 60 canonical texts, actual page-aware retrieval, and source restoration.");
}

#[test]
fn collection_contract_matches_the_manifest_and_survives_upgrade() {
    let (_directory, database, coordinator) = fixture();
    coordinator.activate(true).unwrap();
    let initial = database.load_workspace().unwrap();
    let ids: Vec<_> = initial
        .snapshot
        .items
        .iter()
        .map(|p| p.id.clone())
        .collect();
    // Emulate the prior installation with metadata but no collections.
    let connection = database.connect().unwrap();
    connection
        .execute(
            "DELETE FROM item_collections WHERE collection_id IN
        (SELECT id FROM collections WHERE library_id=?1)",
            [LIBRARY_ID],
        )
        .unwrap();
    connection
        .execute("DELETE FROM collections WHERE library_id=?1", [LIBRARY_ID])
        .unwrap();
    for _ in 0..2 {
        let workspace = coordinator.activate(true).unwrap();
        assert_eq!(
            ids,
            workspace
                .snapshot
                .items
                .iter()
                .map(|p| p.id.clone())
                .collect::<Vec<_>>()
        );
        let root = workspace
            .snapshot
            .collections
            .iter()
            .find(|c| c.parent_id.is_none())
            .unwrap();
        assert_eq!(root.name, "Machine learning in seismology");
        let manifest = catalog::manifest().unwrap();
        let themes: BTreeSet<_> = manifest
            .papers
            .iter()
            .flat_map(|p| p.themes.iter())
            .collect();
        assert_eq!(workspace.snapshot.collections.len(), themes.len() + 1);
        for collection in &workspace.snapshot.collections {
            if collection.id != root.id {
                assert_eq!(collection.parent_id.as_ref(), Some(&root.id));
                assert!(themes.contains(&collection.name));
            }
            let expected: BTreeSet<_> = manifest
                .papers
                .iter()
                .filter(|p| collection.id == root.id || p.themes.contains(&collection.name))
                .map(|p| p.doi.clone())
                .collect();
            let page = database
                .query_items(&crate::model::ItemQueryRequest {
                    collection_id: Some(collection.id.clone()),
                    limit: Some(200),
                    item_type: None,
                    year_from: None,
                    year_to: None,
                    tag: None,
                    sort: None,
                    offset: None,
                })
                .unwrap();
            assert_eq!(
                page.items
                    .iter()
                    .map(|p| p.doi.clone().unwrap())
                    .collect::<BTreeSet<_>>(),
                expected
            );
            let projection = database
                .graph_projection(&crate::model::GraphProjectionRequest {
                    collection_id: Some(collection.id.clone()),
                    seed_item_ids: Vec::new(),
                    limit: Some(200),
                    offset: None,
                })
                .unwrap();
            assert_eq!(
                projection
                    .items
                    .iter()
                    .map(|p| p.doi.clone().unwrap())
                    .collect::<BTreeSet<_>>(),
                expected
            );
        }
    }
}
