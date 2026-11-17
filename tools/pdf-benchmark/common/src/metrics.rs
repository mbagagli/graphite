use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::canonical::canonical_hash;
use crate::model::{
    Anchor, AnchorKind, AnchorMatcher, AnchorResult, AnchorSet, AnchorVerificationStatus,
    BenchmarkElement, BenchmarkElementKind, BenchmarkExtractionEnvelopeV1, CorpusDocument,
    ExtractionOutcome, ScoreReport, ZoteroTextSidecar,
};
use crate::validation::content_hash;

pub fn score_extraction(
    document: &CorpusDocument,
    anchors: &AnchorSet,
    extraction: &BenchmarkExtractionEnvelopeV1,
    baseline: Option<&ZoteroTextSidecar>,
) -> ScoreReport {
    let verified_anchors = AnchorSet {
        schema_version: anchors.schema_version,
        document_id: anchors.document_id.clone(),
        anchors: anchors
            .anchors
            .iter()
            .filter(|anchor| anchor.verification_status == AnchorVerificationStatus::HumanVerified)
            .cloned()
            .collect(),
    };
    let projection = extraction.searchable_projection.as_deref().unwrap_or("");
    let page_text = extraction
        .document
        .as_ref()
        .map(|value| {
            value
                .pages
                .iter()
                .map(|page| (page.page_number, page.text.as_str()))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let elements = extraction
        .document
        .as_ref()
        .map(|document| document.elements.as_slice())
        .unwrap_or_default();
    let mut anchor_results = Vec::new();
    for anchor in &verified_anchors.anchors {
        anchor_results.push(match_anchor(anchor, projection, &page_text, elements));
    }
    apply_order_constraints(&mut anchor_results, &verified_anchors, extraction);
    let required_projection_recall = recall_dimension(&anchor_results, true, |result| {
        Some(result.projection_matched)
    })
    .unwrap_or(1.0);
    let required_page_qualified_recall = recall_dimension(&anchor_results, true, |result| {
        result.page_qualified_matched
    });
    let required_structural_recall =
        recall_dimension(&anchor_results, true, |result| result.structural_matched);
    let required_anchor_recall = recall(&anchor_results, true);
    let optional_anchor_recall = recall(&anchor_results, false);
    let required_projection_failure = extraction.outcome.has_output()
        && anchor_results
            .iter()
            .any(|result| result.required && !result.projection_matched);
    let silent_loss = required_projection_failure;
    let (benchmark_quality, mut quality_reasons) =
        benchmark_quality(extraction, required_projection_failure);
    let false_good = extraction.outcome == ExtractionOutcome::Succeeded
        && extraction.warnings.is_empty()
        && extraction.observations.empty_pages.is_empty()
        && extraction.observations.suspicious_pages.is_empty()
        && extraction.observations.encoding_warnings.is_empty()
        && extraction.observations.layout_warnings.is_empty()
        && required_projection_failure;
    if false_good {
        quality_reasons.push("quality gate false-good: required anchor is missing".to_owned());
    }
    let provider_degraded = matches!(
        extraction.outcome,
        ExtractionOutcome::Partial | ExtractionOutcome::Suspicious
    ) || !extraction.warnings.is_empty()
        || !extraction.observations.encoding_warnings.is_empty()
        || !extraction.observations.layout_warnings.is_empty()
        || !extraction.observations.empty_pages.is_empty()
        || !extraction.observations.suspicious_pages.is_empty();
    let false_degraded = provider_degraded
        && !required_projection_failure
        && extraction.warnings.is_empty()
        && extraction.observations.empty_pages.is_empty()
        && extraction.observations.suspicious_pages.is_empty()
        && extraction.observations.encoding_warnings.is_empty()
        && extraction.observations.layout_warnings.is_empty();
    let expected = document
        .expected_ocr_pages
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let observed = extraction
        .observations
        .ocr_pages
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let expected_ocr_recall = if expected.is_empty()
        || extraction.observations.ocr_page_indices_reported == Some(false)
    {
        None
    } else {
        Some(expected.intersection(&observed).count() as f64 / expected.len() as f64)
    };
    let false_ocr_pages = observed.difference(&expected).copied().collect::<Vec<_>>();
    let (replacement, controls, private_use) = character_anomalies(projection);
    let expected_page_count = document.expected_pages;
    let extracted_page_count = extraction.observations.extracted_page_count;
    let pages_with_text = extraction
        .observations
        .pages_with_text
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let missing_pages = expected_page_count
        .map(|count| {
            (1..=count)
                .filter(|page| !pages_with_text.contains(page))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let page_coverage_ratio = expected_page_count.map(|count| {
        if count == 0 {
            1.0
        } else {
            pages_with_text
                .iter()
                .filter(|page| **page <= count)
                .count() as f64
                / count as f64
        }
    });
    let characters_by_page = extraction.observations.characters_by_page.clone();
    let page_character_anomaly_pages = page_character_anomalies(&characters_by_page);
    let baseline_character_ratio =
        baseline.map(|value| ratio(projection.chars().count(), value.content.chars().count()));
    let baseline_token_ratio = baseline.map(|value| {
        ratio(
            benchmark_tokens(projection).len(),
            benchmark_tokens(&value.content).len(),
        )
    });
    let gross_coverage_warning = baseline_character_ratio.is_some_and(outside_coverage_band)
        || baseline_token_ratio.is_some_and(outside_coverage_band);
    let truncation_suspected = baseline_character_ratio.is_some_and(|value| value < 0.65)
        || baseline_token_ratio.is_some_and(|value| value < 0.65)
        || expected_page_count
            .zip(extracted_page_count)
            .is_some_and(|(expected, extracted)| extracted < expected)
        || expected_page_count.is_some_and(|count| {
            count > 0 && !pages_with_text.is_empty() && !pages_with_text.contains(&count)
        });
    ScoreReport {
        schema_version: "graphite-pdf-benchmark-score-v1".to_owned(),
        document_id: extraction.document_id.clone(),
        candidate_id: extraction.candidate.candidate_id.clone(),
        profile: extraction.profile.clone(),
        outcome: extraction.outcome.clone(),
        anchor_results,
        required_projection_recall,
        required_page_qualified_recall,
        required_structural_recall,
        required_anchor_recall,
        optional_anchor_recall,
        byte_count: projection.len() as u64,
        unicode_scalar_count: projection.chars().count() as u64,
        token_count: benchmark_tokens(projection).len() as u64,
        expected_page_count,
        extracted_page_count,
        page_coverage_ratio,
        missing_pages,
        characters_by_page,
        page_character_anomaly_pages,
        baseline_character_ratio,
        baseline_token_ratio,
        gross_coverage_warning,
        truncation_suspected,
        replacement_character_count: replacement,
        control_character_count: controls,
        private_use_character_count: private_use,
        duplicate_five_gram_ratio: duplicate_five_gram_ratio(projection),
        empty_pages: extraction.observations.empty_pages.clone(),
        suspicious_pages: extraction.observations.suspicious_pages.clone(),
        expected_ocr_recall,
        false_ocr_pages,
        silent_loss,
        benchmark_quality,
        quality_reasons,
        false_good,
        false_degraded,
        projection_hash: (!projection.is_empty()).then(|| content_hash(projection.as_bytes())),
        canonical_hash: extraction
            .canonical_document
            .as_ref()
            .and_then(|value| canonical_hash(value).ok()),
    }
}

fn ratio(observed: usize, baseline: usize) -> f64 {
    if baseline == 0 {
        if observed == 0 {
            1.0
        } else {
            observed as f64
        }
    } else {
        observed as f64 / baseline as f64
    }
}

fn outside_coverage_band(value: f64) -> bool {
    !(0.65..=1.60).contains(&value)
}

fn page_character_anomalies(characters: &[u64]) -> Vec<u32> {
    let mut non_zero = characters
        .iter()
        .copied()
        .filter(|count| *count > 0)
        .collect::<Vec<_>>();
    if non_zero.len() < 3 {
        return Vec::new();
    }
    non_zero.sort_unstable();
    let median = non_zero[non_zero.len() / 2] as f64;
    characters
        .iter()
        .enumerate()
        .filter(|(_, count)| {
            let value = **count as f64;
            value == 0.0 || value < median * 0.10 || value > median * 10.0
        })
        .map(|(index, _)| index as u32 + 1)
        .collect()
}

fn match_anchor(
    anchor: &Anchor,
    projection: &str,
    pages: &HashMap<u32, &str>,
    elements: &[BenchmarkElement],
) -> AnchorResult {
    let matched_projection = matcher_matches(&anchor.matcher, projection);
    let matched_page = anchor
        .expected_pages
        .iter()
        .find(|page| {
            pages
                .get(page)
                .is_some_and(|text| matcher_matches(&anchor.matcher, text))
        })
        .copied();
    let page_qualified_matched =
        (!anchor.expected_pages.is_empty()).then_some(matched_projection && matched_page.is_some());
    let structural_matched = structural_kind(&anchor.kind).map(|kind| {
        anchor.kind == AnchorKind::ReadingOrderPair
            || elements.iter().any(|element| {
                element.kind == kind
                    && matcher_matches(&anchor.matcher, &element_searchable_text(element))
            })
    });
    let complete = matched_projection
        && page_qualified_matched.unwrap_or(true)
        && structural_matched.unwrap_or(true);
    AnchorResult {
        anchor_id: anchor.id.clone(),
        kind: anchor.kind.clone(),
        required: anchor.required,
        projection_matched: matched_projection,
        page_qualified_matched,
        structural_matched,
        matched: complete,
        matched_page,
        reason: if !matched_projection {
            "anchor text was not found in the searchable projection".to_owned()
        } else if page_qualified_matched == Some(false) {
            "anchor text was not associated with an expected page".to_owned()
        } else if structural_matched == Some(false) {
            "anchor was not preserved with its required structural type".to_owned()
        } else {
            "matched deterministically".to_owned()
        },
    }
}

fn structural_kind(kind: &AnchorKind) -> Option<BenchmarkElementKind> {
    match kind {
        AnchorKind::Heading => Some(BenchmarkElementKind::Heading),
        AnchorKind::TableValue => Some(BenchmarkElementKind::Table),
        AnchorKind::Formula => Some(BenchmarkElementKind::Formula),
        AnchorKind::FigureCaption => Some(BenchmarkElementKind::Caption),
        AnchorKind::Reference => Some(BenchmarkElementKind::Reference),
        AnchorKind::ReadingOrderPair => Some(BenchmarkElementKind::Other),
        AnchorKind::TextFact | AnchorKind::PageAssociation => None,
    }
}

fn element_searchable_text(element: &BenchmarkElement) -> String {
    element.table.as_ref().map_or_else(
        || element.text.clone(),
        |table| {
            table
                .cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<Vec<_>>()
                .join("\t")
        },
    )
}

fn matcher_matches(matcher: &AnchorMatcher, text: &str) -> bool {
    match matcher {
        AnchorMatcher::Exact { value } => text.contains(value),
        AnchorMatcher::Normalized { value } => normalized(text).contains(&normalized(value)),
        AnchorMatcher::Alternatives { values } => values
            .iter()
            .any(|value| normalized(text).contains(&normalized(value))),
        AnchorMatcher::OrderedTokens { tokens, max_gap } => {
            ordered_tokens_match(&benchmark_tokens(text), tokens, *max_gap)
        }
        AnchorMatcher::NumericContext {
            value,
            unit,
            tolerance,
            context,
            window,
        } => numeric_context_match(
            text,
            value,
            unit.as_deref(),
            *tolerance,
            context,
            window.unwrap_or(120),
        ),
    }
}

fn ordered_tokens_match(haystack: &[String], needles: &[String], max_gap: usize) -> bool {
    if needles.is_empty() {
        return false;
    }
    for start in 0..haystack.len() {
        if haystack[start] != normalized(&needles[0]) {
            continue;
        }
        let mut position = start;
        let mut matched = true;
        for needle in needles.iter().skip(1) {
            let expected = normalized(needle);
            let limit = (position + max_gap + 2).min(haystack.len());
            if let Some(next) = haystack[position + 1..limit]
                .iter()
                .position(|token| token == &expected)
            {
                position += next + 1;
            } else {
                matched = false;
                break;
            }
        }
        if matched {
            return true;
        }
    }
    false
}

fn numeric_context_match(
    text: &str,
    value: &str,
    unit: Option<&str>,
    tolerance: Option<f64>,
    context: &[String],
    window: usize,
) -> bool {
    let normalized_text = normalized(text);
    let target = match value.parse::<f64>() {
        Ok(value) => value,
        Err(_) => return false,
    };
    let allowed = tolerance.unwrap_or(0.0);
    let candidates = benchmark_tokens(text)
        .into_iter()
        .filter(|token| {
            token
                .parse::<f64>()
                .is_ok_and(|candidate| (candidate - target).abs() <= allowed)
        })
        .collect::<BTreeSet<_>>();
    for needle in candidates {
        for (position, _) in normalized_text.match_indices(&needle) {
            if !is_token_occurrence(&normalized_text, position, needle.len()) {
                continue;
            }
            let (start, end) = safe_text_window(&normalized_text, position, needle.len(), window);
            let scope = &normalized_text[start..end];
            let unit_ok = unit.is_none_or(|unit| scope.contains(&normalized(unit)));
            let context_ok = context.iter().all(|item| scope.contains(&normalized(item)));
            if unit_ok && context_ok {
                return true;
            }
        }
    }
    false
}

fn is_token_occurrence(text: &str, position: usize, length: usize) -> bool {
    let previous = text[..position].chars().next_back();
    let next = text[position + length..].chars().next();
    previous.is_none_or(|character| !benchmark_token_character(character))
        && next.is_none_or(|character| !benchmark_token_character(character))
}

fn safe_text_window(text: &str, position: usize, length: usize, window: usize) -> (usize, usize) {
    let mut start = position.saturating_sub(window);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (position + length + window).min(text.len());
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    (start, end)
}

fn benchmark_token_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '.' | '+' | '-' | '_' | '/' | '%' | '^')
}

fn apply_order_constraints(
    results: &mut [AnchorResult],
    anchors: &AnchorSet,
    extraction: &BenchmarkExtractionEnvelopeV1,
) {
    let Some(document) = extraction.document.as_ref() else {
        for result in results
            .iter_mut()
            .filter(|result| result.kind == AnchorKind::ReadingOrderPair)
        {
            result.structural_matched = Some(false);
            result.matched = false;
            result.reason = "reading-order provenance was not reported".to_owned();
        }
        return;
    };
    let elements = document
        .elements
        .iter()
        .map(|element| (element.id.as_str(), element))
        .collect::<HashMap<_, _>>();
    let mut positions = BTreeMap::new();
    for anchor in &anchors.anchors {
        if let Some(position) = document.reading_order.iter().position(|element_id| {
            elements.get(element_id.as_str()).is_some_and(|element| {
                matcher_matches(&anchor.matcher, &element_searchable_text(element))
            })
        }) {
            positions.insert(anchor.id.clone(), position);
        }
    }
    for (index, anchor) in anchors.anchors.iter().enumerate() {
        if anchor.kind != AnchorKind::ReadingOrderPair {
            continue;
        }
        let after_ok = anchor.expected_after.as_ref().is_none_or(|other| {
            positions
                .get(other)
                .zip(positions.get(&anchor.id))
                .is_some_and(|(other_position, own_position)| own_position > other_position)
        });
        let before_ok = anchor.expected_before.as_ref().is_none_or(|other| {
            positions
                .get(other)
                .zip(positions.get(&anchor.id))
                .is_some_and(|(other_position, own_position)| own_position < other_position)
        });
        if !after_ok || !before_ok {
            results[index].structural_matched = Some(false);
            results[index].matched = false;
            results[index].reason = "reading-order constraint was not satisfied".to_owned();
        } else {
            results[index].structural_matched = Some(true);
            results[index].matched = results[index].projection_matched
                && results[index].page_qualified_matched.unwrap_or(true);
        }
    }
}

fn benchmark_quality(
    extraction: &BenchmarkExtractionEnvelopeV1,
    required_anchor_failure: bool,
) -> (String, Vec<String>) {
    let mut reasons = Vec::new();
    if !extraction.outcome.has_output()
        || extraction
            .searchable_projection
            .as_deref()
            .is_none_or(str::is_empty)
    {
        reasons.push("extraction has no searchable output".to_owned());
        return ("failed".to_owned(), reasons);
    }
    if !extraction.observations.encoding_warnings.is_empty() {
        reasons.push("encoding warnings were observed".to_owned());
    }
    if !extraction.observations.layout_warnings.is_empty() {
        reasons.push("layout warnings were observed".to_owned());
    }
    if !extraction.observations.empty_pages.is_empty() {
        reasons.push("empty pages were observed".to_owned());
    }
    if !extraction.observations.suspicious_pages.is_empty() {
        reasons.push("suspicious pages were observed".to_owned());
    }
    if extraction.outcome != ExtractionOutcome::Succeeded {
        reasons.push(format!("outcome is {:?}", extraction.outcome));
    }
    if required_anchor_failure {
        reasons.push("required benchmark anchor or constraint failed".to_owned());
    }
    if reasons.is_empty() {
        ("good".to_owned(), reasons)
    } else {
        ("degraded".to_owned(), reasons)
    }
}

fn recall(results: &[AnchorResult], required: bool) -> f64 {
    let selected = results
        .iter()
        .filter(|result| result.required == required)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return 1.0;
    }
    selected.iter().filter(|result| result.matched).count() as f64 / selected.len() as f64
}

fn recall_dimension(
    results: &[AnchorResult],
    required: bool,
    dimension: impl Fn(&AnchorResult) -> Option<bool>,
) -> Option<f64> {
    let selected = results
        .iter()
        .filter(|result| result.required == required)
        .filter_map(dimension)
        .collect::<Vec<_>>();
    (!selected.is_empty())
        .then(|| selected.iter().filter(|matched| **matched).count() as f64 / selected.len() as f64)
}

pub fn benchmark_tokens(text: &str) -> Vec<String> {
    text.split(|character: char| !benchmark_token_character(character))
        .filter(|token| !token.is_empty())
        .map(normalized)
        .collect()
}

pub fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn character_anomalies(text: &str) -> (u64, u64, u64) {
    let mut replacement = 0;
    let mut controls = 0;
    let mut private_use = 0;
    for character in text.chars() {
        if character == '\u{fffd}' {
            replacement += 1;
        }
        if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
            controls += 1;
        }
        let value = character as u32;
        if matches!(value, 0xe000..=0xf8ff | 0xf0000..=0xffffd | 0x100000..=0x10fffd) {
            private_use += 1;
        }
    }
    (replacement, controls, private_use)
}

fn duplicate_five_gram_ratio(text: &str) -> f64 {
    let tokens = benchmark_tokens(text);
    if tokens.len() < 5 {
        return 0.0;
    }
    let mut counts = HashMap::new();
    for window in tokens.windows(5) {
        *counts.entry(window.join(" ")).or_insert(0usize) += 1;
    }
    let repeated = counts
        .values()
        .map(|count| count.saturating_sub(1))
        .sum::<usize>();
    repeated as f64 / (tokens.len() - 4) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AnchorVerificationStatus, BenchmarkProfile, CandidateDescriptor, ExtractionObservations,
        TextLayerKind, TimingObservation, ANCHOR_SCHEMA_VERSION, BENCHMARK_SCHEMA_VERSION,
    };

    fn extraction(text: &str) -> BenchmarkExtractionEnvelopeV1 {
        BenchmarkExtractionEnvelopeV1 {
            schema_version: BENCHMARK_SCHEMA_VERSION.to_owned(),
            request_id: "request".to_owned(),
            document_id: "document".to_owned(),
            input_sha256: "0".repeat(64),
            candidate: CandidateDescriptor {
                candidate_id: "fixture".to_owned(),
                version: "1".to_owned(),
                revision: "fixture".to_owned(),
                license: "MIT".to_owned(),
                adapter_version: "1".to_owned(),
                features: Vec::new(),
                supported_profiles: vec![BenchmarkProfile::Native],
                configured_available: true,
                availability_reason: None,
            },
            profile: BenchmarkProfile::Native,
            outcome: ExtractionOutcome::Succeeded,
            outcome_reason: None,
            searchable_projection: Some(text.to_owned()),
            document: None,
            canonical_document: None,
            mapping_losses: Vec::new(),
            warnings: Vec::new(),
            observations: ExtractionObservations::default(),
            timing: TimingObservation::default(),
            diagnostics: BTreeMap::new(),
        }
    }

    #[test]
    fn missing_required_anchor_is_silent_loss_and_false_good_is_prevented() {
        let document = CorpusDocument {
            id: "document".to_owned(),
            file: "local/document.pdf".to_owned(),
            sha256: "0".repeat(64),
            source_url: None,
            license: "generated".to_owned(),
            redistributable: true,
            categories: vec!["native".to_owned()],
            language: "en".to_owned(),
            expected_pages: Some(1),
            text_layer: TextLayerKind::Native,
            expected_ocr_pages: Vec::new(),
            anchors: "anchors/document.json".to_owned(),
            anchors_sha256: "0".repeat(64),
            zotero_baseline: None,
            zotero_baseline_sha256: None,
        };
        let anchors = AnchorSet {
            schema_version: ANCHOR_SCHEMA_VERSION,
            document_id: "document".to_owned(),
            anchors: vec![Anchor {
                id: "required".to_owned(),
                kind: AnchorKind::TextFact,
                required: true,
                verification_status: AnchorVerificationStatus::HumanVerified,
                verification_note: "verified test fixture".to_owned(),
                expected_pages: Vec::new(),
                matcher: AnchorMatcher::Exact {
                    value: "missing".to_owned(),
                },
                expected_after: None,
                expected_before: None,
            }],
        };
        let report = score_extraction(&document, &anchors, &extraction("present"), None);
        assert!(report.silent_loss);
        assert_eq!(report.benchmark_quality, "degraded");
        assert!(report.false_good);
    }

    #[test]
    fn failed_reading_order_is_separate_from_projection_loss() {
        let document = CorpusDocument {
            id: "document".to_owned(),
            file: "local/document.pdf".to_owned(),
            sha256: "0".repeat(64),
            source_url: None,
            license: "generated".to_owned(),
            redistributable: true,
            categories: vec!["native".to_owned()],
            language: "en".to_owned(),
            expected_pages: Some(1),
            text_layer: TextLayerKind::Native,
            expected_ocr_pages: Vec::new(),
            anchors: "anchors/document.json".to_owned(),
            anchors_sha256: "0".repeat(64),
            zotero_baseline: None,
            zotero_baseline_sha256: None,
        };
        let anchors = AnchorSet {
            schema_version: ANCHOR_SCHEMA_VERSION,
            document_id: "document".to_owned(),
            anchors: vec![
                Anchor {
                    id: "first".to_owned(),
                    kind: AnchorKind::ReadingOrderPair,
                    required: true,
                    verification_status: AnchorVerificationStatus::HumanVerified,
                    verification_note: "verified test fixture".to_owned(),
                    expected_pages: Vec::new(),
                    matcher: AnchorMatcher::Exact {
                        value: "first".to_owned(),
                    },
                    expected_after: Some("second".to_owned()),
                    expected_before: None,
                },
                Anchor {
                    id: "second".to_owned(),
                    kind: AnchorKind::TextFact,
                    required: true,
                    verification_status: AnchorVerificationStatus::HumanVerified,
                    verification_note: "verified test fixture".to_owned(),
                    expected_pages: Vec::new(),
                    matcher: AnchorMatcher::Exact {
                        value: "second".to_owned(),
                    },
                    expected_after: None,
                    expected_before: None,
                },
            ],
        };

        let report = score_extraction(&document, &anchors, &extraction("first then second"), None);

        assert_eq!(report.required_anchor_recall, 0.5);
        assert_eq!(report.required_projection_recall, 1.0);
        assert_eq!(report.required_structural_recall, Some(0.0));
        assert!(!report.silent_loss);
        assert!(!report.false_good);
    }

    #[test]
    fn projection_page_and_structure_recall_are_independent() {
        let document = CorpusDocument {
            id: "document".to_owned(),
            file: "local/document.pdf".to_owned(),
            sha256: "0".repeat(64),
            source_url: None,
            license: "generated".to_owned(),
            redistributable: true,
            categories: vec!["native".to_owned()],
            language: "en".to_owned(),
            expected_pages: Some(2),
            text_layer: TextLayerKind::Native,
            expected_ocr_pages: Vec::new(),
            anchors: "anchors/document.json".to_owned(),
            anchors_sha256: "0".repeat(64),
            zotero_baseline: None,
            zotero_baseline_sha256: None,
        };
        let anchors = AnchorSet {
            schema_version: ANCHOR_SCHEMA_VERSION,
            document_id: "document".to_owned(),
            anchors: vec![Anchor {
                id: "heading".to_owned(),
                kind: AnchorKind::Heading,
                required: true,
                verification_status: AnchorVerificationStatus::HumanVerified,
                verification_note: "verified test fixture".to_owned(),
                expected_pages: vec![2],
                matcher: AnchorMatcher::Exact {
                    value: "Methods".to_owned(),
                },
                expected_after: None,
                expected_before: None,
            }],
        };
        let mut value = extraction("Methods");
        value.document = Some(crate::model::BenchmarkDocumentV1 {
            pages: vec![crate::model::BenchmarkPage {
                page_number: 1,
                label: None,
                width: None,
                height: None,
                rotation_degrees: None,
                text: "Methods".to_owned(),
                regions: Vec::new(),
            }],
            elements: vec![crate::model::BenchmarkElement {
                id: "paragraph".to_owned(),
                kind: BenchmarkElementKind::Paragraph,
                text: "Methods".to_owned(),
                source_order: 0,
                page_number: Some(1),
                parent_id: None,
                heading_level: None,
                region: None,
                table: None,
                formula_latex: None,
                figure_asset_id: None,
                list_ordered: None,
            }],
            ..crate::model::BenchmarkDocumentV1::default()
        });

        let report = score_extraction(&document, &anchors, &value, None);
        assert_eq!(report.required_projection_recall, 1.0);
        assert_eq!(report.required_page_qualified_recall, Some(0.0));
        assert_eq!(report.required_structural_recall, Some(0.0));
        assert!(!report.silent_loss);
    }

    #[test]
    fn numeric_context_checks_every_matching_number_occurrence() {
        let text = format!(
            "100 km unrelated {} sampling rate was 100 Hz near croûte",
            "padding ".repeat(40)
        );

        assert!(numeric_context_match(
            &text,
            "100",
            Some("Hz"),
            Some(0.0),
            &["sampling".to_owned(), "rate".to_owned()],
            40,
        ));
    }

    #[test]
    fn unreported_ocr_page_indices_are_not_scored_as_zero_recall() {
        let document = CorpusDocument {
            id: "document".to_owned(),
            file: "local/document.pdf".to_owned(),
            sha256: "0".repeat(64),
            source_url: None,
            license: "generated".to_owned(),
            redistributable: true,
            categories: vec!["scanned".to_owned()],
            language: "en".to_owned(),
            expected_pages: Some(2),
            text_layer: TextLayerKind::Scanned,
            expected_ocr_pages: vec![2],
            anchors: "anchors/document.json".to_owned(),
            anchors_sha256: "0".repeat(64),
            zotero_baseline: None,
            zotero_baseline_sha256: None,
        };
        let anchors = AnchorSet {
            schema_version: ANCHOR_SCHEMA_VERSION,
            document_id: "document".to_owned(),
            anchors: Vec::new(),
        };
        let mut value = extraction("present");
        value.observations.ocr_used = Some(true);
        value.observations.ocr_page_indices_reported = Some(false);

        let report = score_extraction(&document, &anchors, &value, None);
        assert_eq!(report.expected_ocr_recall, None);

        value.observations.ocr_page_indices_reported = Some(true);
        value.observations.ocr_pages = vec![2];
        let report = score_extraction(&document, &anchors, &value, None);
        assert_eq!(report.expected_ocr_recall, Some(1.0));
    }

    #[test]
    fn suggested_anchor_is_excluded_from_fidelity_scoring() {
        let document = CorpusDocument {
            id: "document".to_owned(),
            file: "local/document.pdf".to_owned(),
            sha256: "0".repeat(64),
            source_url: None,
            license: "generated".to_owned(),
            redistributable: true,
            categories: vec!["native".to_owned()],
            language: "en".to_owned(),
            expected_pages: Some(1),
            text_layer: TextLayerKind::Native,
            expected_ocr_pages: Vec::new(),
            anchors: "anchors/document.json".to_owned(),
            anchors_sha256: "0".repeat(64),
            zotero_baseline: None,
            zotero_baseline_sha256: None,
        };
        let anchors = AnchorSet {
            schema_version: ANCHOR_SCHEMA_VERSION,
            document_id: "document".to_owned(),
            anchors: vec![Anchor {
                id: "unverified".to_owned(),
                kind: AnchorKind::TextFact,
                required: true,
                verification_status: AnchorVerificationStatus::SuggestedUnverified,
                verification_note: "candidate for human review".to_owned(),
                expected_pages: Vec::new(),
                matcher: AnchorMatcher::Exact {
                    value: "missing".to_owned(),
                },
                expected_after: None,
                expected_before: None,
            }],
        };

        let report = score_extraction(&document, &anchors, &extraction("present"), None);
        assert!(report.anchor_results.is_empty());
        assert!(!report.silent_loss);
        assert!(!report.false_good);
    }
}
