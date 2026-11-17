use crate::database::ResearchDatabase;
use crate::error::AppResult;

use super::model::SourceDelta;

#[derive(Clone)]
pub struct KnowledgeStore {
    database: ResearchDatabase,
}

impl KnowledgeStore {
    pub fn new(database: ResearchDatabase) -> Self {
        Self { database }
    }

    pub fn apply_source_delta(
        &self,
        source_id: &str,
        library_id: &str,
        delta: &SourceDelta,
    ) -> AppResult<(u64, u64, u64)> {
        self.database
            .apply_source_delta(source_id, library_id, delta)
    }

    pub fn database(&self) -> &ResearchDatabase {
        &self.database
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::knowledge::model::{
        ArtifactRole, DocumentArtifactRecord, KnowledgeItemRecord, KnowledgeKind, SourceDelta,
        SourceProviderDescriptor,
    };
    use crate::model::{RemoteLibrary, SourceProbe};

    use super::*;

    #[test]
    fn persists_neutral_records_without_reconstructing_legacy_objects() {
        let path = std::env::temp_dir().join(format!(
            "graphite-neutral-store-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let database = ResearchDatabase::initialize(path.clone()).expect("database");
        let source = database
            .register_source(&SourceProbe {
                server_id: "neutral-source".to_owned(),
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
                    name: "Neutral library".to_owned(),
                }],
            )
            .expect("library")
            .remove(0);
        let delta = SourceDelta {
            source_instance_id: format!("source-instance:{}", library.id),
            descriptor: SourceProviderDescriptor {
                provider_kind: "fixture".to_owned(),
                provider_version: "1.0.0".to_owned(),
                display_name: "Neutral fixture".to_owned(),
                read_only: true,
            },
            checkpoint_before: 0,
            checkpoint_after: 1,
            knowledge_items: vec![KnowledgeItemRecord {
                external_id: "ITEM".to_owned(),
                external_kind: "article".to_owned(),
                knowledge_kind: KnowledgeKind::Paper,
                source_revision: 1,
                title: "Neutral authority".to_owned(),
                abstract_text: Some("Persisted from SourceDelta".to_owned()),
                publication_year: Some(2026),
                source_metadata: serde_json::json!({"sourceField": "value"}),
            }],
            source_groups: Vec::new(),
            artifacts: vec![DocumentArtifactRecord {
                external_id: "ARTIFACT".to_owned(),
                parent_external_item_id: Some("ITEM".to_owned()),
                source_revision: 1,
                artifact_role: ArtifactRole::Attachment,
                media_type: "application/pdf".to_owned(),
                title: "Neutral artifact".to_owned(),
                filename: Some("neutral.pdf".to_owned()),
                source_metadata: serde_json::json!({"linkMode": "fixture"}),
            }],
            source_objects: Vec::new(),
            item_tombstones: Vec::new(),
            artifact_tombstones: Vec::new(),
            group_tombstones: Vec::new(),
            snapshot_complete: true,
            warnings: Vec::new(),
        };
        let store = KnowledgeStore::new(database.clone());
        store
            .apply_source_delta(&source.id, &library.id, &delta)
            .expect("neutral delta");
        store
            .apply_source_delta(&source.id, &library.id, &delta)
            .expect("idempotent neutral delta");

        let connection = database.connect().expect("connection");
        let persisted = connection
            .query_row(
                "SELECT i.title, i.knowledge_kind, d.title,
                        COUNT(DISTINCT ib.id), COUNT(DISTINCT ab.id)
                 FROM research_items i
                 JOIN knowledge_item_source_bindings ib
                   ON ib.knowledge_item_id = i.id
                 JOIN documents d ON d.research_item_id = i.id
                 JOIN document_artifact_source_bindings ab
                   ON ab.artifact_id = d.id
                 WHERE ib.external_item_id = 'ITEM'
                   AND ab.external_artifact_id = 'ARTIFACT'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .expect("neutral persistence");
        assert_eq!(
            persisted,
            (
                "Neutral authority".to_owned(),
                "paper".to_owned(),
                "Neutral artifact".to_owned(),
                1,
                1,
            )
        );
        fs::remove_file(path).expect("remove database");
    }
}
