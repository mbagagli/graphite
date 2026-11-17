use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use graphite_lib::extraction::model::{
    CanonicalBlockContent, CanonicalDocumentV1, ExtractionAttempt, ExtractionObservations,
    ExtractionProviderDescriptor, ExtractionSignal, SignalAvailability,
};
use graphite_lib::extraction::projection::render_plain_text;
use graphite_lib::extraction::quality::ExtractionQualityEvaluator;
use serde::{Deserialize, Serialize};

use graphite_pdf_benchmark_quality_oracle::{json_files, read_json, write_json};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkCandidate {
    candidate_id: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkObservations {
    reported_page_count: Option<u32>,
    extracted_page_count: Option<u32>,
    pages_with_text: Vec<u32>,
    empty_pages: Vec<u32>,
    suspicious_pages: Vec<u32>,
    native_text_present: Option<bool>,
    ocr_used: Option<bool>,
    ocr_page_indices_reported: Option<bool>,
    ocr_pages: Vec<u32>,
    encoding_warnings: Vec<String>,
    layout_warnings: Vec<String>,
    characters_by_page: Vec<u64>,
    provider_text_character_count: Option<u64>,
    #[serde(default)]
    structure_retention_warnings: Vec<String>,
    #[serde(default)]
    ocr_page_provenance: Vec<BenchmarkOcrProvenance>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkOcrProvenance {
    confidence: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkEnvelope {
    document_id: String,
    input_sha256: String,
    candidate: BenchmarkCandidate,
    profile: String,
    outcome: String,
    canonical_document: Option<CanonicalDocumentV1>,
    warnings: Vec<String>,
    observations: BenchmarkObservations,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScoreAggregate {
    reports: Vec<ScoreRow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScoreRow {
    document_id: String,
    candidate_id: String,
    profile: String,
    required_anchor_recall: f64,
    required_projection_recall: f64,
    required_page_qualified_recall: Option<f64>,
    required_structural_recall: Option<f64>,
    unicode_scalar_count: u64,
    page_character_anomaly_pages: Vec<u32>,
    replacement_character_count: u64,
    truncation_suspected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QualityAggregate {
    schema_version: &'static str,
    quality_profile_id: String,
    reports: Vec<QualityRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QualityRow {
    document_id: String,
    candidate_id: String,
    profile: String,
    status: String,
    legacy_status: String,
    usable: bool,
    reasons: Vec<String>,
    required_anchor_recall: f64,
    required_projection_recall: f64,
    required_page_qualified_recall: Option<f64>,
    required_structural_recall: Option<f64>,
    classification: String,
    legacy_classification: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("quality-oracle: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() != 3 {
        return Err(
            "usage: graphite-pdf-benchmark-quality-oracle <run-dir> <score-aggregate> <output>"
                .to_owned(),
        );
    }
    let run_directory = PathBuf::from(&arguments[0]);
    let score_aggregate: ScoreAggregate = read_json(Path::new(&arguments[1]))?;
    let output = PathBuf::from(&arguments[2]);
    let evaluator = ExtractionQualityEvaluator::default_v1();
    let quality_profile_id = evaluator.profile().id.clone();
    let recalls = score_aggregate
        .reports
        .into_iter()
        .map(|report| {
            (
                (
                    report.document_id.clone(),
                    report.candidate_id.clone(),
                    report.profile.clone(),
                ),
                report,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut reports = Vec::new();
    for path in json_files(&run_directory)? {
        let envelope: BenchmarkEnvelope = read_json(&path)?;
        let key = (
            envelope.document_id.clone(),
            envelope.candidate.candidate_id.clone(),
            envelope.profile.clone(),
        );
        let score = recalls
            .get(&key)
            .ok_or_else(|| format!("score aggregate has no row for {key:?}"))?;
        let attempt = production_attempt(&envelope, score);
        let legacy_report = evaluator.evaluate(&legacy_observable_attempt(&attempt));
        let report = evaluator.evaluate(&attempt);
        let status = report.status.as_str().to_owned();
        let legacy_status = legacy_report.status.as_str().to_owned();
        reports.push(QualityRow {
            document_id: envelope.document_id,
            candidate_id: envelope.candidate.candidate_id,
            profile: envelope.profile,
            classification: classify(&status, score.required_projection_recall),
            legacy_classification: classify(&legacy_status, score.required_projection_recall),
            status,
            legacy_status,
            usable: report.usable,
            reasons: report.reasons,
            required_anchor_recall: score.required_anchor_recall,
            required_projection_recall: score.required_projection_recall,
            required_page_qualified_recall: score.required_page_qualified_recall,
            required_structural_recall: score.required_structural_recall,
        });
    }
    reports.sort_by(|left, right| {
        (
            left.candidate_id.as_str(),
            left.profile.as_str(),
            left.document_id.as_str(),
        )
            .cmp(&(
                right.candidate_id.as_str(),
                right.profile.as_str(),
                right.document_id.as_str(),
            ))
    });
    write_json(
        &output,
        &QualityAggregate {
            schema_version: "graphite-m5-quality-oracle-v3",
            quality_profile_id,
            reports,
        },
    )
}

fn legacy_observable_attempt(attempt: &ExtractionAttempt) -> ExtractionAttempt {
    let mut legacy = attempt.clone();
    let profile_id = legacy.observations.provider_profile_id.clone();
    legacy.observations.page_text_density_anomaly_indices = ExtractionSignal::unavailable(
        SignalAvailability::NotReported,
        &profile_id,
        "signal was introduced by extraction-quality-v1.1",
    );
    legacy.observations.ocr_used = ExtractionSignal::unavailable(
        SignalAvailability::NotReported,
        &profile_id,
        "OCR confidence was not evaluated by extraction-quality-v1",
    );
    legacy.observations.ocr_mean_confidence = ExtractionSignal::unavailable(
        SignalAvailability::NotReported,
        &profile_id,
        "signal was introduced by extraction-quality-v1.1",
    );
    legacy.observations.structure_retention_warnings = ExtractionSignal::unavailable(
        SignalAvailability::NotReported,
        &profile_id,
        "signal was introduced by extraction-quality-v1.1",
    );
    legacy.observations.replacement_character_count = ExtractionSignal::unavailable(
        SignalAvailability::NotReported,
        &profile_id,
        "signal was introduced by extraction-quality-v1.1",
    );
    legacy.observations.projection_truncated = ExtractionSignal::unavailable(
        SignalAvailability::NotReported,
        &profile_id,
        "signal was introduced by extraction-quality-v1.1",
    );
    legacy.observations.native_to_projection_retention_ratio = ExtractionSignal::unavailable(
        SignalAvailability::NotReported,
        &profile_id,
        "signal was introduced by extraction-quality-v1.1",
    );
    legacy
}

fn production_attempt(envelope: &BenchmarkEnvelope, score: &ScoreRow) -> ExtractionAttempt {
    let profile_id = format!(
        "{}-{}-{}",
        envelope.candidate.candidate_id, envelope.candidate.version, envelope.profile
    );
    let provider = provider_descriptor(envelope, &profile_id);
    let mut observations = ExtractionObservations::unreported(
        &profile_id,
        format!("benchmark-artifact-{}", envelope.document_id),
        1,
        &envelope.input_sha256,
    );
    observations.reported_page_count = optional_signal(
        envelope.observations.reported_page_count,
        &profile_id,
        "reported page count was not available",
    );
    observations.extracted_page_count = optional_signal(
        envelope.observations.extracted_page_count,
        &profile_id,
        "extracted page count was not available",
    );
    observations.pages_with_text = ExtractionSignal::observed(
        envelope.observations.pages_with_text.len() as u32,
        &profile_id,
    );
    observations.empty_page_indices =
        ExtractionSignal::observed(envelope.observations.empty_pages.clone(), &profile_id);
    observations.suspicious_page_indices =
        ExtractionSignal::observed(envelope.observations.suspicious_pages.clone(), &profile_id);
    observations.native_text_presence = optional_signal(
        envelope.observations.native_text_present,
        &profile_id,
        "native text presence was not reported",
    );
    observations.ocr_used = optional_signal(
        envelope.observations.ocr_used,
        &profile_id,
        "OCR use was not reported",
    );
    observations.ocr_page_indices = if envelope.observations.ocr_page_indices_reported == Some(true)
    {
        ExtractionSignal::observed(envelope.observations.ocr_pages.clone(), &profile_id)
    } else {
        ExtractionSignal::unavailable(
            SignalAvailability::NotReported,
            &profile_id,
            "OCR page indices were not reported",
        )
    };
    observations.encoding_warnings =
        ExtractionSignal::observed(envelope.observations.encoding_warnings.clone(), &profile_id);
    observations.layout_warnings =
        ExtractionSignal::observed(envelope.observations.layout_warnings.clone(), &profile_id);
    observations.characters_by_page = ExtractionSignal::observed(
        envelope.observations.characters_by_page.clone(),
        &profile_id,
    );
    observations.page_text_density_anomaly_indices =
        ExtractionSignal::observed(score.page_character_anomaly_pages.clone(), &profile_id);
    observations.replacement_character_count =
        ExtractionSignal::observed(score.replacement_character_count, &profile_id);
    observations.structure_retention_warnings = ExtractionSignal::observed(
        envelope.observations.structure_retention_warnings.clone(),
        &profile_id,
    );
    observations.projection_truncated =
        ExtractionSignal::observed(score.truncation_suspected, &profile_id);
    observations.native_to_projection_retention_ratio = envelope
        .observations
        .provider_text_character_count
        .filter(|count| *count > 0)
        .map_or_else(
            || {
                ExtractionSignal::unavailable(
                    SignalAvailability::NotReported,
                    &profile_id,
                    "provider text character count was not reported",
                )
            },
            |provider_count| {
                ExtractionSignal::observed(
                    score.unicode_scalar_count as f64 / provider_count as f64,
                    &profile_id,
                )
            },
        );
    let ocr_confidences = envelope
        .observations
        .ocr_page_provenance
        .iter()
        .filter_map(|provenance| provenance.confidence)
        .collect::<Vec<_>>();
    observations.ocr_mean_confidence = if ocr_confidences.is_empty() {
        ExtractionSignal::unavailable(
            SignalAvailability::NotReported,
            &profile_id,
            "OCR confidence was not reported",
        )
    } else {
        ExtractionSignal::observed(
            ocr_confidences.iter().sum::<f64>() / ocr_confidences.len() as f64,
            &profile_id,
        )
    };
    if let Some(document) = &envelope.canonical_document {
        let text = render_plain_text(document);
        observations.plain_text_character_count =
            ExtractionSignal::observed(text.chars().count() as u64, &profile_id);
        let (headings, tables, formulas, figures, captions, references) =
            canonical_counts(document);
        observations.heading_count = ExtractionSignal::observed(headings, &profile_id);
        observations.table_count = ExtractionSignal::observed(tables, &profile_id);
        observations.formula_count = ExtractionSignal::observed(formulas, &profile_id);
        observations.figure_count = ExtractionSignal::observed(figures, &profile_id);
        observations.caption_count = ExtractionSignal::observed(captions, &profile_id);
        observations.reference_count = ExtractionSignal::observed(references, &profile_id);
    }
    ExtractionAttempt {
        provider,
        canonical_document: envelope.canonical_document.clone(),
        provenance: BTreeMap::new(),
        observations,
        warnings: envelope.warnings.clone(),
        error: failed_outcome(&envelope.outcome)
            .then(|| format!("benchmark outcome was {}", envelope.outcome)),
    }
}

fn provider_descriptor(
    envelope: &BenchmarkEnvelope,
    profile_id: &str,
) -> ExtractionProviderDescriptor {
    ExtractionProviderDescriptor {
        profile_id: profile_id.to_owned(),
        provider_id: envelope.candidate.candidate_id.clone(),
        provider_version: envelope.candidate.version.clone(),
        mapping_profile: envelope.profile.clone(),
        supported_media_types: vec!["application/pdf".to_owned()],
        required_input_forms: vec!["local-file".to_owned()],
        native_text_capability: true,
        layout_capability: true,
        table_capability: true,
        formula_capability: true,
        figure_capability: true,
        ocr_capability: true,
        page_provenance_capability: true,
        bounding_box_capability: true,
        configured_available: true,
    }
}

fn canonical_counts(document: &CanonicalDocumentV1) -> (u32, u32, u32, u32, u32, u32) {
    let mut counts = [0_u32; 6];
    for block in &document.blocks {
        let index = match block.content {
            CanonicalBlockContent::Heading { .. } => Some(0),
            CanonicalBlockContent::Table { .. } => Some(1),
            CanonicalBlockContent::Formula { .. } => Some(2),
            CanonicalBlockContent::Figure { .. } => Some(3),
            CanonicalBlockContent::Caption { .. } => Some(4),
            CanonicalBlockContent::Reference { .. } => Some(5),
            _ => None,
        };
        if let Some(index) = index {
            counts[index] += 1;
        }
    }
    (
        counts[0], counts[1], counts[2], counts[3], counts[4], counts[5],
    )
}

fn optional_signal<T: Clone>(
    value: Option<T>,
    provenance: &str,
    explanation: &str,
) -> ExtractionSignal<T> {
    value.map_or_else(
        || ExtractionSignal::unavailable(SignalAvailability::NotReported, provenance, explanation),
        |value| ExtractionSignal::observed(value, provenance),
    )
}

fn failed_outcome(outcome: &str) -> bool {
    matches!(
        outcome,
        "failed" | "unsupported" | "unavailable" | "timeout" | "crashed" | "parser_failure"
    )
}

fn classify(status: &str, recall: f64) -> String {
    match (status, recall == 1.0) {
        ("good", true) => "correct-good",
        ("good", false) => "false-good",
        ("degraded", true) => "false-degraded",
        ("degraded", false) => "correct-degraded",
        ("failed", _) => "correct-failed",
        _ => "unclassified",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{classify, failed_outcome};

    #[test]
    fn production_quality_confusion_labels_are_explicit() {
        assert_eq!(classify("good", 1.0), "correct-good");
        assert_eq!(classify("good", 0.5), "false-good");
        assert_eq!(classify("degraded", 1.0), "false-degraded");
        assert_eq!(classify("degraded", 0.5), "correct-degraded");
        assert_eq!(classify("failed", 0.0), "correct-failed");
    }

    #[test]
    fn non_output_outcomes_feed_the_production_failure_path() {
        for outcome in [
            "failed",
            "unsupported",
            "unavailable",
            "timeout",
            "crashed",
            "parser_failure",
        ] {
            assert!(failed_outcome(outcome));
        }
        assert!(!failed_outcome("succeeded"));
        assert!(!failed_outcome("suspicious"));
    }
}
