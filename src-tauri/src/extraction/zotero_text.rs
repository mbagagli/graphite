use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::model::{
    CanonicalBlock, CanonicalBlockContent, CanonicalDocumentMetadata, CanonicalDocumentV1,
    ExtractionAttempt, ExtractionInputDescriptor, ExtractionObservations,
    ExtractionProviderDescriptor, ExtractionRequest, ExtractionSignal, ExtractionSupport,
    SignalAvailability, CANONICAL_SCHEMA_VERSION,
};
use super::provider::DocumentExtractionProvider;
use crate::database::content_hash;
use crate::knowledge::source::ProviderFuture;

pub const SOURCE_TEXT_PROFILE_ID: &str = "source-provided-unstructured-text-v1";

#[derive(Clone, Debug, Default)]
pub struct ZoteroTextExtractionProvider;

impl DocumentExtractionProvider for ZoteroTextExtractionProvider {
    fn descriptor(&self) -> ExtractionProviderDescriptor {
        ExtractionProviderDescriptor {
            profile_id: SOURCE_TEXT_PROFILE_ID.to_owned(),
            provider_id: "zotero-indexed-text".to_owned(),
            provider_version: "1.0.0".to_owned(),
            mapping_profile: "canonical-unstructured-text-v1".to_owned(),
            supported_media_types: vec!["application/pdf".to_owned(), "text/plain".to_owned()],
            required_input_forms: vec!["source-provided-text".to_owned()],
            native_text_capability: false,
            layout_capability: false,
            table_capability: false,
            formula_capability: false,
            figure_capability: false,
            ocr_capability: false,
            page_provenance_capability: false,
            bounding_box_capability: false,
            configured_available: true,
        }
    }

    fn supports(&self, input: &ExtractionInputDescriptor) -> ExtractionSupport {
        if !self.descriptor().configured_available {
            return ExtractionSupport::Unavailable;
        }
        if input.has_source_provided_text
            && self
                .descriptor()
                .supported_media_types
                .contains(&input.media_type)
        {
            ExtractionSupport::Supported
        } else {
            ExtractionSupport::Unsupported
        }
    }

    fn extract<'a>(&'a self, request: ExtractionRequest) -> ProviderFuture<'a, ExtractionAttempt> {
        Box::pin(async move { Ok(extract_source_text(request)) })
    }
}

fn extract_source_text(request: ExtractionRequest) -> ExtractionAttempt {
    let descriptor = ZoteroTextExtractionProvider.descriptor();
    let mut observations = ExtractionObservations::unreported(
        descriptor.profile_id.clone(),
        request.artifact_id.clone(),
        request.artifact_revision_id,
        request.input_content_hash.clone(),
    );
    let Some(source_text) = request.source_text else {
        return ExtractionAttempt {
            provider: descriptor,
            canonical_document: None,
            provenance: BTreeMap::new(),
            observations,
            warnings: Vec::new(),
            error: Some("source-provided text is unavailable".to_owned()),
        };
    };
    if source_text.content.is_empty() {
        return ExtractionAttempt {
            provider: descriptor,
            canonical_document: None,
            provenance: BTreeMap::new(),
            observations,
            warnings: Vec::new(),
            error: Some("source-provided text is empty".to_owned()),
        };
    }
    let block_id = stable_id(
        "canonical-block",
        &[
            &request.artifact_id,
            &request.artifact_revision_id.to_string(),
            SOURCE_TEXT_PROFILE_ID,
            CANONICAL_SCHEMA_VERSION,
            "0",
            "other",
        ],
    );
    let document = CanonicalDocumentV1 {
        schema_version: CANONICAL_SCHEMA_VERSION.to_owned(),
        artifact_id: request.artifact_id.clone(),
        artifact_revision_id: request.artifact_revision_id,
        extraction_profile_id: SOURCE_TEXT_PROFILE_ID.to_owned(),
        metadata: CanonicalDocumentMetadata {
            reported_page_count_hint: source_text.total_pages,
            ..CanonicalDocumentMetadata::default()
        },
        pages: Vec::new(),
        blocks: vec![CanonicalBlock {
            id: block_id.clone(),
            parent_id: None,
            source_order: 0,
            section_path: Vec::new(),
            page_range: None,
            regions: Vec::new(),
            content: CanonicalBlockContent::Other {
                label: "unstructured-text".to_owned(),
                text: source_text.content.clone(),
            },
        }],
        reading_order: vec![block_id],
        relationships: Vec::new(),
        assets: Vec::new(),
    };
    observations.reported_page_count = match source_text.total_pages {
        Some(value) => ExtractionSignal::observed(value, SOURCE_TEXT_PROFILE_ID),
        None => ExtractionSignal::unavailable(
            SignalAvailability::NotReported,
            SOURCE_TEXT_PROFILE_ID,
            "the source did not report a total page count",
        ),
    };
    observations.extracted_page_count = ExtractionSignal::unavailable(
        SignalAvailability::Unsupported,
        SOURCE_TEXT_PROFILE_ID,
        "flat source text has no page segmentation",
    );
    observations.pages_with_text = ExtractionSignal::unavailable(
        SignalAvailability::Unsupported,
        SOURCE_TEXT_PROFILE_ID,
        "flat source text has no page segmentation",
    );
    observations.native_text_presence = ExtractionSignal::unavailable(
        SignalAvailability::NotReported,
        SOURCE_TEXT_PROFILE_ID,
        "the source does not identify native text versus OCR",
    );
    observations.ocr_used = ExtractionSignal::unavailable(
        SignalAvailability::NotReported,
        SOURCE_TEXT_PROFILE_ID,
        "the source does not report OCR use",
    );
    observations.ocr_page_indices = ExtractionSignal::unavailable(
        SignalAvailability::Unsupported,
        SOURCE_TEXT_PROFILE_ID,
        "flat source text has no page segmentation",
    );
    observations.layout_warnings = ExtractionSignal::unavailable(
        SignalAvailability::Unsupported,
        SOURCE_TEXT_PROFILE_ID,
        "the provider does not extract layout",
    );
    observations.characters_by_page = ExtractionSignal::unavailable(
        SignalAvailability::Unsupported,
        SOURCE_TEXT_PROFILE_ID,
        "flat source text has no page segmentation",
    );
    observations.tokens_by_page = ExtractionSignal::unavailable(
        SignalAvailability::NotApplicable,
        SOURCE_TEXT_PROFILE_ID,
        "canonicalization does not load a model tokenizer",
    );
    observations.plain_text_character_count = ExtractionSignal::observed(
        source_text.content.chars().count() as u64,
        SOURCE_TEXT_PROFILE_ID,
    );
    observations.plain_text_hash =
        ExtractionSignal::observed(content_hash(&source_text.content), SOURCE_TEXT_PROFILE_ID);
    for count in [
        &mut observations.heading_count,
        &mut observations.table_count,
        &mut observations.formula_count,
        &mut observations.figure_count,
        &mut observations.caption_count,
        &mut observations.reference_count,
    ] {
        *count = ExtractionSignal::unavailable(
            SignalAvailability::Unsupported,
            SOURCE_TEXT_PROFILE_ID,
            "the unstructured adapter does not infer typed structure",
        );
    }
    let mut provenance = BTreeMap::new();
    provenance.insert(
        "sourceTextProvider".to_owned(),
        serde_json::json!(source_text.provider),
    );
    provenance.insert(
        "sourceTextId".to_owned(),
        serde_json::json!(source_text.text_id),
    );
    provenance.insert(
        "sourceVersion".to_owned(),
        serde_json::json!(source_text.source_version),
    );
    provenance.insert(
        "indexedPages".to_owned(),
        serde_json::json!(source_text.indexed_pages),
    );
    provenance.insert(
        "totalPages".to_owned(),
        serde_json::json!(source_text.total_pages),
    );
    provenance.insert(
        "indexedCharacters".to_owned(),
        serde_json::json!(source_text.indexed_characters),
    );
    provenance.insert(
        "totalCharacters".to_owned(),
        serde_json::json!(source_text.total_characters),
    );
    ExtractionAttempt {
        provider: descriptor,
        canonical_document: Some(document),
        provenance,
        observations,
        warnings: vec!["source text has no page, layout, or structure provenance".to_owned()],
        error: None,
    }
}

fn stable_id(kind: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    for part in parts {
        hasher.update([0x1f]);
        hasher.update(part.as_bytes());
    }
    let suffix = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{kind}_{}", &suffix[..20])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::projection::{
        canonical_hash, canonical_json, render_markdown, render_plain_text,
    };

    #[test]
    fn maps_source_text_without_inventing_structure() {
        let content = "α  β\n\n 1.20e-3\r\nfinal line ";
        let attempt = extract_source_text(ExtractionRequest {
            artifact_id: "artifact-1".to_owned(),
            artifact_revision_id: 7,
            input_content_hash: content_hash(content),
            input: ExtractionInputDescriptor {
                media_type: "application/pdf".to_owned(),
                has_local_file: false,
                has_bounded_reader: false,
                has_source_provided_text: true,
            },
            source_text: Some(super::super::model::SourceTextInput {
                text_id: "text-1".to_owned(),
                provider: "fixture-source-text".to_owned(),
                source_version: 4,
                content: content.to_owned(),
                indexed_pages: Some(2),
                total_pages: Some(3),
                indexed_characters: Some(content.len() as u64),
                total_characters: None,
            }),
        });
        let document = attempt.canonical_document.expect("canonical document");
        assert!(document.pages.is_empty());
        assert_eq!(document.blocks.len(), 1);
        assert!(document.blocks[0].parent_id.is_none());
        assert!(document.blocks[0].section_path.is_empty());
        assert!(document.blocks[0].page_range.is_none());
        assert!(document.blocks[0].regions.is_empty());
        assert_eq!(render_plain_text(&document), content);
        assert_eq!(render_markdown(&document), content);
        let first_json = canonical_json(&document).expect("canonical JSON");
        let second_json = canonical_json(&document).expect("canonical JSON");
        assert_eq!(first_json, second_json);
        assert_eq!(
            canonical_hash(&document).expect("canonical hash"),
            content_hash(&first_json)
        );
    }

    #[test]
    fn rejects_empty_source_text_explicitly() {
        let attempt = extract_source_text(ExtractionRequest {
            artifact_id: "artifact-1".to_owned(),
            artifact_revision_id: 7,
            input_content_hash: content_hash(""),
            input: ExtractionInputDescriptor {
                media_type: "application/pdf".to_owned(),
                has_local_file: false,
                has_bounded_reader: false,
                has_source_provided_text: true,
            },
            source_text: Some(super::super::model::SourceTextInput {
                text_id: "text-1".to_owned(),
                provider: "fixture-source-text".to_owned(),
                source_version: 4,
                content: String::new(),
                indexed_pages: None,
                total_pages: None,
                indexed_characters: None,
                total_characters: None,
            }),
        });
        assert!(attempt.canonical_document.is_none());
        assert_eq!(
            attempt.error.as_deref(),
            Some("source-provided text is empty")
        );
    }
}
