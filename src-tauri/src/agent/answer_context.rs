//! Render complete page ranges while preserving the frozen experiment assembler.

use crate::error::{AppError, AppResult};

pub use super::context::{
    select_evidence, ContextAssembly, EvidenceSelection, COMPACT_EVIDENCE_BUDGET_TOKENS,
    LARGE_EVIDENCE_BUDGET_TOKENS, MAX_EVIDENCE, MAX_PASSAGES_PER_PAPER, TOKEN_ACCOUNTING_PROFILE,
};
use super::model::AgentEvidence;
use super::provider::EvidenceTokenCounter;

pub const CONTEXT_ASSEMBLER_VERSION: &str = "bounded-diverse-context-v6";

/// Extend the frozen source renderer with the full physical-page range.
pub fn render_context_source(source: &AgentEvidence) -> String {
    let block = super::context::render_context_source(source);
    match (source.page_start, source.page_end) {
        (Some(start), Some(end)) if end != start => block.replacen(
            &format!(" - page={start}\nContent:"),
            &format!(" - pages={start}-{end}\nContent:"),
            1,
        ),
        _ => block,
    }
}

/// Reuse evidence selection and budgeting, recounting expanded page metadata.
pub fn assemble_context(
    selection: EvidenceSelection,
    budget_tokens: u32,
    token_counter: &dyn EvidenceTokenCounter,
) -> AppResult<ContextAssembly> {
    // The v2 source is checksum-locked by the separate frozen PDF experiment.
    let mut result = super::context::assemble_context(selection, budget_tokens, token_counter)?;
    loop {
        result.context = result
            .included_evidence
            .iter()
            .map(render_context_source)
            .collect();
        let tokens = token_counter.count_tokens(&result.context)?;
        if tokens <= budget_tokens {
            result.stats.profile = CONTEXT_ASSEMBLER_VERSION.to_owned();
            result.stats.used_tokens = tokens;
            result.stats.unused_tokens = budget_tokens - tokens;
            result.stats.included_source_count = result.included_evidence.len() as u32;
            return Ok(result);
        }
        if result.included_evidence.is_empty() {
            return Err(AppError::Agent(
                "The evidence budget cannot fit this tokenizer's empty context.".into(),
            ));
        }
        let removed = result
            .included_evidence
            .pop()
            .expect("nonempty over-budget context");
        let block_tokens = token_counter.count_tokens(&render_context_source(&removed))?;
        if block_tokens > budget_tokens {
            result.stats.oversized_evidence_omitted += 1;
        } else {
            result.stats.omitted_for_budget += 1;
        }
        if result.stats.first_non_fitting_source_id.is_none() {
            result.stats.first_non_fitting_source_id =
                Some(removed.chunk_id.unwrap_or(removed.source_fingerprint));
            result.stats.first_non_fitting_source_tokens = Some(block_tokens);
            result.stats.first_non_fitting_source_oversized = block_tokens > budget_tokens;
        }
    }
}

/// Focus exact-decimal lookups only when retrieved passages contain the literal.
///
/// This operates after scoped retrieval and never adds sources. Queries without
/// decimal anchors, or with no matching passage, retain the ordinary selection.
/// Whole passages and table expansions remain intact.
pub fn focus_literal_evidence(evidence: Vec<AgentEvidence>, question: &str) -> Vec<AgentEvidence> {
    let lookup = question.trim_start().to_lowercase();
    if !lookup.starts_with("which ") && !lookup.starts_with("where ") {
        return evidence;
    }
    let decimal = regex::Regex::new(r"\b\d+\.\d+\b").expect("static decimal anchor regex");
    let anchors: Vec<_> = decimal.find_iter(question).map(|m| m.as_str()).collect();
    if anchors.is_empty() {
        return evidence;
    }
    let matches = |source: &AgentEvidence| {
        source.eligible_for_scientific_claim
            && decimal
                .find_iter(source.excerpt.as_deref().unwrap_or_default())
                .any(|value| anchors.contains(&value.as_str()))
    };
    if !evidence.iter().any(matches) {
        return evidence;
    }
    evidence.into_iter().filter(matches).collect()
}

/// Distinguish representative descriptions from targeted evidence questions.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EvidencePurpose {
    Overview,
    Definition,
    Explanation,
    Specific,
}

/// Select evidence policy from ordinary question wording, never reference data.
pub(super) fn evidence_purpose(question: &str) -> EvidencePurpose {
    let question = question.to_lowercase();
    if [
        "research subject",
        "main subject",
        "main topic",
        "research purpose",
        "study design",
        "overview of the papers",
        "overview of these papers",
        "overview of selected papers",
    ]
    .iter()
    .any(|phrase| question.contains(phrase))
        || (question.contains("compare")
            && question.contains("paper")
            && (question.contains("purpose") || question.contains("evidence type")))
    {
        EvidencePurpose::Overview
    } else if question.contains("define") || question.contains("definition") {
        EvidencePurpose::Definition
    } else if question.contains("why ") || question.contains("explain ") {
        EvidencePurpose::Explanation
    } else {
        EvidencePurpose::Specific
    }
}

/// Split a bounded quantitative question into complementary search clauses.
pub(super) fn measurement_queries(question: &str) -> Vec<String> {
    let normalized = question.to_lowercase();
    if evidence_purpose(question) != EvidencePurpose::Specific
        || !normalized.split(|c: char| !c.is_alphabetic()).any(|word| {
            matches!(
                word,
                "rate"
                    | "rates"
                    | "number"
                    | "minimum"
                    | "maximum"
                    | "frequency"
                    | "frequencies"
                    | "speed"
            )
        })
    {
        return Vec::new();
    }
    for (first, second) in [("minimum", "maximum"), ("maximum", "minimum")] {
        let pair = format!("{first} and {second}");
        if normalized.contains(&pair) {
            return vec![
                normalized.replace(&pair, first),
                normalized.replace(&pair, second),
            ];
        }
    }
    let Some((first, second)) = normalized.split_once(" and ") else {
        return Vec::new();
    };
    if first.split_whitespace().count() < 2 || second.split_whitespace().count() < 2 {
        return Vec::new();
    }
    vec![first.trim().to_owned(), second.trim().to_owned()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantitative_queries_are_bounded_and_skip_overviews() {
        assert_eq!(
            measurement_queries("Give the acquisition frequency and channel count."),
            vec!["give the acquisition frequency", "channel count."]
        );
        assert_eq!(
            measurement_queries("Which standards specify minimum and maximum porosity?"),
            vec![
                "which standards specify minimum porosity?",
                "which standards specify maximum porosity?",
            ]
        );
        assert!(
            measurement_queries("Compare the purposes and evidence types of these papers.")
                .is_empty()
        );
        assert!(measurement_queries("Explain the rate and its uncertainty.").is_empty());
        assert!(
            evidence_purpose("Compare the papers' purposes and evidence types.")
                == EvidencePurpose::Overview
        );
        assert!(
            evidence_purpose("What type of evidence supports Table 7?")
                == EvidencePurpose::Specific
        );
        assert_eq!(
            measurement_queries("What number of channels and samples and recordings? ").len(),
            2
        );
    }
}
