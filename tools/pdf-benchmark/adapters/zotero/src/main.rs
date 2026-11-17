use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use graphite_pdf_benchmark_common::{
    canonical_unstructured, content_hash, read_json, run_adapter, Adapter, BenchmarkDocumentV1,
    BenchmarkExtractionEnvelopeV1, BenchmarkProfile, CandidateDescriptor, ExtractionOutcome,
    ExtractionRequest, MappingLoss, TimingObservation, ZoteroTextSidecar, BENCHMARK_SCHEMA_VERSION,
};

fn main() {
    if let Err(error) = run_adapter(&mut ZoteroAdapter) {
        eprintln!("zotero benchmark adapter: {error}");
        std::process::exit(1);
    }
}

struct ZoteroAdapter;

impl Adapter for ZoteroAdapter {
    fn descriptor(&self) -> CandidateDescriptor {
        CandidateDescriptor {
            candidate_id: "zotero".to_owned(),
            version: "graphite-source-text-v1".to_owned(),
            revision: "source-provided-unstructured-text-v1".to_owned(),
            license: "Graphite compatibility adapter".to_owned(),
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
            features: vec!["source-provided-text".to_owned()],
            supported_profiles: vec![BenchmarkProfile::Native],
            configured_available: true,
            availability_reason: None,
        }
    }

    fn extract(&mut self, request: ExtractionRequest) -> BenchmarkExtractionEnvelopeV1 {
        let started = Instant::now();
        let descriptor = self.descriptor();
        if request.profile != BenchmarkProfile::Native {
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                descriptor,
                ExtractionOutcome::Unsupported,
                "Zotero indexed text has no OCR profile",
            );
        }
        let Some(path) = &request.zotero_baseline_path else {
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                descriptor,
                ExtractionOutcome::Unavailable,
                "the corpus entry has no Zotero full-text sidecar",
            );
        };
        let sidecar = match read_json::<ZoteroTextSidecar>(Path::new(path)) {
            Ok(sidecar) => sidecar,
            Err(error) => {
                return BenchmarkExtractionEnvelopeV1::terminal(
                    &request,
                    descriptor,
                    ExtractionOutcome::Failed,
                    error,
                )
            }
        };
        if !matches!(sidecar.schema_version, 1 | 2) {
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                descriptor,
                ExtractionOutcome::Failed,
                format!(
                    "unsupported Zotero sidecar schema {}",
                    sidecar.schema_version
                ),
            );
        }
        if sidecar
            .document_id
            .as_deref()
            .is_some_and(|document_id| document_id != request.document_id)
        {
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                descriptor,
                ExtractionOutcome::Failed,
                "Zotero sidecar document identity does not match the request",
            );
        }
        let current_hash = content_hash(sidecar.content.as_bytes());
        if sidecar
            .content_sha256
            .as_deref()
            .is_some_and(|expected| expected != current_hash)
        {
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                descriptor,
                ExtractionOutcome::Failed,
                "Zotero sidecar content checksum does not match",
            );
        }
        if sidecar.content.is_empty() {
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                descriptor,
                ExtractionOutcome::EmptySuccess,
                "Zotero full-text sidecar is empty",
            );
        }
        let canonical = canonical_unstructured(
            &request.artifact_id,
            request.artifact_revision_id,
            "zotero-indexed-text:source-provided-unstructured-text-v1",
            &sidecar.content,
            sidecar.total_pages,
        );
        BenchmarkExtractionEnvelopeV1 {
            schema_version: BENCHMARK_SCHEMA_VERSION.to_owned(),
            request_id: request.request_id,
            document_id: request.document_id,
            input_sha256: request.input_sha256,
            candidate: descriptor,
            profile: request.profile,
            outcome: ExtractionOutcome::Succeeded,
            outcome_reason: None,
            searchable_projection: Some(sidecar.content.clone()),
            document: Some(BenchmarkDocumentV1 {
                reported_page_count: sidecar.total_pages,
                ..BenchmarkDocumentV1::default()
            }),
            canonical_document: Some(canonical),
            mapping_losses: vec![
                MappingLoss {
                    capability: "page-provenance".to_owned(),
                    reason: "Zotero indexed text is not segmented by page".to_owned(),
                    source_field: Some("totalPages".to_owned()),
                },
                MappingLoss {
                    capability: "document-structure".to_owned(),
                    reason: "the baseline is intentionally unstructured".to_owned(),
                    source_field: None,
                },
            ],
            warnings: Vec::new(),
            observations: graphite_pdf_benchmark_common::ExtractionObservations {
                reported_page_count: sidecar.total_pages,
                native_text_present: None,
                ocr_used: None,
                provider_quality: None,
                ..Default::default()
            },
            timing: TimingObservation {
                wall_time_ms: started.elapsed().as_millis() as u64,
                ..TimingObservation::default()
            },
            diagnostics: BTreeMap::from([
                (
                    "sourceProvider".to_owned(),
                    serde_json::json!(sidecar.provider),
                ),
                (
                    "sourceVersion".to_owned(),
                    serde_json::json!(sidecar.source_version),
                ),
                (
                    "indexedPages".to_owned(),
                    serde_json::json!(sidecar.indexed_pages),
                ),
                (
                    "indexedCharacters".to_owned(),
                    serde_json::json!(sidecar.indexed_characters),
                ),
            ]),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use graphite_pdf_benchmark_common::{render_plain_text, write_json_atomic};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn baseline_projection_is_byte_faithful() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("sidecar.json");
        let content = "  Unicode mu\n\n3.2e-4\r\n";
        write_json_atomic(
            &path,
            &ZoteroTextSidecar {
                schema_version: 1,
                document_id: None,
                provider: "fixture".to_owned(),
                source_version: 1,
                content: content.to_owned(),
                indexed_pages: Some(1),
                total_pages: Some(1),
                indexed_characters: Some(content.chars().count() as u64),
                total_characters: Some(content.chars().count() as u64),
                content_sha256: None,
            },
        )
        .expect("write sidecar");
        let pdf = directory.path().join("input.pdf");
        fs::write(&pdf, b"%PDF").expect("write input");
        let request = ExtractionRequest {
            request_id: "request".to_owned(),
            document_id: "document".to_owned(),
            artifact_id: "artifact".to_owned(),
            artifact_revision_id: 1,
            input_path: pdf.to_string_lossy().into_owned(),
            input_sha256: "0".repeat(64),
            media_type: "application/pdf".to_owned(),
            profile: BenchmarkProfile::Native,
            forced_ocr_pages: Vec::new(),
            zotero_baseline_path: Some(path.to_string_lossy().into_owned()),
        };
        let result = ZoteroAdapter.extract(request);
        assert_eq!(result.searchable_projection.as_deref(), Some(content));
        assert_eq!(
            render_plain_text(result.canonical_document.as_ref().expect("canonical")),
            content
        );
    }
}
