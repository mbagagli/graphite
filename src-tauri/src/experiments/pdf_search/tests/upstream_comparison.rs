//! Observe production retrieval on the approved upstream comparison queries.
//!
//! This opt-in probe uses disposable storage and installed local embeddings.
//! It never reads the answer key and does not generate or score answers.

use std::collections::HashMap;
use std::time::Instant;

use crate::agent::grounding::{prepare_run, retrieve_run};
use crate::agent::model::{AgentContext, AgentDetail, AgentProviderKind, AgentRequest};
use crate::agent::provider::ConservativeTokenEstimator;

use super::*;

mod generated_answers;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProbeQuery {
    id: String,
    selected_document_ids: Vec<String>,
    question: String,
}

#[derive(Deserialize)]
struct ProbeQueries {
    questions: Vec<ProbeQuery>,
}

/// Run the real scoped retriever without loading an answer model.
#[test]
#[ignore = "requires local corpus and isolated installed embedding assets"]
fn approved_upstream_queries_production_retrieval() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let corpus_root = root.join("tools/pdf-benchmark/corpus");
    let bundle = root.join("evaluation/upstream-rag-v1");
    let runtime = root.join(".cache/rag-comparison");
    let output = runtime.join("runs/graphite-selected-only-probe.json");
    assert!(!output.exists(), "preserve previous probe output");
    let queries: ProbeQueries =
        serde_json::from_slice(&std::fs::read(bundle.join("queries.json")).expect("query inputs"))
            .expect("query schema");
    let mut manifest: RepresentativeManifest = serde_json::from_slice(
        &std::fs::read(corpus_root.join("representative-manifest.json")).expect("source manifest"),
    )
    .expect("source manifest schema");
    let primary = ["paper11", "paper16", "paper20", "paper23", "paper30"];
    manifest
        .documents
        .retain(|doc| primary.contains(&doc.id.as_str()));
    assert_eq!(manifest.documents.len(), primary.len());
    let fixture = seed_representative_database(&corpus_root, &manifest);
    let ids: HashMap<_, _> = manifest
        .documents
        .iter()
        .zip(&fixture.item_ids)
        .map(|(doc, id)| (doc.id.clone(), id.clone()))
        .collect();
    let provider: Arc<dyn EmbeddingProvider> = Arc::new(
        LocalFastEmbedProvider::new(runtime.join("graphite-semantic-models"))
            .expect("isolated installed embedding provider"),
    );
    let metadata = provider
        .installed_metadata()
        .expect("model metadata")
        .expect("model must already be installed");
    let semantic = SemanticCoordinator::with_components(
        fixture.database.clone(),
        provider,
        Arc::new(
            UsearchVectorIndex::new(fixture.root.join("vectors")).expect("disposable vector index"),
        ),
    );
    let started = Instant::now();
    semantic
        .install_model()
        .expect("activate installed embedding assets");
    let mut indexing = Vec::new();
    for _ in 0..3 {
        indexing.push(
            semantic
                .index(&SemanticIndexRequest {
                    max_papers: Some(64),
                    max_documents: Some(16),
                })
                .expect("index the five available Zotero baselines"),
        );
    }
    let indexing_ms = started.elapsed().as_millis();
    let before = file_sha256(&fixture.database_path);
    let mut rows = Vec::new();
    for query in &queries.questions {
        if query
            .selected_document_ids
            .iter()
            .any(|id| !ids.contains_key(id))
        {
            rows.push(serde_json::json!({
                "queryId": query.id, "status": "unavailable",
                "reason": "paper05 has no Zotero text baseline"
            }));
            continue;
        }
        let selected: Vec<_> = query
            .selected_document_ids
            .iter()
            .map(|id| ids[id].clone())
            .collect();
        let request = AgentRequest {
            question: query.question.clone(),
            conversation_id: None,
            provider_kind: AgentProviderKind::GraphiteManaged,
            model_profile: "balanced".to_owned(),
            inference_backend: Default::default(),
            context_window: Default::default(),
            detail: AgentDetail::Concise,
            context: AgentContext {
                selected_only: true,
                selected_item_ids: selected.clone(),
                current_item_id: (selected.len() == 1).then(|| selected[0].clone()),
                ..AgentContext::default()
            },
            external_provider: None,
        };
        let observed = retrieve_run(&fixture.database, &semantic, &request).and_then(|retrieved| {
            prepare_run(retrieved, &request, 16_384, &ConservativeTokenEstimator)
        });
        match observed {
            Ok(prepared) => {
                let leaks = prepared
                    .evidence
                    .iter()
                    .filter(|evidence| {
                        evidence
                            .item_id
                            .as_ref()
                            .is_none_or(|id| !selected.contains(id))
                    })
                    .count();
                rows.push(serde_json::json!({
                    "queryId": query.id, "status": "succeeded",
                    "contextScopeLeaks": leaks,
                    "selectionReindexCount": 0,
                    "scope": prepared.scope,
                    "evidence": prepared.evidence,
                    "retrievalTrace": prepared.retrieval_trace,
                    "contextStats": prepared.context_stats,
                    "limitations": prepared.limitations
                }));
            }
            Err(error) => rows.push(serde_json::json!({
                "queryId": query.id, "status": "failed",
                "reason": error.to_string()
            })),
        }
    }
    let unchanged = before == file_sha256(&fixture.database_path);
    let report = serde_json::json!({
        "evaluationId": "upstream-rag-v1",
        "phase": "retrieval-only-probe",
        "scopeMode": "selected-only",
        "answerQualityScored": false,
        "tokenAccounting": "estimated-characters-3.0-v1",
        "limitations": [
            "No answer generation or scientific correctness score",
            "Five primary texts indexed; paper05 unavailable",
            "Fixture metadata uses paper identifiers, not full Zotero metadata",
            "Conservative context token estimate, not Qwen tokenizer"
        ],
        "itemIds": ids, "embedding": {
            "provider": metadata.provider,
            "modelIdentifier": metadata.model_identifier,
            "modelRevision": metadata.model_revision,
            "artifactIdentity": metadata.artifact_identity
        },
        "indexingMs": indexing_ms, "indexing": indexing,
        "databaseUnchangedByQueries": unchanged, "results": rows
    });
    std::fs::write(
        &output,
        serde_json::to_vec_pretty(&report).expect("report JSON"),
    )
    .expect("write local probe report");
    assert!(unchanged, "queries must not reindex or mutate the database");
    drop(semantic);
    fixture.cleanup();
    assert!(
        rows.iter()
            .all(|row| row["contextScopeLeaks"].as_u64().unwrap_or(0) == 0),
        "selected-paper context boundary; inspect the saved probe report"
    );
}
