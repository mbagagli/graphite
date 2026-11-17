//! Verify the public demo against an explicitly isolated Zotero instance.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::corpus::CorpusCoordinator;
use crate::database::ResearchDatabase;
use crate::document::DocumentCoordinator;
use crate::model::SearchRequest;
use crate::sync::SyncCoordinator;
use crate::zotero::ZoteroReadClient;

#[tokio::test]
#[ignore = "requires the real demo in an isolated Zotero profile"]
async fn imports_real_demo_without_duplicates_and_exports_preview() {
    let root = std::env::var("GRAPHITE_DEMO_ZOTERO_ROOT")
        .expect("set the isolated Zotero API URL explicitly");
    let url = reqwest::Url::parse(&root).expect("valid isolated API URL");
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert_ne!(url.port_or_known_default(), Some(23119));
    assert_eq!(url.path(), "/api");
    assert!(url.query().is_none() && url.fragment().is_none());
    let output = PathBuf::from(
        std::env::var("GRAPHITE_DEMO_OUTPUT")
            .expect("set a new disposable output directory explicitly"),
    );
    fs::create_dir(&output).expect("output directory must not already exist");
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../demo/manifest.json")).expect("demo manifest");
    let records = manifest["papers"].as_array().expect("manifest papers");
    assert_eq!(records.len(), 60);
    let database = ResearchDatabase::initialize(output.join("demo.sqlite3"))
        .expect("fresh application database");
    let client = ZoteroReadClient::with_root(root).expect("isolated client");
    let sync = SyncCoordinator::with_client(database.clone(), client.clone());
    let corpus = CorpusCoordinator::with_client(database.clone(), client.clone());
    let documents = DocumentCoordinator::with_client(database.clone(), client);
    let initial = sync.synchronize().await.expect("metadata synchronization");
    assert_eq!(initial.snapshot.total_item_count, 60);
    assert_eq!(initial.snapshot.items.len(), 60);
    let expected_dois: HashSet<_> = records
        .iter()
        .map(|record| record["doi"].as_str().expect("manifest DOI"))
        .collect();
    let actual_dois: HashSet<_> = initial
        .snapshot
        .items
        .iter()
        .map(|item| item.doi.as_deref().expect("imported DOI"))
        .collect();
    assert_eq!(actual_dois, expected_dois);
    let indexed = corpus
        .synchronize()
        .await
        .expect("full-text synchronization");
    assert_eq!(indexed.corpus.full_text_count, 60);
    let mut years = BTreeMap::<i32, u64>::new();
    for item in &indexed.snapshot.items {
        *years
            .entry(item.year.expect("publication year"))
            .or_default() += 1;
        assert_eq!(item.attachments.len(), 1);
        let attachment = &item.attachments[0];
        let record = records
            .iter()
            .find(|record| record["doi"].as_str() == item.doi.as_deref())
            .expect("matching manifest record");
        let text = database
            .document_text(&attachment.document_id)
            .expect("indexed text lookup")
            .expect("indexed PDF text");
        assert!(text
            .content
            .to_lowercase()
            .contains(&item.doi.as_ref().expect("DOI").to_lowercase()));
        let opened = documents
            .resolve(&attachment.document_id)
            .await
            .expect("resolve imported PDF");
        assert_eq!(opened.status, "ready");
        let path = database
            .document_path(&attachment.document_id)
            .expect("validated document path");
        let digest: String = Sha256::digest(fs::read(path).expect("PDF bytes"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(digest, record["sha256"].as_str().expect("PDF checksum"));
    }
    let year_metrics: BTreeMap<_, _> = indexed
        .analytics
        .by_year
        .iter()
        .map(|entry| (entry.year, entry.count))
        .collect();
    assert_eq!(year_metrics, years);
    let retrieved = database
        .search(&search("DynaPicker"))
        .expect("full-text retrieval");
    assert!(retrieved
        .results
        .iter()
        .any(|result| result.item.doi.as_deref() == Some("10.5194/se-15-197-2024")));
    let absent = database
        .search(&search("xylophonemethanequasar"))
        .expect("unsupported retrieval");
    assert_eq!(absent.total, 0);
    sync.synchronize()
        .await
        .expect("repeat metadata synchronization");
    let repeated = corpus
        .synchronize()
        .await
        .expect("repeat text synchronization");
    assert_eq!(repeated.snapshot.total_item_count, 60);
    assert_eq!(repeated.corpus.full_text_count, 60);
    let first_ids: HashSet<_> = initial.snapshot.items.iter().map(|item| &item.id).collect();
    let repeated_ids: HashSet<_> = repeated
        .snapshot
        .items
        .iter()
        .map(|item| &item.id)
        .collect();
    assert_eq!(first_ids, repeated_ids);
    fs::write(
        output.join("workspace.json"),
        serde_json::to_vec_pretty(&repeated).expect("serialize preview"),
    )
    .expect("write preview workspace");
    let evidence = serde_json::json!({
        "manifestId": manifest["id"],
        "papers": 60,
        "pdfsResolvedAndChecksummed": 60,
        "fullTextRecords": 60,
        "duplicateSyncPreservedIds": true,
        "yearCountsMatch": true,
        "fullTextRetrieval": "DynaPicker",
        "unsupportedLexicalQueryReturnedZero": true,
        "generatedAnswerVerified": false,
    });
    fs::write(
        output.join("evidence.json"),
        serde_json::to_vec_pretty(&evidence).expect("serialize evidence"),
    )
    .expect("write evidence");
}

fn search(query: &str) -> SearchRequest {
    SearchRequest {
        query: query.to_owned(),
        mode: Some("keyword".to_owned()),
        limit: Some(60),
        offset: None,
        item_ids: Vec::new(),
        collection_id: None,
        citation_filter: None,
        year_from: None,
        year_to: None,
        item_type: None,
    }
}

#[test]
#[ignore = "downloads public local models and runs the imported real demo"]
fn answers_real_demo_with_local_models_and_resolvable_citations() {
    use std::sync::Arc;

    use crate::agent::model::{
        AgentContext, AgentDetail, AgentProviderKind, AgentRequest, AgentStreamEvent,
    };
    use crate::agent::AgentCoordinator;
    use crate::model::SemanticIndexRequest;
    use crate::semantic::SemanticCoordinator;

    let output = PathBuf::from(
        std::env::var("GRAPHITE_DEMO_OUTPUT")
            .expect("set the already verified disposable demo directory"),
    );
    let evidence: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("evidence.json")).expect("import evidence"))
            .expect("import evidence JSON");
    assert_eq!(evidence["manifestId"], "seismology-ml-v2");
    assert_eq!(evidence["papers"], 60);
    let database = ResearchDatabase::initialize(output.join("demo.sqlite3"))
        .expect("verified public demo database");
    let workspace = database.load_workspace().expect("public demo workspace");
    assert_eq!(workspace.snapshot.total_item_count, 60);
    let paper = workspace
        .snapshot
        .items
        .iter()
        .find(|item| item.doi.as_deref() == Some("10.5194/se-11-1527-2020"))
        .expect("wave simulation paper");
    let semantic = SemanticCoordinator::new(
        database.clone(),
        output.join("semantic-models"),
        output.join("semantic-indexes"),
    )
    .expect("local semantic service");
    println!("Installing the public semantic model in the disposable demo.");
    semantic
        .install_model()
        .expect("public semantic model installation");
    for batch in 1..=3 {
        let indexed = semantic
            .index(&SemanticIndexRequest {
                max_papers: Some(64),
                max_documents: Some(16),
            })
            .expect("index public papers and passages");
        println!(
            "Semantic batch {batch}: {} papers, {} documents",
            indexed.semantic.indexed_papers, indexed.semantic.chunked_documents
        );
        if indexed.semantic.indexed_papers == 60 && indexed.semantic.chunked_documents == 60 {
            break;
        }
    }
    let indexed = semantic.load_workspace().expect("indexed workspace");
    assert_eq!(indexed.semantic.indexed_papers, 60);
    assert_eq!(indexed.semantic.chunked_documents, 60);
    fs::write(
        output.join("workspace-indexed.json"),
        serde_json::to_vec_pretty(&indexed).expect("indexed preview JSON"),
    )
    .expect("indexed preview");
    let agent = AgentCoordinator::new(database.clone(), semantic, output.join("agent-models"))
        .expect("local answer service");
    agent
        .install_model(
            "lightweight",
            Arc::new(|event| {
                if event.kind != "progress" {
                    println!("{}", event.message);
                }
            }),
        )
        .expect("public managed model installation");
    let on_event: Arc<dyn Fn(AgentStreamEvent) + Send + Sync> = Arc::new(|event| {
        if let Some(stage) = event.stage {
            if event.kind != "delta" {
                println!("Agent stage: {stage}");
            }
        }
    });
    let request = AgentRequest {
        question:
            "Which two network architectures are used to simulate seismic waves in this paper?"
                .to_owned(),
        conversation_id: None,
        provider_kind: AgentProviderKind::GraphiteManaged,
        model_profile: "lightweight".to_owned(),
        inference_backend: Default::default(),
        context_window: Default::default(),
        detail: AgentDetail::Concise,
        context: AgentContext {
            selected_only: true,
            current_item_id: Some(paper.id.clone()),
            current_document_id: Some(paper.attachments[0].document_id.clone()),
            selected_item_ids: vec![paper.id.clone()],
            ..Default::default()
        },
        external_provider: None,
    };
    let supported = agent
        .run(request.clone(), on_event.clone())
        .expect("supported local answer");
    fs::write(
        output.join("rag-supported.json"),
        serde_json::to_vec_pretty(&supported).expect("answer JSON"),
    )
    .expect("supported answer evidence");
    assert!(
        !supported.citations.is_empty(),
        "supported answer needs citations"
    );
    let answer = supported.answer_markdown.to_lowercase();
    assert!(answer.contains("wavenet"));
    assert!(answer.contains("autoencoder"));
    for citation in &supported.citations {
        let source = supported
            .sources
            .iter()
            .find(|source| source.source_id == citation.source_id)
            .expect("citation resolves to an evidence source");
        assert_eq!(source.item_id.as_deref(), Some(paper.id.as_str()));
        assert_eq!(
            source.document_id.as_deref(),
            Some(paper.attachments[0].document_id.as_str())
        );
        assert!(source.eligible_for_scientific_claim && !source.stale);
    }
    let mut unsupported_request = request;
    unsupported_request.question =
        "What measured lunar methane concentrations does this paper report?".to_owned();
    let unsupported = agent
        .run(unsupported_request, on_event)
        .expect("unsupported local question");
    fs::write(
        output.join("rag-unsupported.json"),
        serde_json::to_vec_pretty(&unsupported).expect("unsupported answer JSON"),
    )
    .expect("unsupported answer evidence");
    assert!(
        crate::agent::grounding::is_abstention(&unsupported.answer_markdown),
        "unsupported question must abstain: {}",
        unsupported.answer_markdown
    );
    agent.unload_model().expect("unload temporary local model");
}
