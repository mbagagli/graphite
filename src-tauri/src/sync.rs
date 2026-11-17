use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::database::{ResearchDatabase, StoredLibrary, SyncRunCompletion};
use crate::error::{AppError, AppResult};
use crate::knowledge::model::SourceDiscoveryRequest;
use crate::knowledge::source::SourceProvider;
use crate::knowledge::store::KnowledgeStore;
use crate::knowledge::zotero::ZoteroSourceProvider;
use crate::model::{SourceProbe, WorkspacePayload};
#[cfg(test)]
use crate::zotero::ZoteroReadClient;

const MAX_CONCURRENT_CHANGE_ATTEMPTS: usize = 3;

#[derive(Clone)]
pub struct SyncCoordinator {
    database: ResearchDatabase,
    provider: ZoteroSourceProvider,
    store: KnowledgeStore,
    guard: Arc<Mutex<()>>,
}

impl SyncCoordinator {
    pub fn new(database: ResearchDatabase, guard: Arc<Mutex<()>>) -> AppResult<Self> {
        let provider = ZoteroSourceProvider::new()?;
        let store = KnowledgeStore::new(database.clone());
        Ok(Self {
            database,
            provider,
            store,
            guard,
        })
    }

    #[cfg(test)]
    pub fn with_client(database: ResearchDatabase, client: ZoteroReadClient) -> Self {
        let store = KnowledgeStore::new(database.clone());
        Self {
            database,
            provider: ZoteroSourceProvider::with_client(client),
            store,
            guard: Arc::new(Mutex::new(())),
        }
    }

    pub async fn synchronize(&self) -> AppResult<WorkspacePayload> {
        let _single_flight = self.guard.lock().await;
        self.require_zotero_partition()?;
        let result = self.synchronize_inner().await;
        if let Err(error) = &result {
            let _ = self.database.record_connection_error(&error.to_string());
        }
        result
    }

    pub async fn probe(&self) -> AppResult<SourceProbe> {
        let _single_flight = self.guard.lock().await;
        self.require_zotero_partition()?;
        self.provider.probe_source().await
    }

    pub async fn rebuild(&self) -> AppResult<WorkspacePayload> {
        let _single_flight = self.guard.lock().await;
        self.require_zotero_partition()?;
        self.database.clear_active_partition()?;
        let result = self.synchronize_inner().await;
        if let Err(error) = &result {
            let _ = self.database.record_connection_error(&error.to_string());
        }
        result
    }

    fn require_zotero_partition(&self) -> AppResult<()> {
        if crate::demo::catalog::is_active(&self.database)? {
            return Err(AppError::ApplicationData(
                "Switch to Zotero before connecting or synchronizing it.".into(),
            ));
        }
        Ok(())
    }

    async fn synchronize_inner(&self) -> AppResult<WorkspacePayload> {
        let probe = self.provider.probe_source().await?;
        let source = self.database.register_source(&probe)?;
        let remote_libraries = self.provider.discover_libraries(&probe).await?;
        let libraries = self
            .database
            .register_libraries(&source.id, &remote_libraries)?;
        let run_id = self.database.begin_sync(&source.id)?;
        let mut changed = 0_u64;
        let mut deleted = 0_u64;
        let mut warnings = 0_u64;
        let mut scanned = 0_u32;

        for library in &libraries {
            match self
                .sync_library(&probe.server_id, &source.id, library)
                .await
            {
                Ok((library_changed, library_deleted, library_warnings)) => {
                    changed += library_changed;
                    deleted += library_deleted;
                    warnings += library_warnings;
                    scanned += 1;
                }
                Err(error) => {
                    let message = error.to_string();
                    self.database.finish_sync(
                        run_id,
                        SyncRunCompletion {
                            status: "failed",
                            libraries_scanned: scanned,
                            objects_changed: changed,
                            objects_deleted: deleted,
                            normalization_warnings: warnings,
                            error: Some(&message),
                        },
                    )?;
                    return Err(error);
                }
            }
        }
        self.database.finish_sync(
            run_id,
            SyncRunCompletion {
                status: "complete",
                libraries_scanned: scanned,
                objects_changed: changed,
                objects_deleted: deleted,
                normalization_warnings: warnings,
                error: None,
            },
        )?;
        let mut workspace = self.database.load_workspace()?;
        workspace.source.source_changed = source.changed;
        workspace.source.connection_status = "online".to_owned();
        Ok(workspace)
    }

    async fn sync_library(
        &self,
        server_id: &str,
        source_id: &str,
        library: &StoredLibrary,
    ) -> AppResult<(u64, u64, u64)> {
        for attempt in 0..MAX_CONCURRENT_CHANGE_ATTEMPTS {
            let known_item_ids = self
                .database
                .object_keys(&library.id, "items")?
                .into_iter()
                .collect();
            let known_group_ids = self
                .database
                .object_keys(&library.id, "collections")?
                .into_iter()
                .collect();
            let request = SourceDiscoveryRequest {
                source_instance_id: format!("source-instance:{}", library.id),
                external_source_identity: server_id.to_owned(),
                provider_scope_kind: library.remote.kind.clone(),
                provider_scope_identity: library.remote.zotero_id.clone(),
                display_name: library.remote.name.clone(),
                checkpoint: library.version,
                known_item_ids,
                known_group_ids,
            };
            match self.provider.discover(request).await {
                Ok(delta) => {
                    return self
                        .store
                        .apply_source_delta(source_id, &library.id, &delta);
                }
                Err(AppError::ConcurrentChange) if attempt + 1 < MAX_CONCURRENT_CHANGE_ATTEMPTS => {
                    let delay_ms = 180_u64 * (attempt as u64 + 1);
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                Err(error) => return Err(error),
            }
        }
        Err(AppError::ConcurrentChange)
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
    use crate::zotero::ZoteroReadClient;

    use super::SyncCoordinator;

    struct MockServer {
        root: String,
        requests: Arc<Mutex<Vec<String>>>,
        thread: thread::JoinHandle<()>,
    }

    fn start_mock_server(expected_requests: usize) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock listener");
        let address = listener.local_addr().expect("mock address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let server_thread = thread::spawn(move || {
            for request_index in 0..expected_requests {
                let (mut stream, _) = listener.accept().expect("mock request");
                let mut buffer = [0_u8; 16_384];
                let read = stream.read(&mut buffer).expect("read mock request");
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                captured.lock().expect("request log").push(request.clone());
                let first_line = request.lines().next().unwrap_or_default();
                let body = mock_body(first_line, request_index);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Zotero-API-Version: 3\r\nZotero-Server-ID: mock-server\r\n\
                     Zotero-Schema-Version: 37\r\nLast-Modified-Version: {}\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    if request_index >= 6 { 6 } else { 5 },
                    body.len(),
                    body,
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write mock response");
            }
        });
        MockServer {
            root: format!("http://{address}/api"),
            requests,
            thread: server_thread,
        }
    }

    fn mock_body(request_line: &str, request_index: usize) -> String {
        if request_line == "GET /api/ HTTP/1.1" {
            return "Zotero is running".to_owned();
        }
        if request_line.contains("/groups") {
            return "[]".to_owned();
        }
        if request_line.contains("/items?") && request_line.contains("format=versions") {
            return if request_index >= 6 {
                r#"{"ITEM2":6}"#.to_owned()
            } else {
                r#"{"ITEM1":5,"ITEM2":5}"#.to_owned()
            };
        }
        if request_line.contains("/collections?") && request_line.contains("format=versions") {
            return r#"{"COLL1":5}"#.to_owned();
        }
        if request_line.contains("itemKey=") {
            let version = if request_line.contains("ITEM1%2CITEM2")
                || request_line.contains("ITEM1%2cITEM2")
                || request_line.contains("ITEM1,ITEM2")
            {
                5
            } else {
                6
            };
            if version == 5 {
                return format!(
                    "[{},{}]",
                    mock_item("ITEM1", "Initial paper", 5),
                    mock_item("ITEM2", "Second paper", 5),
                );
            }
            return format!("[{}]", mock_item("ITEM2", "Updated paper", 6));
        }
        if request_line.contains("collectionKey=") {
            return r#"[{"key":"COLL1","version":5,"data":{"key":"COLL1","version":5,"name":"Field studies","parentCollection":false}}]"#.to_owned();
        }
        panic!("unexpected mock request: {request_line}");
    }

    fn mock_item(key: &str, title: &str, version: i64) -> String {
        serde_json::json!({
            "key": key,
            "version": version,
            "data": {
                "key": key,
                "version": version,
                "itemType": "journalArticle",
                "title": title,
                "date": "2025",
                "creators": [{
                    "creatorType": "author",
                    "firstName": "Mara",
                    "lastName": "Chen"
                }],
                "tags": [{"tag": "surface waves"}],
                "collections": ["COLL1"]
            }
        })
        .to_string()
    }

    #[tokio::test]
    async fn synchronizes_incrementally_and_reconciles_deletions_with_get_only_requests() {
        let path = std::env::temp_dir().join(format!(
            "graphite-http-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let database = ResearchDatabase::initialize(path.clone()).expect("database");
        let fresh_workspace = database
            .load_workspace()
            .expect("fresh workspace before first Zotero sync");
        assert!(fresh_workspace.snapshot.items.is_empty());
        assert!(fresh_workspace.snapshot.active_library_id.is_none());

        let mock = start_mock_server(11);
        let client = ZoteroReadClient::with_root(mock.root.clone()).expect("client");
        let coordinator = SyncCoordinator::with_client(database.clone(), client);

        let initial = coordinator.synchronize().await.expect("initial sync");
        assert_eq!(initial.snapshot.items.len(), 2);
        assert_eq!(initial.snapshot.collections.len(), 1);
        assert!(initial.snapshot.active_library_id.is_some());
        let connection = database.connect().expect("neutral authority connection");
        let neutral_counts = connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM knowledge_item_source_bindings
                     WHERE state = 'active'),
                    (SELECT COUNT(*) FROM source_instances
                     WHERE provider_kind = 'zotero' AND state = 'active'),
                    (SELECT COUNT(*) FROM collections
                     WHERE name = 'Field studies')",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .expect("neutral authority rows");
        let item_two_identity = connection
            .query_row(
                "SELECT item.id, binding.id
                 FROM research_items item
                 JOIN knowledge_item_source_bindings binding
                   ON binding.knowledge_item_id = item.id
                 WHERE binding.external_item_id = 'ITEM2'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .expect("neutral item identity");
        assert_eq!(neutral_counts, (2, 1, 1));
        assert_ne!(
            item_two_identity.1,
            format!("knowledge-binding:{}", item_two_identity.0)
        );
        drop(connection);
        let incremental = coordinator.synchronize().await.expect("incremental sync");
        assert_eq!(incremental.snapshot.items.len(), 1);
        assert_eq!(incremental.snapshot.items[0].title, "Updated paper");
        let connection = database.connect().expect("incremental identity connection");
        let item_two_after: String = connection
            .query_row(
                "SELECT knowledge_item_id
                 FROM knowledge_item_source_bindings
                 WHERE external_item_id = 'ITEM2' AND state = 'active'",
                [],
                |row| row.get(0),
            )
            .expect("stable neutral identity");
        let removed_binding = connection
            .query_row(
                "SELECT state, knowledge_item_id IS NULL
                 FROM knowledge_item_source_bindings
                 WHERE external_item_id = 'ITEM1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .expect("tombstoned neutral binding");
        assert_eq!(item_two_after, item_two_identity.0);
        assert_eq!(removed_binding, ("deleted".to_owned(), true));

        mock.thread.join().expect("mock server thread");
        let requests = mock.requests.lock().expect("request log");
        assert_eq!(requests.len(), 11);
        assert!(requests.iter().all(|request| request.starts_with("GET ")));
        assert!(requests.iter().all(|request| {
            request.starts_with("GET /api/ HTTP/1.1")
                || request
                    .to_ascii_lowercase()
                    .contains("zotero-server-id: mock-server")
        }));
        assert!(requests.iter().all(|request| !request.contains("/deleted")));
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.contains("format=versions"))
                .count(),
            4
        );
        fs::remove_file(path).expect("remove temporary database");
    }
}
