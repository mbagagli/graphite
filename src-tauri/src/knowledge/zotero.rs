use std::collections::HashSet;

use crate::error::{AppError, AppResult};
use crate::model::{RemoteLibrary, SourceProbe};
use crate::normalize::{normalize_collection, normalize_item};
use crate::zotero::ZoteroReadClient;

use super::model::{
    ArtifactRole, DocumentArtifactRecord, KnowledgeItemRecord, KnowledgeKind, SourceDelta,
    SourceDiscoveryRequest, SourceGroupRecord, SourceObjectRecord, SourceProviderDescriptor,
};
use super::source::{ProviderFuture, SourceProvider};

#[derive(Clone)]
pub struct ZoteroSourceProvider {
    client: ZoteroReadClient,
}

impl ZoteroSourceProvider {
    pub fn new() -> AppResult<Self> {
        Ok(Self {
            client: ZoteroReadClient::new()?,
        })
    }

    #[cfg(test)]
    pub fn with_client(client: ZoteroReadClient) -> Self {
        Self { client }
    }

    pub async fn probe_source(&self) -> AppResult<SourceProbe> {
        self.client.probe().await
    }

    pub async fn discover_libraries(&self, probe: &SourceProbe) -> AppResult<Vec<RemoteLibrary>> {
        self.client.discover_libraries(probe).await
    }

    async fn discover_delta(&self, request: SourceDiscoveryRequest) -> AppResult<SourceDelta> {
        let source_instance_id = request.source_instance_id.clone();
        let remote = RemoteLibrary {
            kind: request.provider_scope_kind,
            zotero_id: request.provider_scope_identity,
            name: request.display_name,
        };
        let item_inventory = self
            .client
            .item_inventory(&remote, &request.external_source_identity)
            .await?;
        let group_inventory = self
            .client
            .collection_inventory(&remote, &request.external_source_identity)
            .await?;
        if item_inventory.library_version != group_inventory.library_version {
            return Err(AppError::ConcurrentChange);
        }
        let checkpoint_after = item_inventory.library_version;
        let known_items: HashSet<_> = request.known_item_ids.iter().cloned().collect();
        let known_groups: HashSet<_> = request.known_group_ids.iter().cloned().collect();
        let remote_items: HashSet<_> = item_inventory
            .versions
            .iter()
            .map(|(key, _)| key.clone())
            .collect();
        let remote_groups: HashSet<_> = group_inventory
            .versions
            .iter()
            .map(|(key, _)| key.clone())
            .collect();
        let item_keys = item_inventory
            .versions
            .into_iter()
            .filter(|(key, version)| *version > request.checkpoint || !known_items.contains(key))
            .map(|(key, _)| key)
            .collect::<Vec<_>>();
        let group_keys = group_inventory
            .versions
            .into_iter()
            .filter(|(key, version)| *version > request.checkpoint || !known_groups.contains(key))
            .map(|(key, _)| key)
            .collect::<Vec<_>>();
        let mut raw_objects = self
            .client
            .fetch_items(
                &remote,
                &item_keys,
                &request.external_source_identity,
                checkpoint_after,
            )
            .await?;
        raw_objects.extend(
            self.client
                .fetch_collections(
                    &remote,
                    &group_keys,
                    &request.external_source_identity,
                    checkpoint_after,
                )
                .await?,
        );

        let mut knowledge_items = Vec::new();
        let mut artifacts = Vec::new();
        let mut source_groups = Vec::new();
        let mut warnings = Vec::new();
        for object in &raw_objects {
            match object.object_type.as_str() {
                "items" => match normalize_item(object) {
                    Ok(item) if item.is_top_level_research_item() => {
                        knowledge_items.push(KnowledgeItemRecord {
                            external_id: item.key,
                            external_kind: item.item_type.clone(),
                            knowledge_kind: knowledge_kind(&item.item_type),
                            source_revision: item.version,
                            title: item.title,
                            abstract_text: item.abstract_text,
                            publication_year: item.year,
                            source_metadata: source_item_metadata(&item.item_type),
                        });
                    }
                    Ok(item) if item.item_type == "attachment" => {
                        artifacts.push(DocumentArtifactRecord {
                            external_id: item.key,
                            parent_external_item_id: item.parent_key,
                            source_revision: item.version,
                            artifact_role: ArtifactRole::Attachment,
                            media_type: item
                                .content_type
                                .unwrap_or_else(|| "application/octet-stream".to_owned()),
                            title: item.title,
                            filename: item.filename,
                            source_metadata: serde_json::json!({
                                "linkMode": item.link_mode,
                            }),
                        });
                    }
                    Ok(_) => {}
                    Err(error) => warnings.push(error.to_string()),
                },
                "collections" => match normalize_collection(object) {
                    Ok(group) => source_groups.push(SourceGroupRecord {
                        external_id: group.key,
                        source_revision: group.version,
                        name: group.name,
                        parent_external_id: group.parent_key,
                    }),
                    Err(error) => warnings.push(error.to_string()),
                },
                _ => {}
            }
        }
        let source_objects = raw_objects
            .into_iter()
            .map(|object| SourceObjectRecord {
                object_kind: object.object_type,
                external_id: object.key,
                source_revision: object.version,
                payload: object.json,
            })
            .collect();
        let mut item_tombstones = known_items
            .difference(&remote_items)
            .cloned()
            .collect::<Vec<_>>();
        item_tombstones.sort();
        let mut group_tombstones = known_groups
            .difference(&remote_groups)
            .cloned()
            .collect::<Vec<_>>();
        group_tombstones.sort();
        Ok(SourceDelta {
            source_instance_id,
            descriptor: self.descriptor(),
            checkpoint_before: request.checkpoint,
            checkpoint_after,
            knowledge_items,
            source_groups,
            artifacts,
            source_objects,
            item_tombstones,
            artifact_tombstones: Vec::new(),
            group_tombstones,
            snapshot_complete: true,
            warnings,
        })
    }
}

impl SourceProvider for ZoteroSourceProvider {
    fn descriptor(&self) -> SourceProviderDescriptor {
        SourceProviderDescriptor {
            provider_kind: "zotero".to_owned(),
            provider_version: "local-api-v3".to_owned(),
            display_name: "Zotero Local API".to_owned(),
            read_only: true,
        }
    }

    fn discover<'a>(&'a self, request: SourceDiscoveryRequest) -> ProviderFuture<'a, SourceDelta> {
        Box::pin(async move { self.discover_delta(request).await })
    }
}

fn knowledge_kind(item_type: &str) -> KnowledgeKind {
    match item_type {
        "journalArticle" | "conferencePaper" | "preprint" => KnowledgeKind::Paper,
        "book" | "bookSection" => KnowledgeKind::Book,
        "report" | "thesis" | "manuscript" => KnowledgeKind::Report,
        "presentation" => KnowledgeKind::Presentation,
        "dataset" => KnowledgeKind::Dataset,
        "note" => KnowledgeKind::Note,
        _ => KnowledgeKind::Other,
    }
}

fn source_item_metadata(item_type: &str) -> serde_json::Value {
    serde_json::json!({"externalKind": item_type})
}
