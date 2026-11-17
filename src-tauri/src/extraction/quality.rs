use super::model::{
    ExtractionAttempt, ExtractionQualityProfile, ExtractionQualityReport, ExtractionQualityStatus,
    SignalAvailability,
};
use super::projection::{render_plain_text, validate_canonical_document};

#[derive(Clone, Debug)]
pub struct ExtractionQualityEvaluator {
    profile: ExtractionQualityProfile,
}

impl ExtractionQualityEvaluator {
    pub fn new(profile: ExtractionQualityProfile) -> Self {
        Self { profile }
    }

    pub fn default_v1() -> Self {
        Self::new(ExtractionQualityProfile {
            id: "extraction-quality-v1.1".to_owned(),
            version: "1.1.0".to_owned(),
            require_searchable_text: true,
            flat_pdf_text_is_degraded: true,
            maximum_coverage_discrepancy: 0.15,
        })
    }

    pub fn profile(&self) -> &ExtractionQualityProfile {
        &self.profile
    }

    pub fn evaluate(&self, attempt: &ExtractionAttempt) -> ExtractionQualityReport {
        let mut reasons = Vec::new();
        if let Some(error) = &attempt.error {
            reasons.push(error.clone());
            return self.report(attempt, ExtractionQualityStatus::Failed, false, reasons);
        }
        let Some(document) = &attempt.canonical_document else {
            reasons.push("provider returned no canonical document".to_owned());
            return self.report(attempt, ExtractionQualityStatus::Failed, false, reasons);
        };
        if let Err(error) = validate_canonical_document(document) {
            reasons.push(format!("canonical validation failed: {error}"));
            return self.report(attempt, ExtractionQualityStatus::Failed, false, reasons);
        }
        let plain_text = render_plain_text(document);
        if self.profile.require_searchable_text && plain_text.is_empty() {
            reasons.push("searchable projection is empty".to_owned());
            return self.report(attempt, ExtractionQualityStatus::Failed, false, reasons);
        }
        if self.profile.flat_pdf_text_is_degraded
            && !attempt.provider.layout_capability
            && document.pages.is_empty()
        {
            reasons.push("flat source text has no page or layout provenance".to_owned());
        }
        append_observed_warning_reason(
            &mut reasons,
            "encoding warnings",
            &attempt.observations.encoding_warnings,
        );
        append_observed_warning_reason(
            &mut reasons,
            "layout warnings",
            &attempt.observations.layout_warnings,
        );
        append_observed_indices_reason(
            &mut reasons,
            "empty pages",
            &attempt.observations.empty_page_indices,
        );
        append_observed_indices_reason(
            &mut reasons,
            "suspicious pages",
            &attempt.observations.suspicious_page_indices,
        );
        append_observed_indices_reason(
            &mut reasons,
            "page text density anomalies",
            &attempt.observations.page_text_density_anomaly_indices,
        );
        append_observed_warning_reason(
            &mut reasons,
            "structure retention warnings",
            &attempt.observations.structure_retention_warnings,
        );
        if attempt
            .observations
            .replacement_character_count
            .value
            .is_some_and(|count| count > 0)
        {
            reasons.push("replacement characters are present".to_owned());
        }
        if attempt.observations.projection_truncated.value == Some(true) {
            reasons.push("searchable projection was truncated".to_owned());
        }
        if attempt
            .observations
            .native_to_projection_retention_ratio
            .value
            .is_some_and(|ratio| ratio < 1.0 - self.profile.maximum_coverage_discrepancy)
        {
            reasons.push("structured projection retained too little provider text".to_owned());
        }
        if attempt.observations.ocr_used.value == Some(true)
            && attempt.observations.ocr_mean_confidence.availability != SignalAvailability::Observed
        {
            reasons.push("OCR confidence was not reported".to_owned());
        }
        if attempt
            .observations
            .coverage_discrepancy
            .value
            .is_some_and(|value| value > self.profile.maximum_coverage_discrepancy)
        {
            reasons.push("alternative extraction coverage differs".to_owned());
        }
        if attempt.provider.page_provenance_capability
            && attempt.observations.extracted_page_count.availability
                != SignalAvailability::Observed
        {
            reasons.push("page coverage was not reported".to_owned());
        }
        let status = if reasons.is_empty() {
            ExtractionQualityStatus::Good
        } else {
            ExtractionQualityStatus::Degraded
        };
        self.report(attempt, status, !plain_text.is_empty(), reasons)
    }

    fn report(
        &self,
        attempt: &ExtractionAttempt,
        status: ExtractionQualityStatus,
        usable: bool,
        reasons: Vec<String>,
    ) -> ExtractionQualityReport {
        ExtractionQualityReport {
            profile_id: self.profile.id.clone(),
            status,
            usable,
            reasons,
            signals: attempt.observations.clone(),
        }
    }
}

fn append_observed_warning_reason(
    reasons: &mut Vec<String>,
    label: &str,
    signal: &super::model::ExtractionSignal<Vec<String>>,
) {
    if signal
        .value
        .as_ref()
        .is_some_and(|values| !values.is_empty())
    {
        reasons.push(label.to_owned());
    }
}

fn append_observed_indices_reason(
    reasons: &mut Vec<String>,
    label: &str,
    signal: &super::model::ExtractionSignal<Vec<u32>>,
) {
    if signal
        .value
        .as_ref()
        .is_some_and(|values| !values.is_empty())
    {
        reasons.push(label.to_owned());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;

    use crate::database::content_hash;

    use super::*;
    use crate::extraction::model::{
        CanonicalBlock, CanonicalBlockContent, CanonicalDocumentMetadata, CanonicalDocumentV1,
        CanonicalPage, ExtractionObservations, ExtractionProviderDescriptor, ExtractionSignal,
        SignalAvailability, CANONICAL_SCHEMA_VERSION,
    };

    #[test]
    fn accepts_complete_native_text() {
        let attempt = fixture_attempt(true, "native text");
        let report = ExtractionQualityEvaluator::default_v1().evaluate(&attempt);
        assert_eq!(report.status, ExtractionQualityStatus::Good);
        assert!(report.usable);
    }

    #[test]
    fn degrades_flat_pdf_source_text() {
        let attempt = fixture_attempt(false, "flat text");
        let report = ExtractionQualityEvaluator::default_v1().evaluate(&attempt);
        assert_eq!(report.status, ExtractionQualityStatus::Degraded);
        assert!(report.usable);
        assert!(report
            .reasons
            .iter()
            .any(|reason| reason.contains("page or layout")));
    }

    #[test]
    fn fails_empty_searchable_projection() {
        let attempt = fixture_attempt(true, "");
        let report = ExtractionQualityEvaluator::default_v1().evaluate(&attempt);
        assert_eq!(report.status, ExtractionQualityStatus::Failed);
        assert!(!report.usable);
    }

    #[test]
    fn records_suspicious_pages_and_warnings_as_degraded() {
        let mut attempt = fixture_attempt(true, "usable text");
        attempt.observations.suspicious_page_indices =
            ExtractionSignal::observed(vec![2], "fixture");
        attempt.observations.encoding_warnings =
            ExtractionSignal::observed(vec!["replacement characters".to_owned()], "fixture");
        attempt.observations.layout_warnings =
            ExtractionSignal::observed(vec!["overlapping regions".to_owned()], "fixture");
        let report = ExtractionQualityEvaluator::default_v1().evaluate(&attempt);
        assert_eq!(report.status, ExtractionQualityStatus::Degraded);
        assert!(report.reasons.contains(&"suspicious pages".to_owned()));
        assert!(report.reasons.contains(&"encoding warnings".to_owned()));
        assert!(report.reasons.contains(&"layout warnings".to_owned()));
    }

    #[test]
    fn degrades_parser_neutral_silent_loss_signals() {
        let mut attempt = fixture_attempt(true, "usable text");
        attempt.observations.page_text_density_anomaly_indices =
            ExtractionSignal::observed(vec![2], "fixture");
        attempt.observations.replacement_character_count = ExtractionSignal::observed(3, "fixture");
        attempt.observations.structure_retention_warnings = ExtractionSignal::observed(
            vec!["provider tables were not retained".to_owned()],
            "fixture",
        );
        attempt.observations.native_to_projection_retention_ratio =
            ExtractionSignal::observed(0.5, "fixture");
        let report = ExtractionQualityEvaluator::default_v1().evaluate(&attempt);
        assert_eq!(report.status, ExtractionQualityStatus::Degraded);
        assert!(report
            .reasons
            .contains(&"page text density anomalies".to_owned()));
        assert!(report
            .reasons
            .contains(&"replacement characters are present".to_owned()));
        assert!(report
            .reasons
            .contains(&"structure retention warnings".to_owned()));
        assert!(report
            .reasons
            .contains(&"structured projection retained too little provider text".to_owned()));
    }

    #[test]
    fn degrades_ocr_without_confidence_but_not_native_text() {
        let mut attempt = fixture_attempt(true, "usable text");
        let native = ExtractionQualityEvaluator::default_v1().evaluate(&attempt);
        assert_eq!(native.status, ExtractionQualityStatus::Good);

        attempt.observations.ocr_used = ExtractionSignal::observed(true, "fixture");
        let ocr = ExtractionQualityEvaluator::default_v1().evaluate(&attempt);
        assert_eq!(ocr.status, ExtractionQualityStatus::Degraded);
        assert!(ocr
            .reasons
            .contains(&"OCR confidence was not reported".to_owned()));
    }

    #[test]
    fn preserves_explicit_ocr_signal_availability() {
        let mut attempt = fixture_attempt(true, "usable text");
        attempt.observations.ocr_used = ExtractionSignal::observed(true, "fixture");
        attempt.observations.ocr_page_indices = ExtractionSignal::observed(vec![1], "fixture");
        let report = ExtractionQualityEvaluator::default_v1().evaluate(&attempt);
        assert_eq!(
            report.signals.ocr_used.availability,
            SignalAvailability::Observed
        );
        assert_eq!(report.signals.ocr_used.value, Some(true));

        attempt.observations.ocr_used = ExtractionSignal::unavailable(
            SignalAvailability::Unsupported,
            "fixture",
            "OCR is unsupported",
        );
        let unsupported = ExtractionQualityEvaluator::default_v1().evaluate(&attempt);
        assert_eq!(
            unsupported.signals.ocr_used.availability,
            SignalAvailability::Unsupported
        );

        attempt.observations.ocr_used = ExtractionSignal::unavailable(
            SignalAvailability::NotReported,
            "fixture",
            "OCR use was not reported",
        );
        let unreported = ExtractionQualityEvaluator::default_v1().evaluate(&attempt);
        assert_eq!(
            unreported.signals.ocr_used.availability,
            SignalAvailability::NotReported
        );
    }

    pub(crate) fn fixture_attempt(structured: bool, text: &str) -> ExtractionAttempt {
        let profile_id = if structured {
            "native-fixture-v1"
        } else {
            "flat-fixture-v1"
        };
        let block_id = "block-1".to_owned();
        let pages = if structured {
            vec![CanonicalPage {
                id: "page-1".to_owned(),
                physical_page_number: 1,
                printed_label: None,
                width: Some(1.0),
                height: Some(1.0),
                rotation_degrees: Some(0),
                coordinate_space: Some("normalized-top-left".to_owned()),
            }]
        } else {
            Vec::new()
        };
        let document = CanonicalDocumentV1 {
            schema_version: CANONICAL_SCHEMA_VERSION.to_owned(),
            artifact_id: "artifact-1".to_owned(),
            artifact_revision_id: 1,
            extraction_profile_id: profile_id.to_owned(),
            metadata: CanonicalDocumentMetadata::default(),
            pages,
            blocks: vec![CanonicalBlock {
                id: block_id.clone(),
                parent_id: None,
                source_order: 0,
                section_path: Vec::new(),
                page_range: None,
                regions: Vec::new(),
                content: CanonicalBlockContent::Paragraph {
                    text: text.to_owned(),
                },
            }],
            reading_order: vec![block_id],
            relationships: Vec::new(),
            assets: Vec::new(),
        };
        let mut observations =
            ExtractionObservations::unreported(profile_id, "artifact-1", 1, content_hash(text));
        observations.plain_text_character_count =
            ExtractionSignal::observed(text.chars().count() as u64, "fixture");
        observations.plain_text_hash = ExtractionSignal::observed(content_hash(text), "fixture");
        observations.encoding_warnings = ExtractionSignal::observed(Vec::new(), "fixture");
        observations.layout_warnings = ExtractionSignal::observed(Vec::new(), "fixture");
        observations.empty_page_indices = ExtractionSignal::observed(Vec::new(), "fixture");
        observations.suspicious_page_indices = ExtractionSignal::observed(Vec::new(), "fixture");
        if structured {
            observations.reported_page_count = ExtractionSignal::observed(1, "fixture");
            observations.extracted_page_count = ExtractionSignal::observed(1, "fixture");
            observations.pages_with_text = ExtractionSignal::observed(1, "fixture");
            observations.native_text_presence = ExtractionSignal::observed(true, "fixture");
        }
        ExtractionAttempt {
            provider: ExtractionProviderDescriptor {
                profile_id: profile_id.to_owned(),
                provider_id: profile_id.to_owned(),
                provider_version: "1.0.0".to_owned(),
                mapping_profile: "fixture".to_owned(),
                supported_media_types: vec!["application/pdf".to_owned()],
                required_input_forms: Vec::new(),
                native_text_capability: structured,
                layout_capability: structured,
                table_capability: structured,
                formula_capability: structured,
                figure_capability: structured,
                ocr_capability: structured,
                page_provenance_capability: structured,
                bounding_box_capability: structured,
                configured_available: true,
            },
            canonical_document: Some(document),
            provenance: BTreeMap::new(),
            observations,
            warnings: Vec::new(),
            error: None,
        }
    }
}
