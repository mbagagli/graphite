use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::AppResult;

use super::model::{AgentEvidence, AgentIntent, ContextAssemblyStats};
use super::provider::EvidenceTokenCounter;

pub const CONTEXT_ASSEMBLER_VERSION: &str = "bounded-diverse-context-v2";
pub const MAX_EVIDENCE: usize = 16;
pub const MAX_PASSAGES_PER_PAPER: usize = 3;
pub const COMPACT_EVIDENCE_BUDGET_TOKENS: u32 = 6_000;
pub const LARGE_EVIDENCE_BUDGET_TOKENS: u32 = 12_000;
pub const TOKEN_ACCOUNTING_PROFILE: &str = "provider-token-accounting-v1";

pub struct EvidenceSelectionStats {
    input_evidence_count: u32,
    scope_filtered_count: u32,
    exact_duplicates_omitted: u32,
    redundant_evidence_omitted: u32,
    selection_limit_omitted: u32,
}

pub struct EvidenceSelection {
    pub evidence: Vec<AgentEvidence>,
    stats: EvidenceSelectionStats,
}

pub struct ContextAssembly {
    pub context: String,
    pub included_evidence: Vec<AgentEvidence>,
    pub stats: ContextAssemblyStats,
}

pub fn select_evidence(
    evidence: Vec<AgentEvidence>,
    intent: &AgentIntent,
    scope_item_ids: &[String],
    priority_item_ids: &[String],
) -> EvidenceSelection {
    let input_evidence_count = evidence.len() as u32;
    let allowed = scope_item_ids.iter().collect::<HashSet<_>>();
    let mut seen_fingerprints = HashSet::new();
    let mut retained_for_redundancy = Vec::<AgentEvidence>::new();
    let mut scope_filtered_count = 0_u32;
    let mut exact_duplicates_omitted = 0_u32;
    let mut redundant_evidence_omitted = 0_u32;

    for source in evidence {
        let in_scope = matches!(intent, AgentIntent::CitationStructure)
            || allowed.is_empty()
            || source
                .item_id
                .as_ref()
                .is_some_and(|item_id| allowed.contains(item_id));
        if !in_scope {
            scope_filtered_count = scope_filtered_count.saturating_add(1);
            continue;
        }
        if !seen_fingerprints.insert(source.source_fingerprint.clone()) {
            exact_duplicates_omitted = exact_duplicates_omitted.saturating_add(1);
            continue;
        }
        if retained_for_redundancy
            .iter()
            .any(|retained| is_contained_duplicate(&source, retained))
        {
            redundant_evidence_omitted = redundant_evidence_omitted.saturating_add(1);
            continue;
        }
        retained_for_redundancy.push(source);
    }

    let filtered_count = retained_for_redundancy.len();
    let per_paper_limit = if matches!(intent, AgentIntent::CurrentPaperQa) {
        10
    } else {
        MAX_PASSAGES_PER_PAPER
    };
    let mut buckets = HashMap::<String, VecDeque<AgentEvidence>>::new();
    let mut order = Vec::new();
    for item_id in priority_item_ids.iter().chain(scope_item_ids.iter()) {
        if !order.contains(item_id) {
            order.push(item_id.clone());
        }
    }
    for source in retained_for_redundancy {
        let key = source
            .item_id
            .clone()
            .unwrap_or_else(|| "structural".to_owned());
        if !order.contains(&key) {
            order.push(key.clone());
        }
        buckets.entry(key).or_default().push_back(source);
    }

    let mut selected = Vec::new();
    for _ in 0..per_paper_limit {
        let mut added = false;
        for item_id in &order {
            if selected.len() >= MAX_EVIDENCE {
                break;
            }
            if let Some(source) = buckets.get_mut(item_id).and_then(VecDeque::pop_front) {
                selected.push(source);
                added = true;
            }
        }
        if !added || selected.len() >= MAX_EVIDENCE {
            break;
        }
    }

    EvidenceSelection {
        stats: EvidenceSelectionStats {
            input_evidence_count,
            scope_filtered_count,
            exact_duplicates_omitted,
            redundant_evidence_omitted,
            selection_limit_omitted: filtered_count.saturating_sub(selected.len()) as u32,
        },
        evidence: selected,
    }
}

pub fn assemble_context(
    selection: EvidenceSelection,
    budget_tokens: u32,
    token_counter: &dyn EvidenceTokenCounter,
) -> AppResult<ContextAssembly> {
    let candidate_source_count = selection.evidence.len() as u32;
    let mut context = String::new();
    let mut included_evidence = Vec::new();
    let mut used_tokens = 0_u32;
    let mut oversized_evidence_omitted = 0_u32;
    let mut omitted_for_budget = 0_u32;
    let mut first_non_fitting_source_id = None;
    let mut first_non_fitting_source_tokens = None;
    let mut first_non_fitting_source_oversized = false;

    for source in selection.evidence {
        let mut candidate_source = source;
        candidate_source.source_id = format!("S{}", included_evidence.len() + 1);
        let block = render_context_source(&candidate_source);
        let block_tokens = token_counter.count_tokens(&block)?;
        let candidate_context = format!("{context}{block}");
        let candidate_tokens = token_counter.count_tokens(&candidate_context)?;
        if candidate_tokens > budget_tokens {
            let oversized = block_tokens > budget_tokens;
            if oversized {
                oversized_evidence_omitted = oversized_evidence_omitted.saturating_add(1);
            } else {
                omitted_for_budget = omitted_for_budget.saturating_add(1);
            }
            if first_non_fitting_source_id.is_none() {
                first_non_fitting_source_id = Some(
                    candidate_source
                        .chunk_id
                        .clone()
                        .unwrap_or_else(|| candidate_source.source_fingerprint.clone()),
                );
                first_non_fitting_source_tokens = Some(block_tokens);
                first_non_fitting_source_oversized = oversized;
            }
            continue;
        }
        context = candidate_context;
        used_tokens = candidate_tokens;
        included_evidence.push(candidate_source);
    }

    Ok(ContextAssembly {
        context,
        stats: ContextAssemblyStats {
            profile: CONTEXT_ASSEMBLER_VERSION.to_owned(),
            token_accounting: token_counter.token_accounting_method().to_owned(),
            budget_tokens,
            used_tokens,
            unused_tokens: budget_tokens.saturating_sub(used_tokens),
            input_evidence_count: selection.stats.input_evidence_count,
            scope_filtered_count: selection.stats.scope_filtered_count,
            exact_duplicates_omitted: selection.stats.exact_duplicates_omitted,
            redundancy_policy: "exact-fingerprint-and-contained-range-v1".to_owned(),
            redundant_evidence_omitted: selection.stats.redundant_evidence_omitted,
            selection_limit_omitted: selection.stats.selection_limit_omitted,
            candidate_source_count,
            included_source_count: included_evidence.len() as u32,
            oversized_evidence_omitted,
            omitted_for_budget,
            stopped_on_non_fitting_source: false,
            first_non_fitting_source_id,
            first_non_fitting_source_tokens,
            first_non_fitting_source_oversized,
            unevaluated_after_budget_stop: 0,
            duration_ms: 0,
        },
        included_evidence,
    })
}

pub fn render_context_source(source: &AgentEvidence) -> String {
    let excerpt = source.excerpt.as_deref().unwrap_or("Metadata record only");
    let page = source
        .page_start
        .map(|page| format!("page={page}"))
        .unwrap_or_else(|| "page=unavailable".to_owned());
    format!(
        "<source id=\"{}\" kind=\"{}\" eligible_scientific=\"{}\">\nPaper: {}\nProvenance: {} - {}\nContent: {}\n</source>\n",
        source.source_id,
        source.kind,
        source.eligible_for_scientific_claim,
        escape_source(&source.item_title),
        escape_source(&source.provider),
        page,
        escape_source(excerpt),
    )
}

fn is_contained_duplicate(candidate: &AgentEvidence, retained: &AgentEvidence) -> bool {
    if candidate.document_id.is_none()
        || candidate.document_id != retained.document_id
        || candidate.document_revision_id != retained.document_revision_id
    {
        return false;
    }
    let (Some(candidate_start), Some(candidate_end), Some(retained_start), Some(retained_end)) = (
        candidate.character_start,
        candidate.character_end,
        retained.character_start,
        retained.character_end,
    ) else {
        return false;
    };
    if candidate_start < retained_start || candidate_end > retained_end {
        return false;
    }
    let candidate_text = normalized_excerpt(candidate);
    let retained_text = normalized_excerpt(retained);
    !candidate_text.is_empty() && retained_text.contains(&candidate_text)
}

fn normalized_excerpt(source: &AgentEvidence) -> String {
    source
        .excerpt
        .as_deref()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn escape_source(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::{assemble_context, render_context_source, select_evidence};
    use crate::agent::model::{AgentEvidence, AgentIntent};
    use crate::agent::provider::DeterministicFakeProvider;

    fn evidence(
        item_id: &str,
        fingerprint: &str,
        chunk_id: &str,
        start: u64,
        end: u64,
        excerpt: &str,
    ) -> AgentEvidence {
        AgentEvidence {
            source_id: String::new(),
            kind: "document-passage".to_owned(),
            item_id: Some(item_id.to_owned()),
            item_title: format!("Paper {item_id}"),
            document_id: Some(format!("document-{item_id}")),
            document_revision_id: Some(7),
            chunk_id: Some(chunk_id.to_owned()),
            excerpt: Some(excerpt.to_owned()),
            character_start: Some(start),
            character_end: Some(end),
            page_start: None,
            page_end: None,
            retrieval_modes: vec!["hybrid".to_owned()],
            lexical_rank: Some(1),
            semantic_rank: Some(1),
            provider: "fixture".to_owned(),
            source_fingerprint: fingerprint.to_owned(),
            eligible_for_scientific_claim: true,
            stale: false,
        }
    }

    #[test]
    fn removes_exact_and_contained_duplicates_but_keeps_useful_adjacent_evidence() {
        let broad = evidence(
            "a",
            "broad",
            "chunk-broad",
            0,
            100,
            "opening evidence middle evidence closing evidence",
        );
        let exact = evidence(
            "a",
            "broad",
            "chunk-exact",
            0,
            100,
            "opening evidence middle evidence closing evidence",
        );
        let contained = evidence(
            "a",
            "contained",
            "chunk-contained",
            20,
            50,
            "middle evidence",
        );
        let adjacent = evidence(
            "a",
            "adjacent",
            "chunk-adjacent",
            90,
            150,
            "closing evidence and a materially different result",
        );

        let selection = select_evidence(
            vec![broad, exact, contained, adjacent],
            &AgentIntent::LibrarySynthesis,
            &[],
            &[],
        );

        assert_eq!(selection.evidence.len(), 2);
        assert_eq!(selection.stats.exact_duplicates_omitted, 1);
        assert_eq!(selection.stats.redundant_evidence_omitted, 1);
        assert_eq!(
            selection.evidence[0].chunk_id.as_deref(),
            Some("chunk-broad")
        );
        assert_eq!(
            selection.evidence[1].chunk_id.as_deref(),
            Some("chunk-adjacent")
        );
    }

    #[test]
    fn rendered_context_and_returned_evidence_are_exactly_aligned() {
        let selection = select_evidence(
            vec![
                evidence("a", "a1", "chunk-a", 0, 30, "alpha result"),
                evidence("b", "b1", "chunk-b", 0, 30, "beta result"),
            ],
            &AgentIntent::MultiPaperComparison,
            &[],
            &["b".to_owned(), "a".to_owned()],
        );
        let counter = DeterministicFakeProvider {
            response: String::new(),
        };
        let assembly = assemble_context(selection, 200, &counter).expect("context assembly");

        let rendered = assembly
            .included_evidence
            .iter()
            .map(render_context_source)
            .collect::<String>();
        assert_eq!(assembly.context, rendered);
        assert_eq!(assembly.included_evidence[0].item_id.as_deref(), Some("b"));
        assert_eq!(assembly.included_evidence[0].source_id, "S1");
        assert_eq!(assembly.included_evidence[1].source_id, "S2");
        assert_eq!(
            assembly.stats.token_accounting,
            "deterministic-whitespace-test-v1"
        );
    }
}
