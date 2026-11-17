use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum KnowledgeKind {
    Paper,
    Note,
    Presentation,
    Spreadsheet,
    Dataset,
    Book,
    Report,
    Other,
}

impl KnowledgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Paper => "paper",
            Self::Note => "note",
            Self::Presentation => "presentation",
            Self::Spreadsheet => "spreadsheet",
            Self::Dataset => "dataset",
            Self::Book => "book",
            Self::Report => "report",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactRole {
    Attachment,
    Supplement,
    Notes,
    Slides,
    Other,
}

impl ArtifactRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Attachment => "attachment",
            Self::Supplement => "supplement",
            Self::Notes => "notes",
            Self::Slides => "slides",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceProviderDescriptor {
    pub provider_kind: String,
    pub provider_version: String,
    pub display_name: String,
    pub read_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceDiscoveryRequest {
    pub source_instance_id: String,
    pub external_source_identity: String,
    pub provider_scope_kind: String,
    pub provider_scope_identity: String,
    pub display_name: String,
    pub checkpoint: i64,
    pub known_item_ids: Vec<String>,
    pub known_group_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeItemRecord {
    pub external_id: String,
    pub external_kind: String,
    pub knowledge_kind: KnowledgeKind,
    pub source_revision: i64,
    pub title: String,
    pub abstract_text: Option<String>,
    pub publication_year: Option<i32>,
    pub source_metadata: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentArtifactRecord {
    pub external_id: String,
    pub parent_external_item_id: Option<String>,
    pub source_revision: i64,
    pub artifact_role: ArtifactRole,
    pub media_type: String,
    pub title: String,
    pub filename: Option<String>,
    pub source_metadata: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceGroupRecord {
    pub external_id: String,
    pub source_revision: i64,
    pub name: String,
    pub parent_external_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceObjectRecord {
    pub object_kind: String,
    pub external_id: String,
    pub source_revision: i64,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceDelta {
    pub source_instance_id: String,
    pub descriptor: SourceProviderDescriptor,
    pub checkpoint_before: i64,
    pub checkpoint_after: i64,
    pub knowledge_items: Vec<KnowledgeItemRecord>,
    pub source_groups: Vec<SourceGroupRecord>,
    pub artifacts: Vec<DocumentArtifactRecord>,
    pub source_objects: Vec<SourceObjectRecord>,
    pub item_tombstones: Vec<String>,
    pub artifact_tombstones: Vec<String>,
    pub group_tombstones: Vec<String>,
    pub snapshot_complete: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceInstance {
    pub id: String,
    pub provider_kind: String,
    pub display_name: String,
    pub external_identity: String,
    pub state: String,
    pub last_successful_cursor: Option<String>,
    pub last_synced_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeItemSourceBinding {
    pub id: String,
    pub knowledge_item_id: Option<String>,
    pub source_instance_id: String,
    pub external_item_id: String,
    pub external_item_kind: String,
    pub source_revision: Option<String>,
    pub normalized_source_metadata: serde_json::Value,
    pub metadata_hash: String,
    pub is_primary: bool,
    pub state: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_domain_records_have_no_provider_specific_required_fields() {
        let item = KnowledgeItemRecord {
            external_id: "external-item".to_owned(),
            external_kind: "article".to_owned(),
            knowledge_kind: KnowledgeKind::Paper,
            source_revision: 1,
            title: "Neutral item".to_owned(),
            abstract_text: None,
            publication_year: Some(2026),
            source_metadata: serde_json::json!({}),
        };
        let artifact = DocumentArtifactRecord {
            external_id: "external-artifact".to_owned(),
            parent_external_item_id: Some("external-item".to_owned()),
            source_revision: 1,
            artifact_role: ArtifactRole::Attachment,
            media_type: "application/pdf".to_owned(),
            title: "Artifact".to_owned(),
            filename: None,
            source_metadata: serde_json::json!({}),
        };
        let serialized = serde_json::to_string(&(item, artifact)).expect("neutral serialization");
        assert!(!serialized.to_ascii_lowercase().contains("zotero"));
    }
}
