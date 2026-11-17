use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::database::ResearchDatabase;
use crate::error::{AppError, AppResult};
use crate::extraction::{ExtractionCoordinator, DEFAULT_CANONICAL_BACKFILL_BATCH};
use crate::model::{FullTextSyncDelta, WorkspacePayload};
use crate::zotero::ZoteroReadClient;

#[derive(Clone)]
pub struct CorpusCoordinator {
    database: ResearchDatabase,
    client: ZoteroReadClient,
    extraction: ExtractionCoordinator,
    guard: Arc<Mutex<()>>,
}

impl CorpusCoordinator {
    pub fn new(database: ResearchDatabase, guard: Arc<Mutex<()>>) -> AppResult<Self> {
        let extraction = ExtractionCoordinator::new(database.clone());
        Ok(Self {
            database,
            client: ZoteroReadClient::new()?,
            extraction,
            guard,
        })
    }

    #[cfg(test)]
    pub fn with_client(database: ResearchDatabase, client: ZoteroReadClient) -> Self {
        let extraction = ExtractionCoordinator::new(database.clone());
        Self {
            database,
            client,
            extraction,
            guard: Arc::new(Mutex::new(())),
        }
    }

    pub async fn synchronize(&self) -> AppResult<WorkspacePayload> {
        let _single_flight = self.guard.lock().await;
        let Some((source_id, server_id, library)) = self.database.active_library_context()? else {
            return self.database.load_workspace();
        };
        if source_id == crate::demo::catalog::SOURCE_ID {
            return self.database.load_workspace();
        }
        let cursor = self.database.fulltext_cursor(&library.id)?;
        let changed_inventory = self
            .client
            .fulltext_inventory(&library.remote, cursor, &server_id)
            .await?;
        let complete_inventory = if cursor == 0 {
            changed_inventory.clone()
        } else {
            self.client
                .fulltext_inventory(&library.remote, 0, &server_id)
                .await?
        };
        let remote_keys: HashSet<_> = complete_inventory
            .versions
            .iter()
            .map(|(key, _)| key.clone())
            .collect();
        let local_keys = self.database.fulltext_keys(&library.id)?;
        let mut removed_attachment_keys: Vec<_> =
            local_keys.difference(&remote_keys).cloned().collect();
        removed_attachment_keys.sort();

        let mut records = Vec::new();
        let mut warning_count = 0_u64;
        let mut last_error = None;
        for (attachment_key, version) in &changed_inventory.versions {
            match self
                .client
                .fulltext_record(&library.remote, attachment_key, *version, &server_id)
                .await
            {
                Ok(record) => records.push(record),
                Err(AppError::ZoteroResourceUnavailable(_)) => {
                    warning_count += 1;
                    last_error = Some(format!(
                        "Zotero listed text for attachment {attachment_key}, but the text record was unavailable"
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        let discovered_version = changed_inventory
            .library_version
            .or(complete_inventory.library_version)
            .unwrap_or(cursor);
        let successful_version = if warning_count == 0 {
            discovered_version.max(cursor)
        } else {
            cursor
        };
        let status = if warning_count == 0 {
            "ready"
        } else {
            "partial"
        };
        let changed_attachment_keys = records
            .iter()
            .map(|record| record.attachment_key.clone())
            .collect::<Vec<_>>();
        self.database.apply_fulltext_delta(
            &source_id,
            &library.id,
            &FullTextSyncDelta {
                successful_version,
                records,
                removed_attachment_keys,
                warning_count,
                status: status.to_owned(),
                last_error,
            },
        )?;
        self.extraction
            .canonicalize_attachment_keys(&library.id, &changed_attachment_keys)
            .await?;
        self.extraction
            .backfill(DEFAULT_CANONICAL_BACKFILL_BATCH)
            .await?;
        self.database.load_workspace()
    }

    pub async fn rebuild(&self) -> AppResult<WorkspacePayload> {
        let _single_flight = self.guard.lock().await;
        self.database.rebuild_document_index()?;
        self.database.load_workspace()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use crate::database::ResearchDatabase;
    use crate::model::{
        FullTextRecord, FullTextSyncDelta, LibraryDelta, RawObject, RemoteLibrary, SourceProbe,
    };
    use crate::zotero::ZoteroReadClient;

    use super::CorpusCoordinator;

    #[tokio::test]
    async fn synchronizes_zotero_indexed_text_with_get_only_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let server = thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().expect("request");
                let mut bytes = [0_u8; 8_192];
                let length = stream.read(&mut bytes).expect("read");
                let request = String::from_utf8_lossy(&bytes[..length]).to_string();
                captured.lock().expect("log").push(request);
                let body = if index == 0 {
                    r#"{"PDF1":4}"#
                } else {
                    r#"{"content":"distinctive receiver function text","indexedPages":8,"totalPages":8}"#
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Zotero-API-Version: 3\r\nZotero-Server-ID: corpus-source\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                stream.write_all(response.as_bytes()).expect("response");
            }
        });
        let path = std::env::temp_dir().join(format!(
            "graphite-corpus-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let database = ResearchDatabase::initialize(path.clone()).expect("database");
        let source = database
            .register_source(&SourceProbe {
                server_id: "corpus-source".to_owned(),
                api_version: "3".to_owned(),
                schema_version: None,
            })
            .expect("source");
        let library = database
            .register_libraries(
                &source.id,
                &[RemoteLibrary {
                    kind: "user".to_owned(),
                    zotero_id: "0".to_owned(),
                    name: "My Library".to_owned(),
                }],
            )
            .expect("library")
            .remove(0);
        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 1,
                    objects: vec![research_item(), attachment()],
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("metadata");
        let client = ZoteroReadClient::with_root(format!("http://{address}/api")).expect("client");
        let coordinator = CorpusCoordinator::with_client(database, client);
        let workspace = coordinator.synchronize().await.expect("corpus sync");
        assert_eq!(workspace.corpus.full_text_count, 1);
        assert_eq!(workspace.corpus.successful_version, 4);
        server.join().expect("server");
        let requests = requests.lock().expect("request log");
        assert!(requests.iter().all(|request| request.starts_with("GET ")));
        assert!(requests
            .iter()
            .any(|request| request.contains("/fulltext?since=0")));
        assert!(requests.iter().all(|request| !request.starts_with("POST ")));
        fs::remove_file(path).expect("remove database");
    }

    #[tokio::test]
    async fn handles_no_changes_then_an_incremental_fulltext_update() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            let bodies = [
                r#"{}"#,
                r#"{"PDF1":4}"#,
                r#"{"PDF1":6}"#,
                r#"{"PDF1":6}"#,
                r#"{"content":"updated ellipticity text","indexedPages":9,"totalPages":9}"#,
            ];
            for body in bodies {
                let (mut stream, _) = listener.accept().expect("request");
                let mut bytes = [0_u8; 8_192];
                let _ = stream.read(&mut bytes).expect("read");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Zotero-API-Version: 3\r\nZotero-Server-ID: corpus-source\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                stream.write_all(response.as_bytes()).expect("response");
            }
        });
        let (database, path, source_id, library_id) = prepared_database("incremental");
        seed_fulltext(&database, &source_id, &library_id, 4, "initial text");
        let client = ZoteroReadClient::with_root(format!("http://{address}/api")).expect("client");
        let coordinator = CorpusCoordinator::with_client(database.clone(), client);
        let unchanged = coordinator.synchronize().await.expect("unchanged sync");
        assert_eq!(unchanged.corpus.changed_documents, 0);
        assert_eq!(unchanged.corpus.successful_version, 4);
        let updated = coordinator.synchronize().await.expect("incremental sync");
        assert_eq!(updated.corpus.changed_documents, 1);
        assert_eq!(updated.corpus.successful_version, 6);
        let document_id = updated.snapshot.items[0].attachments[0].document_id.clone();
        assert!(database
            .document_text(&document_id)
            .expect("document text")
            .expect("indexed text")
            .content
            .contains("ellipticity"));
        server.join().expect("server");
        fs::remove_file(path).expect("remove database");
    }

    #[tokio::test]
    async fn leaves_the_fulltext_cursor_unchanged_after_a_listed_record_404() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = thread::spawn(move || {
            for index in 0..3 {
                let (mut stream, _) = listener.accept().expect("request");
                let mut bytes = [0_u8; 8_192];
                let _ = stream.read(&mut bytes).expect("read");
                if index == 2 {
                    stream
                        .write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .expect("404 response");
                    continue;
                }
                let body = r#"{"PDF1":7}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Zotero-API-Version: 3\r\nZotero-Server-ID: corpus-source\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                stream.write_all(response.as_bytes()).expect("response");
            }
        });
        let (database, path, source_id, library_id) = prepared_database("record-404");
        seed_fulltext(&database, &source_id, &library_id, 4, "stable text");
        let client = ZoteroReadClient::with_root(format!("http://{address}/api")).expect("client");
        let coordinator = CorpusCoordinator::with_client(database, client);
        let workspace = coordinator.synchronize().await.expect("partial sync");
        assert_eq!(workspace.corpus.status, "partial");
        assert_eq!(workspace.corpus.warning_count, 1);
        assert_eq!(workspace.corpus.successful_version, 4);
        server.join().expect("server");
        fs::remove_file(path).expect("remove database");
    }

    fn prepared_database(name: &str) -> (ResearchDatabase, std::path::PathBuf, String, String) {
        let path = std::env::temp_dir().join(format!(
            "graphite-corpus-{name}-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let database = ResearchDatabase::initialize(path.clone()).expect("database");
        let source = database
            .register_source(&SourceProbe {
                server_id: "corpus-source".to_owned(),
                api_version: "3".to_owned(),
                schema_version: None,
            })
            .expect("source");
        let library = database
            .register_libraries(
                &source.id,
                &[RemoteLibrary {
                    kind: "user".to_owned(),
                    zotero_id: "0".to_owned(),
                    name: "My Library".to_owned(),
                }],
            )
            .expect("library")
            .remove(0);
        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 1,
                    objects: vec![research_item(), attachment()],
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("metadata");
        (database, path, source.id, library.id)
    }

    fn seed_fulltext(
        database: &ResearchDatabase,
        source_id: &str,
        library_id: &str,
        version: i64,
        content: &str,
    ) {
        database
            .apply_fulltext_delta(
                source_id,
                library_id,
                &FullTextSyncDelta {
                    successful_version: version,
                    records: vec![FullTextRecord {
                        attachment_key: "PDF1".to_owned(),
                        version,
                        content: content.to_owned(),
                        indexed_pages: Some(8),
                        total_pages: Some(8),
                        indexed_chars: None,
                        total_chars: None,
                    }],
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("seed full text");
    }

    fn research_item() -> RawObject {
        raw_item(
            "PAPER1",
            serde_json::json!({
                "key": "PAPER1",
                "version": 1,
                "itemType": "journalArticle",
                "title": "Receiver functions",
                "creators": [],
                "tags": [],
                "collections": []
            }),
        )
    }

    fn attachment() -> RawObject {
        raw_item(
            "PDF1",
            serde_json::json!({
                "key": "PDF1",
                "version": 1,
                "itemType": "attachment",
                "parentItem": "PAPER1",
                "title": "PDF",
                "contentType": "application/pdf",
                "filename": "paper.pdf",
                "linkMode": "imported_file"
            }),
        )
    }

    fn raw_item(key: &str, data: serde_json::Value) -> RawObject {
        RawObject {
            key: key.to_owned(),
            version: 1,
            object_type: "items".to_owned(),
            json: serde_json::json!({"key": key, "version": 1, "data": data}),
        }
    }
}
