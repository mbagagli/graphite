//! Expose existing local inference through an opt-in, stdin-only test bridge.
//!
//! The Python comparison controls this subprocess under network denial.
//! Model registration is read-only and no application research data is opened.

use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use rusqlite::OpenFlags;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::managed::{profile_spec, GraphiteManagedProvider, ManagedRuntime};
use super::model::GenerationRequest;
use super::provider::{EvidenceTokenCounter, LocalModelProvider};
use crate::semantic::{EmbeddingProvider, LocalFastEmbedProvider};

/// Write an identifiable protocol line among the Rust test harness output.
fn reply(value: Value) {
    println!("GRAPHITE_BRIDGE {value}");
    std::io::stdout().flush().expect("flush bridge response");
}

/// Dispatch one bounded local inference request using the production providers.
fn dispatch(
    request: &Value,
    model: &GraphiteManagedProvider,
    embedding: &LocalFastEmbedProvider,
) -> Result<Value, String> {
    match request["op"].as_str().unwrap_or_default() {
        "embed" => {
            let texts: Vec<String> = serde_json::from_value(request["texts"].clone())
                .map_err(|error| error.to_string())?;
            if texts.len() > 32 || texts.iter().any(|text| text.len() > 100_000) {
                return Err("Embedding request exceeds the comparison limit".into());
            }
            let vectors = if request["context"] == "query" {
                texts
                    .iter()
                    .map(|text| embedding.embed_query(text))
                    .collect::<Result<Vec<_>, _>>()
            } else {
                embedding.embed_documents(&texts)
            }
            .map_err(|error| error.to_string())?;
            Ok(json!({"vectors": vectors}))
        }
        "count" => {
            let text = request["text"].as_str().ok_or("Missing text")?;
            model
                .count_tokens(text)
                .map(|tokens| json!({"tokens": tokens}))
                .map_err(|error| error.to_string())
        }
        "generate" => {
            let max_tokens = request["maxTokens"].as_u64().unwrap_or(420);
            if !(1..=2048).contains(&max_tokens) {
                return Err("Generation limit must be between 1 and 2048".into());
            }
            let generation = GenerationRequest {
                system_prompt: request["system"].as_str().unwrap_or_default().into(),
                user_prompt: request["prompt"].as_str().ok_or("Missing prompt")?.into(),
                max_tokens: max_tokens as u32,
                context_tokens: model.context_capacity(),
                temperature: 0.1,
            };
            let cancelled = Arc::new(AtomicBool::new(false));
            let timer_flag = cancelled.clone();
            let (finished, waiting) = mpsc::channel();
            let timer = std::thread::spawn(move || {
                if waiting.recv_timeout(Duration::from_secs(240)).is_err() {
                    timer_flag.store(true, Ordering::SeqCst);
                }
            });
            let result = model.generate(&generation, cancelled.clone(), &mut |_| {});
            let _ = finished.send(());
            timer.join().expect("join generation watchdog");
            result
                .map(|result| {
                    json!({"text": result.text, "generatedTokens": result.generated_tokens,
                        "generationMs": result.generation_ms,
                        "timedOut": cancelled.load(Ordering::SeqCst)})
                })
                .map_err(|error| error.to_string())
        }
        _ => Err("Unsupported bridge operation".into()),
    }
}

/// Serve pinned model callbacks without opening a port or installing models.
#[test]
#[ignore = "requires installed local models and an offline stdin client"]
fn upstream_local_model_bridge() {
    let app = PathBuf::from(std::env::var("GRAPHITE_COMPARISON_APP_ROOT").expect("model registry"));
    let cache =
        PathBuf::from(std::env::var("GRAPHITE_COMPARISON_EMBEDDINGS").expect("embedding cache"));
    let connection = rusqlite::Connection::open_with_flags(
        app.join("graphite-research.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("read-only model registry");
    let (relative, registered_hash): (String, String) = connection
        .query_row(
            "SELECT cache_relative_path,sha256 FROM agent_model_installations
             WHERE profile_id='balanced' AND status='ready'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("already installed balanced model");
    drop(connection);
    let model_root = app.join("agent-models").canonicalize().expect("model root");
    let model_path = model_root
        .join(relative)
        .canonicalize()
        .expect("model file");
    assert!(model_path.starts_with(&model_root));
    let profile = profile_spec("balanced").unwrap();
    assert_eq!(registered_hash, profile.sha256);
    let mut input = std::fs::File::open(&model_path).expect("read model");
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer).expect("hash model");
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual_hash: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(actual_hash, profile.sha256);
    let runtime = ManagedRuntime::initialize().expect("local CPU runtime");
    assert!(
        !runtime
            .hardware_status()
            .expect("runtime devices")
            .available,
        "CPU-only comparison required"
    );
    let model = GraphiteManagedProvider::new(runtime.clone(), "balanced", model_path).unwrap();
    let embedding = LocalFastEmbedProvider::new(cache).expect("existing embedding runtime");
    let metadata = embedding
        .installed_metadata()
        .unwrap()
        .expect("embedding already installed");
    reply(
        json!({"ready": true, "modelSha256": profile.sha256, "embedding": {
            "modelIdentifier": metadata.model_identifier,
            "modelRevision": metadata.model_revision,
            "artifactIdentity": metadata.artifact_identity,
            "dimension": metadata.dimension, "runtimeVersion": metadata.runtime_version,
            "queryTemplate": metadata.query_template,
            "documentTemplate": metadata.document_template,
            "normalization": metadata.normalization},
        "temperature": 0.1, "contextCapacity": profile.context_capacity}),
    );
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("read bridge command");
        assert!(line.len() <= 2_000_000, "Oversized bridge command");
        let request: Value = serde_json::from_str(&line).expect("JSON bridge request");
        if request["op"] == "quit" {
            break;
        }
        match dispatch(&request, &model, &embedding) {
            Ok(value) => reply(json!({"ok": true, "result": value})),
            Err(error) => reply(json!({"ok": false, "error": error})),
        }
    }
    runtime.unload().expect("unload comparison model");
}
