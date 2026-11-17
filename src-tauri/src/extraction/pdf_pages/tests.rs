//! Verify local page extraction boundaries and retained library state.

use std::fs;

use crate::model::{LibraryDelta, RawObject, RemoteLibrary, SourceProbe};

use super::*;

struct Fixture {
    directory: tempfile::TempDir,
    database: ResearchDatabase,
    library_id: String,
    document_id: String,
    original: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("temporary preparation library");
        let database = ResearchDatabase::initialize(directory.path().join("library.sqlite3"))
            .expect("database");
        let source = database
            .register_source(&SourceProbe {
                server_id: "preparation-test-source".into(),
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
                    name: "Preparation test library".into(),
                }],
            )
            .expect("library")
            .remove(0);
        let object = |key: &str, data: serde_json::Value| RawObject {
            key: key.into(),
            version: 1,
            object_type: "items".into(),
            json: serde_json::json!({"key":key,"version":1,"data":data}),
        };
        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 1,
                    objects: vec![
                        object(
                            "PAPER",
                            serde_json::json!({"key":"PAPER","version":1,
                    "itemType":"journalArticle","title":"Paper with a local attachment",
                    "creators":[],"tags":[],"collections":[]}),
                        ),
                        object(
                            "MISSING",
                            serde_json::json!({"key":"MISSING","version":1,
                    "itemType":"journalArticle","title":"Paper without an attachment",
                    "creators":[],"tags":[],"collections":[]}),
                        ),
                        object(
                            "PDF",
                            serde_json::json!({"key":"PDF","version":1,
                    "itemType":"attachment","parentItem":"PAPER","title":"PDF",
                    "contentType":"application/pdf","filename":"paper.pdf",
                    "linkMode":"imported_file"}),
                        ),
                    ],
                    deleted_items: vec![],
                    deleted_collections: vec![],
                },
            )
            .expect("test metadata");
        let document_id = database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT id FROM documents WHERE zotero_attachment_key='PDF'",
                [],
                |row| row.get(0),
            )
            .expect("document");
        let original = b"%PDF-1.7\nlocal page preparation boundary input\n%%EOF".to_vec();
        fs::write(directory.path().join("paper.pdf"), &original).expect("test original");
        let fixture = Self {
            directory,
            database,
            library_id: library.id,
            document_id,
            original,
        };
        fixture.register_original("revision-one");
        fixture
    }

    fn register_original(&self, key: &str) {
        let path = self.directory.path().join("paper.pdf");
        let bytes = fs::read(&path).expect("original bytes");
        let modified = path
            .metadata()
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        self.database
            .store_document_locator(
                &self.document_id,
                &path,
                key,
                &hex(&Sha256::digest(&bytes)),
                bytes.len() as u64,
                Some(modified),
            )
            .expect("resolved original");
    }

    fn pages(&self) -> Vec<String> {
        vec![
            "The first physical page contains selectable text.".into(),
            "The second page preserves its original page number.".into(),
        ]
    }

    fn store(&self) -> AppResult<()> {
        store_document_pages(
            &self.database,
            &self.document_id,
            "revision-one",
            self.pages(),
        )
    }
}

#[test]
fn inventory_includes_missing_attachments_and_rejects_other_libraries() {
    let fixture = Fixture::new();
    let inventory = preparation_documents(&fixture.database, &fixture.library_id).unwrap();
    assert_eq!(inventory.len(), 2);
    assert_eq!(
        inventory
            .iter()
            .filter(|row| row.document_id.is_none())
            .count(),
        1
    );
    assert!(inventory.iter().all(|row| !row.has_usable_text));
    assert!(preparation_documents(&fixture.database, "different-library").is_err());
}

#[test]
fn valid_pages_keep_provenance_and_repeated_preparation_preserves_text_and_source() {
    let fixture = Fixture::new();
    fixture.store().expect("store extracted pages");
    let text = fixture
        .database
        .document_text(&fixture.document_id)
        .unwrap()
        .unwrap();
    assert_eq!(text.content, fixture.pages().join("\n\n"));
    assert_eq!(text.indexed_pages, Some(2));
    assert_eq!(text.total_pages, Some(2));
    let connection = fixture.database.connect().unwrap();
    let canonical: String = connection
        .query_row(
            "SELECT e.canonical_json FROM document_texts t
         JOIN document_extractions e ON e.id=t.extraction_id
         WHERE t.document_id=?1 AND t.is_active=1",
            [&fixture.document_id],
            |row| row.get(0),
        )
        .unwrap();
    let document: CanonicalDocumentV1 = serde_json::from_str(&canonical).unwrap();
    assert_eq!(document.pages.len(), 2);
    assert_eq!(
        document.blocks[1].page_range,
        Some(PageRange { start: 2, end: 2 })
    );
    let before: i64 = connection
        .query_row("SELECT COUNT(*) FROM document_revisions", [], |row| {
            row.get(0)
        })
        .unwrap();
    store_document_pages(
        &fixture.database,
        &fixture.document_id,
        "revision-one",
        vec!["Unexpected replacement must not replace retained text.".into()],
    )
    .unwrap();
    assert_eq!(
        fixture
            .database
            .document_text(&fixture.document_id)
            .unwrap()
            .unwrap()
            .content,
        text.content
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM document_revisions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        fs::read(fixture.directory.path().join("paper.pdf")).unwrap(),
        fixture.original
    );
    assert!(
        preparation_documents(&fixture.database, &fixture.library_id)
            .unwrap()
            .iter()
            .find(|row| row.document_id.as_deref() == Some(&fixture.document_id))
            .unwrap()
            .has_usable_text
    );
    assert!(connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM search_content s JOIN documents d ON d.research_item_id=s.item_id WHERE d.id=?1 AND length(s.body)>0)",
            [&fixture.document_id],
            |row| row.get::<_, bool>(0)
        )
        .unwrap());
}

#[test]
fn changed_original_or_revision_cannot_activate_stale_pages() {
    let fixture = Fixture::new();
    fixture.store().unwrap();
    let path = fixture.directory.path().join("paper.pdf");
    fs::write(&path, b"%PDF-1.7\nchanged original attachment bytes\n%%EOF").unwrap();
    assert!(fixture.store().unwrap_err().to_string().contains("changed"));
    fixture.register_original("revision-two");
    assert!(fixture.store().unwrap_err().to_string().contains("changed"));
    assert!(
        !preparation_documents(&fixture.database, &fixture.library_id)
            .unwrap()
            .iter()
            .find(|row| row.document_id.as_deref() == Some(&fixture.document_id))
            .unwrap()
            .has_usable_text
    );
    store_document_pages(
        &fixture.database,
        &fixture.document_id,
        "revision-two",
        fixture.pages(),
    )
    .unwrap();
    assert!(
        preparation_documents(&fixture.database, &fixture.library_id)
            .unwrap()
            .iter()
            .find(|row| row.document_id.as_deref() == Some(&fixture.document_id))
            .unwrap()
            .has_usable_text
    );
}

#[test]
fn foreign_library_and_empty_or_oversized_pages_are_rejected() {
    let fixture = Fixture::new();
    assert!(store_document_pages(
        &fixture.database,
        &fixture.document_id,
        "revision-one",
        vec![" ".into()]
    )
    .unwrap_err()
    .to_string()
    .contains("OCR"));
    assert!(store_document_pages(
        &fixture.database,
        &fixture.document_id,
        "revision-one",
        vec!["x".repeat(MAX_PAGE_BYTES + 1)]
    )
    .is_err());
    fixture
        .database
        .connect()
        .unwrap()
        .execute(
            "UPDATE app_meta SET value='different-library' WHERE key='active_library_id'",
            [],
        )
        .unwrap();
    assert!(fixture
        .store()
        .unwrap_err()
        .to_string()
        .contains("active library changed"));
    assert!(fixture
        .database
        .document_text(&fixture.document_id)
        .unwrap()
        .is_none());
}

#[test]
fn manual_selection_is_preserved_even_when_it_needs_review() {
    let fixture = Fixture::new();
    fixture.store().unwrap();
    let connection = fixture.database.connect().unwrap();
    connection
        .execute(
            "INSERT INTO extraction_routing_profiles(id,version,policy_json,created_at,active)
        VALUES ('manual-extraction-v1','1','{}',CURRENT_TIMESTAMP,0)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE active_extraction_selections SET routing_profile_id='manual-extraction-v1'
        WHERE artifact_id=?1",
            [&fixture.document_id],
        )
        .unwrap();
    let selected: String = connection
        .query_row(
            "SELECT extraction_id FROM active_extraction_selections WHERE artifact_id=?1",
            [&fixture.document_id],
            |row| row.get(0),
        )
        .unwrap();
    fixture.register_original("same-bytes-new-locator");
    store_document_pages(
        &fixture.database,
        &fixture.document_id,
        "same-bytes-new-locator",
        fixture.pages(),
    )
    .expect("same original retains manual selection");
    connection
        .execute(
            "UPDATE artifact_extraction_state SET execution_state='stale' WHERE artifact_id=?1",
            [&fixture.document_id],
        )
        .unwrap();
    assert!(store_document_pages(
        &fixture.database,
        &fixture.document_id,
        "same-bytes-new-locator",
        fixture.pages(),
    )
    .unwrap_err()
    .to_string()
    .contains("selected extraction needs review"));
    assert_eq!(
        connection
            .query_row(
                "SELECT extraction_id FROM active_extraction_selections WHERE artifact_id=?1",
                [&fixture.document_id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        selected
    );
}

#[test]
fn changed_page_boundaries_invalidate_indexed_revision_even_with_identical_text() {
    let fixture = Fixture::new();
    fixture.store().unwrap();
    let connection = fixture.database.connect().unwrap();
    let before: i64 = connection
        .query_row(
            "SELECT source_version FROM document_texts WHERE document_id=?1 AND is_active=1",
            [&fixture.document_id],
            |row| row.get(0),
        )
        .unwrap();
    fs::write(
        fixture.directory.path().join("paper.pdf"),
        b"%PDF-1.7\nchanged physical page boundaries\n%%EOF",
    )
    .unwrap();
    fixture.register_original("revision-two");
    let pages = vec![fixture.pages().join("\n\n")];
    store_document_pages(
        &fixture.database,
        &fixture.document_id,
        "revision-two",
        pages,
    )
    .unwrap();
    let after: i64 = connection
        .query_row(
            "SELECT source_version FROM document_texts WHERE document_id=?1 AND is_active=1",
            [&fixture.document_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(after > before);
    assert_eq!(
        fixture
            .database
            .document_text(&fixture.document_id)
            .unwrap()
            .unwrap()
            .total_pages,
        Some(1)
    );
}

#[tokio::test]
async fn later_zotero_text_sync_keeps_local_pages_and_does_not_rewrap_them() {
    let fixture = Fixture::new();
    fixture.store().unwrap();
    let before = fixture
        .database
        .document_text(&fixture.document_id)
        .unwrap()
        .unwrap();
    let (source_id, _, _) = fixture.database.active_library_context().unwrap().unwrap();
    let mut delta = crate::model::FullTextSyncDelta {
        successful_version: 2,
        records: vec![crate::model::FullTextRecord {
            attachment_key: "PDF".into(),
            version: 2,
            content: "Later flat Zotero text".into(),
            indexed_pages: Some(2),
            total_pages: Some(2),
            indexed_chars: None,
            total_chars: None,
        }],
        removed_attachment_keys: vec![],
        warning_count: 0,
        status: "ready".into(),
        last_error: None,
    };
    fixture
        .database
        .apply_fulltext_delta(&source_id, &fixture.library_id, &delta)
        .unwrap();
    let coordinator = crate::extraction::ExtractionCoordinator::new(fixture.database.clone());
    assert_eq!(
        coordinator
            .canonicalize_attachment_keys(&fixture.library_id, &["PDF".into()])
            .await
            .unwrap(),
        0
    );
    assert_eq!(coordinator.backfill(16).await.unwrap().processed, 0);
    let retained = fixture
        .database
        .document_text(&fixture.document_id)
        .unwrap()
        .unwrap();
    assert_eq!(retained.content, before.content);
    assert_eq!(retained.provider, before.provider);
    assert_eq!(retained.indexed_pages, before.indexed_pages);
    delta.records.clear();
    delta.removed_attachment_keys.push("PDF".into());
    fixture
        .database
        .apply_fulltext_delta(&source_id, &fixture.library_id, &delta)
        .unwrap();
    assert_eq!(
        fixture
            .database
            .document_text(&fixture.document_id)
            .unwrap()
            .unwrap()
            .content,
        before.content
    );
}

#[test]
fn inventory_detects_changed_local_original_but_keeps_offline_text_available() {
    let fixture = Fixture::new();
    fixture.store().unwrap();
    let path = fixture.directory.path().join("paper.pdf");
    fs::write(
        &path,
        b"%PDF-1.7\na changed file with a different byte length\n%%EOF",
    )
    .unwrap();
    assert!(
        !preparation_documents(&fixture.database, &fixture.library_id)
            .unwrap()
            .iter()
            .find(|row| row.document_id.as_deref() == Some(&fixture.document_id))
            .unwrap()
            .has_usable_text
    );
    fs::remove_file(path).unwrap();
    assert!(
        preparation_documents(&fixture.database, &fixture.library_id)
            .unwrap()
            .iter()
            .find(|row| row.document_id.as_deref() == Some(&fixture.document_id))
            .unwrap()
            .has_usable_text
    );
}

#[test]
fn changed_pdf_retires_searchable_text_and_indexes_but_preserves_history() {
    let fixture = Fixture::new();
    fixture.store().unwrap();
    let connection = fixture.database.connect().unwrap();
    connection
        .execute_batch(
            "INSERT INTO embedding_profiles (
           id,provider,model_identifier,model_revision,artifact_identity,
           embedding_dimension,similarity_metric,query_template,
           document_template,normalization,runtime_version,vector_backend,
           vector_index_format_version,active,created_at)
         VALUES ('test','test','test','1','test',3,'cosine','{query}',
           '{document}','l2','1','test','1',1,CURRENT_TIMESTAMP);",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO reference_extractions (
           id,source_id,library_id,source_item_id,document_id,
           document_revision_id,provider,extractor_version,
           source_text_provider,source_text_version,source_text_hash,
           source_completeness,status,is_current,indexed_at)
         SELECT 'prepared-references',d.source_id,d.library_id,
           d.research_item_id,d.id,e.artifact_revision_id,'test','1',
           t.provider,t.source_version,t.content_hash,'full',
           'references-extracted',1,CURRENT_TIMESTAMP
         FROM documents d JOIN document_texts t ON t.document_id=d.id
         JOIN document_extractions e ON e.id=t.extraction_id
         WHERE d.id=?1 AND t.is_active=1",
            [&fixture.document_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO semantic_vector_entries (
           vector_key,embedding_profile_id,library_id,index_kind,target_id,
           active,created_at,updated_at)
         SELECT 1,'test',library_id,'chunk','prepared-passage',1,
           CURRENT_TIMESTAMP,CURRENT_TIMESTAMP FROM documents WHERE id=?1",
            [&fixture.document_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO semantic_chunks (
           id,item_id,library_id,document_id,document_revision_id,
           embedding_profile_id,chunking_profile_id,source_provider,
           character_start,character_end,ordinal,token_count,text,
           text_fingerprint,vector_key,status,created_at,updated_at)
         SELECT 'prepared-passage',d.research_item_id,d.library_id,d.id,
           e.artifact_revision_id,'test','scientific-paragraph-v1',t.provider,
           0,length(t.content),0,12,t.content,t.content_hash,1,'active',
           CURRENT_TIMESTAMP,CURRENT_TIMESTAMP
         FROM documents d JOIN document_texts t ON t.document_id=d.id
         JOIN document_extractions e ON e.id=t.extraction_id
         WHERE d.id=?1 AND t.is_active=1",
            [&fixture.document_id],
        )
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO semantic_document_states (
           document_id,document_revision_id,embedding_profile_id,
           chunking_profile_id,status,chunk_count,updated_at)
         SELECT document_id,document_revision_id,embedding_profile_id,
           chunking_profile_id,'indexed',1,CURRENT_TIMESTAMP
         FROM semantic_chunks;",
        )
        .unwrap();
    let retrieval_counts = || -> (i64, i64, i64, i64) {
        connection
            .query_row(
                "SELECT
             (SELECT COUNT(*) FROM document_texts WHERE is_active=1),
             (SELECT COUNT(*) FROM semantic_vector_entries WHERE active=1),
             (SELECT COUNT(*) FROM semantic_chunks),
             (SELECT COUNT(*) FROM reference_extractions WHERE is_current=1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
    };
    let before = retrieval_counts();
    assert_eq!(before.0, 1);
    assert_eq!(before.1, 1);
    assert_eq!(before.2, 1);
    assert_eq!(before.3, 1);
    fixture.register_original("same-bytes-new-locator");
    assert_eq!(retrieval_counts(), before);
    assert!(has_usable_text(&connection, &fixture.document_id).unwrap());
    fs::write(
        fixture.directory.path().join("paper.pdf"),
        b"%PDF-1.7\nreplacement scanned attachment\n%%EOF",
    )
    .unwrap();
    fixture.register_original("different-bytes");
    assert_eq!(retrieval_counts(), (0, 0, 0, 0));
    assert!(fixture
        .database
        .document_text(&fixture.document_id)
        .unwrap()
        .is_none());
    let retained: (i64, i64, String, String) = connection
        .query_row(
            "SELECT
         (SELECT COUNT(*) FROM document_extractions),
         (SELECT COUNT(*) FROM active_extraction_selections),
         (SELECT execution_state FROM artifact_extraction_state WHERE artifact_id=?1),
         (SELECT body FROM search_content WHERE item_id=(
           SELECT research_item_id FROM documents WHERE id=?1))",
            [&fixture.document_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(retained, (1, 1, "stale".into(), String::new()));
    assert!(store_document_pages(
        &fixture.database,
        &fixture.document_id,
        "different-bytes",
        vec![" ".into()],
    )
    .is_err());
    assert_eq!(retrieval_counts(), (0, 0, 0, 0));
}
