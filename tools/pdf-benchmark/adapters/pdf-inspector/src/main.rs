use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use graphite_pdf_benchmark_common::canonical::canonical_from_benchmark;
use graphite_pdf_benchmark_common::render_plain_text;
use graphite_pdf_benchmark_common::{
    run_adapter, Adapter, BenchmarkDocumentV1, BenchmarkElement, BenchmarkElementKind,
    BenchmarkExtractionEnvelopeV1, BenchmarkPage, BenchmarkProfile, CandidateDescriptor,
    ExtractionObservations, ExtractionOutcome, ExtractionRequest, MappingLoss,
    OcrPageProvenanceObservation, TimingObservation, BENCHMARK_SCHEMA_VERSION,
};
use pdf_inspector::{extract_pages_markdown, process_pdf, PdfType};

fn main() {
    if let Err(error) = run_adapter(&mut PdfInspectorAdapter) {
        eprintln!("pdf-inspector benchmark adapter: {error}");
        std::process::exit(1);
    }
}

struct PdfInspectorAdapter;

impl Adapter for PdfInspectorAdapter {
    fn descriptor(&self) -> CandidateDescriptor {
        let profiles = if cfg!(feature = "ocr") {
            vec![
                BenchmarkProfile::Native,
                BenchmarkProfile::RecommendedAuto,
                BenchmarkProfile::ForcedOcr,
            ]
        } else {
            vec![BenchmarkProfile::Native]
        };
        CandidateDescriptor {
            candidate_id: "pdf-inspector".to_owned(),
            version: "1.15.0".to_owned(),
            revision: "ca6d66714683f71d1b1cd447f766a2b287bea751".to_owned(),
            license: "MIT".to_owned(),
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
            features: if cfg!(feature = "ocr") {
                vec!["native".to_owned(), "ocr".to_owned()]
            } else {
                vec!["native".to_owned()]
            },
            supported_profiles: profiles,
            configured_available: true,
            availability_reason: None,
        }
    }

    fn extract(&mut self, request: ExtractionRequest) -> BenchmarkExtractionEnvelopeV1 {
        match request.profile {
            BenchmarkProfile::Native => native_extract(self.descriptor(), request),
            BenchmarkProfile::RecommendedAuto | BenchmarkProfile::ForcedOcr => {
                #[cfg(feature = "ocr")]
                {
                    ocr_extract(self.descriptor(), request)
                }
                #[cfg(not(feature = "ocr"))]
                {
                    BenchmarkExtractionEnvelopeV1::terminal(
                        &request,
                        self.descriptor(),
                        ExtractionOutcome::Unavailable,
                        "adapter was built without the isolated ocr feature",
                    )
                }
            }
        }
    }
}

fn native_extract(
    descriptor: CandidateDescriptor,
    request: ExtractionRequest,
) -> BenchmarkExtractionEnvelopeV1 {
    let started = Instant::now();
    let path = Path::new(&request.input_path);
    let process = match process_pdf(path) {
        Ok(value) => value,
        Err(error) => {
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                descriptor,
                ExtractionOutcome::Failed,
                format!("pdf-inspector native extraction failed: {error}"),
            )
        }
    };
    let pages = match extract_pages_markdown(path, None) {
        Ok(value) => value,
        Err(error) => {
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                descriptor,
                ExtractionOutcome::Failed,
                format!("pdf-inspector page extraction failed: {error}"),
            )
        }
    };
    let benchmark_pages = pages
        .pages
        .iter()
        .map(|page| BenchmarkPage {
            page_number: page.page + 1,
            label: None,
            width: None,
            height: None,
            rotation_degrees: None,
            text: page.markdown.clone(),
            regions: Vec::new(),
        })
        .collect::<Vec<_>>();
    let elements = benchmark_pages
        .iter()
        .enumerate()
        .filter(|(_, page)| !page.text.is_empty())
        .map(|(index, page)| BenchmarkElement {
            id: format!("page-{}-text", page.page_number),
            kind: BenchmarkElementKind::Other,
            text: page.text.clone(),
            source_order: index as u32,
            page_number: Some(page.page_number),
            parent_id: None,
            heading_level: None,
            region: None,
            table: None,
            formula_latex: None,
            figure_asset_id: None,
            list_ordered: None,
        })
        .collect::<Vec<_>>();
    let provider_projection = benchmark_pages
        .iter()
        .map(|page| page.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let document = BenchmarkDocumentV1 {
        title: process.title,
        language: None,
        reported_page_count: Some(process.page_count),
        pages: benchmark_pages,
        reading_order: elements.iter().map(|element| element.id.clone()).collect(),
        elements,
        relationships: Vec::new(),
    };
    let canonical = canonical_from_benchmark(
        &request.artifact_id,
        request.artifact_revision_id,
        "pdf-inspector:1.15.0:native",
        &document,
        &provider_projection,
    );
    let projection = render_plain_text(&canonical);
    let empty_pages = document
        .pages
        .iter()
        .filter(|page| page.text.trim().is_empty())
        .map(|page| page.page_number)
        .collect::<Vec<_>>();
    let suspicious_pages = pages.pages_needing_ocr.clone();
    let outcome = classify_output(&projection, &suspicious_pages);
    BenchmarkExtractionEnvelopeV1 {
        schema_version: BENCHMARK_SCHEMA_VERSION.to_owned(),
        request_id: request.request_id,
        document_id: request.document_id,
        input_sha256: request.input_sha256,
        candidate: descriptor,
        profile: request.profile,
        outcome,
        outcome_reason: None,
        searchable_projection: Some(projection),
        document: Some(document),
        canonical_document: Some(canonical),
        mapping_losses: vec![
            MappingLoss {
                capability: "typed-structure".to_owned(),
                reason:
                    "the provider API exposes per-page Markdown rather than typed semantic blocks; the adapter does not infer structure"
                        .to_owned(),
                source_field: Some("PageMarkdown.markdown".to_owned()),
            },
            MappingLoss {
                capability: "bounding-boxes".to_owned(),
                reason: "per-page Markdown does not expose region geometry".to_owned(),
                source_field: Some("PageMarkdown".to_owned()),
            },
        ],
        warnings: if process.has_encoding_issues {
            vec!["pdf-inspector reported broken text encoding".to_owned()]
        } else {
            Vec::new()
        },
        observations: ExtractionObservations {
            reported_page_count: Some(process.page_count),
            extracted_page_count: Some(pages.pages.len() as u32),
            pages_with_text: pages
                .pages
                .iter()
                .filter(|page| !page.markdown.trim().is_empty())
                .map(|page| page.page + 1)
                .collect(),
            empty_pages,
            suspicious_pages: suspicious_pages.clone(),
            native_text_present: Some(matches!(
                process.pdf_type,
                PdfType::TextBased | PdfType::Mixed
            )),
            ocr_used: Some(false),
            ocr_page_indices_reported: Some(true),
            ocr_pages: Vec::new(),
            ocr_recommended_pages: suspicious_pages,
            encoding_warnings: if process.has_encoding_issues {
                vec!["has_encoding_issues".to_owned()]
            } else {
                Vec::new()
            },
            layout_warnings: if pages.is_complex {
                vec!["complex layout detected".to_owned()]
            } else {
                Vec::new()
            },
            characters_by_page: pages
                .pages
                .iter()
                .map(|page| page.markdown.chars().count() as u64)
                .collect(),
            provider_text_character_count: Some(provider_projection.chars().count() as u64),
            structure_retention_warnings: Vec::new(),
            provider_quality: Some(format!(
                "classification-confidence:{:.3}",
                process.confidence
            )),
            ..ExtractionObservations::default()
        },
        timing: TimingObservation {
            wall_time_ms: started.elapsed().as_millis() as u64,
            provider_time_ms: Some(process.processing_time_ms),
            ..TimingObservation::default()
        },
        diagnostics: BTreeMap::from([
            (
                "pdfType".to_owned(),
                serde_json::json!(format!("{:?}", process.pdf_type)),
            ),
            (
                "pagesWithTables".to_owned(),
                serde_json::json!(pages.pages_with_tables),
            ),
            (
                "pagesWithColumns".to_owned(),
                serde_json::json!(pages.pages_with_columns),
            ),
        ]),
    }
}

#[cfg(feature = "ocr")]
fn ocr_extract(
    descriptor: CandidateDescriptor,
    request: ExtractionRequest,
) -> BenchmarkExtractionEnvelopeV1 {
    use pdf_inspector::vision::{
        process_pdf_with_ocr, ModelDownloadPolicy, OcrMode, OcrPdfOptions,
    };

    let started = Instant::now();
    let mut options = match request.profile {
        BenchmarkProfile::RecommendedAuto => OcrPdfOptions::auto(),
        BenchmarkProfile::ForcedOcr => OcrPdfOptions::new().mode(OcrMode::Force),
        BenchmarkProfile::Native => OcrPdfOptions::new(),
    };
    let mut ocr_options = options
        .ocr
        .clone()
        .model_downloads(ModelDownloadPolicy::Offline);
    if let Ok(root) = std::env::var("GRAPHITE_PDF_BENCH_ASSET_ROOT") {
        let candidate_root = Path::new(&root).join("pdf-inspector");
        ocr_options = ocr_options.model_directory(&candidate_root);
        let pdfium = candidate_root
            .join("pdfium")
            .join(platform_slug())
            .join(pdfium_library_relative_path());
        if pdfium.is_file() {
            std::env::set_var("PDFIUM_LIB_PATH", pdfium);
        }
        let onnxruntime = candidate_root
            .join("onnxruntime")
            .join(platform_slug())
            .join(onnxruntime_library_relative_path());
        if onnxruntime.is_file() {
            std::env::set_var("ORT_DYLIB_PATH", onnxruntime);
        }
    }
    options = options.ocr(ocr_options);
    let result = match process_pdf_with_ocr(&request.input_path, options) {
        Ok(value) => value,
        Err(error) => {
            let reason = error.to_string();
            let unavailable = reason.contains("model")
                || reason.contains("PDFium")
                || reason.contains("runtime")
                || reason.contains("offline");
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                descriptor,
                if unavailable {
                    ExtractionOutcome::Unavailable
                } else {
                    ExtractionOutcome::Failed
                },
                format!("pdf-inspector OCR extraction failed: {reason}"),
            );
        }
    };
    let pages = result
        .pages
        .iter()
        .map(|page| BenchmarkPage {
            page_number: page.page_number,
            label: None,
            width: None,
            height: None,
            rotation_degrees: None,
            text: page.markdown.clone(),
            regions: Vec::new(),
        })
        .collect::<Vec<_>>();
    let elements = pages
        .iter()
        .enumerate()
        .filter(|(_, page)| !page.text.is_empty())
        .map(|(index, page)| BenchmarkElement {
            id: format!("page-{}-text", page.page_number),
            kind: BenchmarkElementKind::Other,
            text: page.text.clone(),
            source_order: index as u32,
            page_number: Some(page.page_number),
            parent_id: None,
            heading_level: None,
            region: None,
            table: None,
            formula_latex: None,
            figure_asset_id: None,
            list_ordered: None,
        })
        .collect::<Vec<_>>();
    let document = BenchmarkDocumentV1 {
        title: None,
        language: None,
        reported_page_count: Some(result.page_count),
        pages,
        reading_order: elements.iter().map(|element| element.id.clone()).collect(),
        elements,
        relationships: Vec::new(),
    };
    let canonical = canonical_from_benchmark(
        &request.artifact_id,
        request.artifact_revision_id,
        &format!("pdf-inspector:1.15.0:{}", request.profile.as_str()),
        &document,
        &result.markdown,
    );
    let projection = render_plain_text(&canonical);
    let suspicious = result.pages_recommending_hosted.clone();
    let ocr_page_provenance = result
        .pages
        .iter()
        .filter(|page| page.provenance.ocr_model.is_some())
        .map(|page| OcrPageProvenanceObservation {
            page_number: page.provenance.page_number,
            source: format!("{:?}", page.provenance.source).to_lowercase(),
            model: page
                .provenance
                .ocr_model
                .as_ref()
                .map(|model| model.name.clone()),
            model_revision: page
                .provenance
                .ocr_model
                .as_ref()
                .map(|model| model.revision.clone()),
            render_dpi: page.provenance.render_dpi.map(f64::from),
            confidence: page.provenance.ocr_confidence.map(f64::from),
            warnings: page.provenance.warnings.clone(),
        })
        .collect::<Vec<_>>();
    BenchmarkExtractionEnvelopeV1 {
        schema_version: BENCHMARK_SCHEMA_VERSION.to_owned(),
        request_id: request.request_id,
        document_id: request.document_id,
        input_sha256: request.input_sha256,
        candidate: descriptor,
        profile: request.profile,
        outcome: classify_output(&projection, &suspicious),
        outcome_reason: None,
        searchable_projection: Some(projection),
        document: Some(document),
        canonical_document: Some(canonical),
        mapping_losses: vec![MappingLoss {
            capability: "typed-structure".to_owned(),
            reason: "OCR Markdown is preserved without inventing semantic block types".to_owned(),
            source_field: Some("OcrPdfResult.pages".to_owned()),
        }],
        warnings: suspicious
            .iter()
            .map(|page| format!("page {page} recommends a heavier hosted fallback"))
            .collect(),
        observations: ExtractionObservations {
            reported_page_count: Some(result.page_count),
            extracted_page_count: Some(result.pages.len() as u32),
            pages_with_text: result
                .pages
                .iter()
                .filter(|page| !page.markdown.trim().is_empty())
                .map(|page| page.page_number)
                .collect(),
            empty_pages: result
                .pages
                .iter()
                .filter(|page| page.markdown.trim().is_empty())
                .map(|page| page.page_number)
                .collect(),
            suspicious_pages: suspicious,
            native_text_present: None,
            ocr_used: Some(!result.pages_routed_to_ocr.is_empty()),
            ocr_page_indices_reported: Some(true),
            ocr_pages: result.pages_routed_to_ocr.clone(),
            ocr_recommended_pages: result.pages_recommended_for_ocr.clone(),
            encoding_warnings: Vec::new(),
            layout_warnings: if result.is_complex {
                vec!["complex layout detected".to_owned()]
            } else {
                Vec::new()
            },
            characters_by_page: result
                .pages
                .iter()
                .map(|page| page.markdown.chars().count() as u64)
                .collect(),
            provider_text_character_count: Some(result.markdown.chars().count() as u64),
            structure_retention_warnings: Vec::new(),
            ocr_page_provenance,
            provider_quality: None,
        },
        timing: TimingObservation {
            wall_time_ms: started.elapsed().as_millis() as u64,
            provider_time_ms: Some(result.processing_time_ms),
            render_time_ms: Some(result.render_time_ms),
            ocr_time_ms: Some(result.ocr_time_ms),
        },
        diagnostics: BTreeMap::from([
            (
                "providerMarkdownCharacters".to_owned(),
                serde_json::json!(result.markdown.chars().count()),
            ),
            (
                "pagesWithTables".to_owned(),
                serde_json::json!(result.pages_with_tables),
            ),
            (
                "pagesWithColumns".to_owned(),
                serde_json::json!(result.pages_with_columns),
            ),
        ]),
    }
}

#[cfg(feature = "ocr")]
fn platform_slug() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "mac-x64",
        ("macos", "aarch64") => "mac-arm64",
        ("windows", "x86_64") => "win-x64",
        _ => "unsupported-platform",
    }
}

#[cfg(all(feature = "ocr", target_os = "windows"))]
fn pdfium_library_relative_path() -> &'static str {
    "bin/pdfium.dll"
}

#[cfg(all(feature = "ocr", target_os = "macos"))]
fn pdfium_library_relative_path() -> &'static str {
    "lib/libpdfium.dylib"
}

#[cfg(all(feature = "ocr", not(any(target_os = "windows", target_os = "macos"))))]
fn pdfium_library_relative_path() -> &'static str {
    "lib/libpdfium.so"
}

#[cfg(all(feature = "ocr", target_os = "windows"))]
fn onnxruntime_library_relative_path() -> &'static str {
    "onnxruntime-win-x64-1.28.0/lib/onnxruntime.dll"
}

#[cfg(all(feature = "ocr", target_os = "macos"))]
fn onnxruntime_library_relative_path() -> &'static str {
    "onnxruntime-osx-arm64-1.28.0/lib/libonnxruntime.dylib"
}

#[cfg(all(feature = "ocr", target_arch = "aarch64", target_os = "linux"))]
fn onnxruntime_library_relative_path() -> &'static str {
    "onnxruntime-linux-aarch64-1.28.0/lib/libonnxruntime.so"
}

#[cfg(all(feature = "ocr", target_arch = "x86_64", target_os = "linux"))]
fn onnxruntime_library_relative_path() -> &'static str {
    "onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so"
}

#[cfg(all(
    feature = "ocr",
    not(any(target_os = "windows", target_os = "macos", target_os = "linux"))
))]
fn onnxruntime_library_relative_path() -> &'static str {
    "unsupported/libonnxruntime"
}

fn classify_output(text: &str, suspicious_pages: &[u32]) -> ExtractionOutcome {
    if text.is_empty() {
        ExtractionOutcome::EmptySuccess
    } else if suspicious_pages.is_empty() {
        ExtractionOutcome::Succeeded
    } else {
        ExtractionOutcome::Suspicious
    }
}
