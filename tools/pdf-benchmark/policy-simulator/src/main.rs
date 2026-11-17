//! Simulate parser-selection policies from privacy-safe benchmark metrics.
//!
//! This binary keeps routing inputs separate from human anchor evaluation.
//! Selection receives only production-observable extraction and quality
//! fields. Human anchor results are joined after selection to evaluate the
//! policy without leaking ground truth into the decision.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const CANDIDATES: [&str; 3] = ["xberg", "pdf-inspector", "docling"];
const AUTO_PROFILE: &str = "recommended-auto";
const ZOTERO_PROFILE: &str = "native";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Aggregate {
    reports: Vec<ScoreReport>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScoreReport {
    document_id: String,
    candidate_id: String,
    profile: String,
    outcome: String,
    anchor_results: Vec<AnchorResult>,
    page_coverage_ratio: Option<f64>,
    baseline_character_ratio: Option<f64>,
    gross_coverage_warning: bool,
    truncation_suspected: bool,
    replacement_character_count: u64,
    empty_pages: Vec<u32>,
    suspicious_pages: Vec<u32>,
    token_count: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnchorResult {
    anchor_id: String,
    required: bool,
    projection_matched: bool,
    page_qualified_matched: bool,
    structural_matched: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QualitySet {
    reports: Vec<QualityReport>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QualityReport {
    document_id: String,
    candidate_id: String,
    profile: String,
    status: String,
    usable: bool,
    classification: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperationalSet {
    reports: Vec<OperationalReport>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperationalReport {
    document_id: String,
    candidate_id: String,
    profile: String,
    timeout_class: String,
    adapter_binary_bytes: u64,
    asset_logical_bytes: u64,
    cold_start: OperationalPhase,
    warm_extraction: Option<OperationalPhase>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperationalPhase {
    outcome: String,
    wall_time_ms: u64,
    user_cpu_time_ms: Option<u64>,
    system_cpu_time_ms: Option<u64>,
    peak_process_tree_rss_bytes: Option<u64>,
    peak_process_tree_thread_count: Option<u64>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct AttemptKey {
    document_id: String,
    candidate_id: String,
    profile: String,
}

impl AttemptKey {
    fn new(document_id: &str, candidate_id: &str, profile: &str) -> Self {
        Self {
            document_id: document_id.to_owned(),
            candidate_id: candidate_id.to_owned(),
            profile: profile.to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
struct ObservableAttempt {
    key: AttemptKey,
    outcome: String,
    quality_status: String,
    quality_usable: bool,
    page_coverage_ratio: Option<f64>,
    baseline_character_ratio: Option<f64>,
    gross_coverage_warning: bool,
    truncation_suspected: bool,
    replacement_character_count: u64,
    empty_page_count: usize,
    suspicious_page_count: usize,
    token_count: u64,
}

#[derive(Clone, Debug)]
struct Decision {
    selected: Option<AttemptKey>,
    supplemental: Vec<AttemptKey>,
    fallback_used: bool,
    reason: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyOutput {
    schema_version: String,
    selection_input_contract: String,
    policy_definitions: Vec<PolicyDefinition>,
    summaries: Vec<PolicySummary>,
    decisions: Vec<DecisionOutput>,
    role_strategy_summaries: Vec<RoleStrategySummary>,
    role_strategy_decisions: Vec<RoleStrategyDecisionOutput>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyDefinition {
    id: String,
    description: String,
    observable_inputs: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicySummary {
    subset: String,
    policy_id: String,
    document_count: usize,
    selected_document_count: usize,
    fallback_document_count: usize,
    required_anchor_count: usize,
    projection_match_count: usize,
    page_qualified_match_count: usize,
    structurally_scored_anchor_count: usize,
    structural_match_count: usize,
    projection_recall: f64,
    page_qualified_recall: f64,
    structural_recall: Option<f64>,
    silent_loss_document_count: usize,
    false_good_document_count: usize,
    false_degraded_document_count: usize,
    selected_candidates: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DecisionOutput {
    subset: String,
    policy_id: String,
    document_id: String,
    selected_candidate_id: Option<String>,
    selected_profile: Option<String>,
    supplemental_candidates: Vec<String>,
    selected_outcome: Option<String>,
    selected_quality_status: Option<String>,
    fallback_used: bool,
    reason: String,
    required_anchor_count: usize,
    projection_match_count: usize,
    page_qualified_match_count: usize,
    structurally_scored_anchor_count: usize,
    structural_match_count: usize,
    silent_loss: bool,
    quality_classification: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoleStrategySummary {
    subset: String,
    policy_id: String,
    document_count: usize,
    required_anchor_count: usize,
    projection_match_count: usize,
    page_qualified_match_count: usize,
    structurally_scored_anchor_count: usize,
    structural_match_count: usize,
    projection_recall: f64,
    page_qualified_recall: f64,
    structural_recall: Option<f64>,
    silent_loss_document_count: usize,
    primary_false_good_document_count: usize,
    zotero_protected_anchor_count: usize,
    ocr_recovered_anchor_count: usize,
    structure_gained_anchor_count: usize,
    stage_activation_counts: BTreeMap<String, usize>,
    stage_quality_counts: BTreeMap<String, usize>,
    ordinary_document_count: usize,
    ordinary_cold_median_ms: Option<u64>,
    ordinary_cold_p90_ms: Option<u64>,
    ordinary_cold_worst_ms: Option<u64>,
    ordinary_warm_median_ms: Option<u64>,
    ordinary_warm_p90_ms: Option<u64>,
    ordinary_warm_worst_ms: Option<u64>,
    ordinary_cold_cpu_median_ms: Option<u64>,
    peak_sequential_process_tree_rss_bytes: Option<u64>,
    maximum_stage_thread_count: Option<u64>,
    unique_adapter_binary_bytes: u64,
    unique_prepared_asset_bytes: u64,
    timeout_document_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoleStrategyDecisionOutput {
    subset: String,
    policy_id: String,
    document_id: String,
    activated_stages: Vec<String>,
    accepted_attempts: Vec<String>,
    ocr_fallback_activated: bool,
    zotero_retained: bool,
    structure_demand: bool,
    trigger_reason: Option<String>,
    required_anchor_count: usize,
    projection_match_count: usize,
    page_qualified_match_count: usize,
    structurally_scored_anchor_count: usize,
    structural_match_count: usize,
    silent_loss: bool,
    primary_quality_status: Option<String>,
    stage_quality_states: BTreeMap<String, String>,
    zotero_protected_anchor_count: usize,
    ocr_recovered_anchor_count: usize,
    structure_gained_anchor_count: usize,
    timeout_class: Option<String>,
    cold_route_wall_time_ms: Option<u64>,
    warm_route_wall_time_ms: Option<u64>,
    cold_route_cpu_time_ms: Option<u64>,
    peak_sequential_process_tree_rss_bytes: Option<u64>,
    maximum_stage_thread_count: Option<u64>,
    route_timed_out: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RoleStrategySelection {
    activated: Vec<AttemptKey>,
    accepted: Vec<AttemptKey>,
    ocr_fallback_activated: bool,
    zotero_retained: bool,
    trigger_reason: Option<String>,
}

#[derive(Clone, Debug)]
struct CorpusData {
    scores: BTreeMap<AttemptKey, ScoreReport>,
    qualities: BTreeMap<AttemptKey, QualityReport>,
    observables: BTreeMap<AttemptKey, ObservableAttempt>,
    operations: BTreeMap<AttemptKey, OperationalReport>,
    documents: BTreeSet<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("pdf-benchmark-policy-simulator: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() != 7 {
        return Err(usage());
    }
    let smoke = load_corpus(
        Path::new(&arguments[0]),
        Path::new(&arguments[1]),
        Path::new(&arguments[2]),
    )?;
    let representative = load_corpus(
        Path::new(&arguments[3]),
        Path::new(&arguments[4]),
        Path::new(&arguments[5]),
    )?;
    let mut decisions = simulate_subset("smoke", &smoke)?;
    decisions.extend(simulate_subset("representative", &representative)?);
    let combined = combine_corpora(&smoke, &representative)?;
    decisions.extend(simulate_subset("combined", &combined)?);
    decisions.sort_by(|left, right| {
        (
            left.subset.as_str(),
            left.policy_id.as_str(),
            left.document_id.as_str(),
        )
            .cmp(&(
                right.subset.as_str(),
                right.policy_id.as_str(),
                right.document_id.as_str(),
            ))
    });
    let summaries = summarize(&decisions);
    let mut role_strategy_decisions = simulate_role_subset("smoke", &smoke)?;
    role_strategy_decisions.extend(simulate_role_subset("representative", &representative)?);
    role_strategy_decisions.extend(simulate_role_subset("combined", &combined)?);
    role_strategy_decisions.sort_by(|left, right| {
        (
            left.subset.as_str(),
            left.policy_id.as_str(),
            left.document_id.as_str(),
        )
            .cmp(&(
                right.subset.as_str(),
                right.policy_id.as_str(),
                right.document_id.as_str(),
            ))
    });
    let role_strategy_summaries = summarize_role_strategies(&role_strategy_decisions, &combined);
    let output = PolicyOutput {
        schema_version: "graphite-pdf-benchmark-policy-simulation-v2".to_owned(),
        selection_input_contract: "production-observable-only-v1".to_owned(),
        policy_definitions: policy_definitions(),
        summaries,
        decisions,
        role_strategy_summaries,
        role_strategy_decisions,
    };
    write_json_atomic(Path::new(&arguments[6]), &output)?;
    Ok(())
}

fn usage() -> String {
    [
        "usage:",
        "  graphite-pdf-benchmark-policy-simulator \\",
        "    SMOKE_AGGREGATE SMOKE_QUALITY SMOKE_OPERATIONAL \\",
        "    REPRESENTATIVE_AGGREGATE REPRESENTATIVE_QUALITY \\",
        "    REPRESENTATIVE_OPERATIONAL OUTPUT",
        "",
        "Selection uses only production-observable metrics. Human anchor",
        "results are joined after selection to evaluate each policy.",
    ]
    .join("\n")
}

fn load_corpus(
    aggregate_path: &Path,
    quality_path: &Path,
    operational_path: &Path,
) -> Result<CorpusData, String> {
    let aggregate: Aggregate = read_json(aggregate_path)?;
    let quality_set: QualitySet = read_json(quality_path)?;
    let operational_set: OperationalSet = read_json(operational_path)?;
    let mut qualities = BTreeMap::new();
    for quality in quality_set.reports {
        let key = AttemptKey::new(
            &quality.document_id,
            &quality.candidate_id,
            &quality.profile,
        );
        if qualities.insert(key, quality).is_some() {
            return Err("quality input contains a duplicate attempt".to_owned());
        }
    }
    let mut scores = BTreeMap::new();
    let mut observables = BTreeMap::new();
    let mut documents = BTreeSet::new();
    for score in aggregate.reports {
        let key = AttemptKey::new(&score.document_id, &score.candidate_id, &score.profile);
        let quality = qualities
            .get(&key)
            .ok_or_else(|| format!("quality input has no row for {key:?}"))?;
        let observable = ObservableAttempt {
            key: key.clone(),
            outcome: score.outcome.clone(),
            quality_status: quality.status.clone(),
            quality_usable: quality.usable,
            page_coverage_ratio: score.page_coverage_ratio,
            baseline_character_ratio: score.baseline_character_ratio,
            gross_coverage_warning: score.gross_coverage_warning,
            truncation_suspected: score.truncation_suspected,
            replacement_character_count: score.replacement_character_count,
            empty_page_count: score.empty_pages.len(),
            suspicious_page_count: score.suspicious_pages.len(),
            token_count: score.token_count,
        };
        documents.insert(score.document_id.clone());
        if scores.insert(key.clone(), score).is_some()
            || observables.insert(key, observable).is_some()
        {
            return Err("aggregate input contains a duplicate attempt".to_owned());
        }
    }
    let mut operations = BTreeMap::new();
    for operation in operational_set.reports {
        let key = AttemptKey::new(
            &operation.document_id,
            &operation.candidate_id,
            &operation.profile,
        );
        if operations.insert(key, operation).is_some() {
            return Err("operational input contains a duplicate attempt".to_owned());
        }
    }
    Ok(CorpusData {
        scores,
        qualities,
        observables,
        operations,
        documents,
    })
}

fn combine_corpora(left: &CorpusData, right: &CorpusData) -> Result<CorpusData, String> {
    let mut scores = left.scores.clone();
    let mut qualities = left.qualities.clone();
    let mut observables = left.observables.clone();
    let mut operations = left.operations.clone();
    let mut documents = left.documents.clone();
    for (key, value) in &right.scores {
        if scores.insert(key.clone(), value.clone()).is_some() {
            return Err(format!("combined corpus repeats score {key:?}"));
        }
    }
    for (key, value) in &right.qualities {
        if qualities.insert(key.clone(), value.clone()).is_some() {
            return Err(format!("combined corpus repeats quality {key:?}"));
        }
    }
    for (key, value) in &right.observables {
        if observables.insert(key.clone(), value.clone()).is_some() {
            return Err(format!("combined corpus repeats observable {key:?}"));
        }
    }
    for (key, value) in &right.operations {
        if operations.insert(key.clone(), value.clone()).is_some() {
            return Err(format!("combined corpus repeats operation {key:?}"));
        }
    }
    documents.extend(right.documents.iter().cloned());
    Ok(CorpusData {
        scores,
        qualities,
        observables,
        operations,
        documents,
    })
}

fn simulate_subset(subset: &str, corpus: &CorpusData) -> Result<Vec<DecisionOutput>, String> {
    let mut output = Vec::new();
    for document_id in &corpus.documents {
        for candidate in CANDIDATES {
            output.push(evaluate_decision(
                subset,
                &format!("single-provider:{candidate}"),
                document_id,
                select_single(corpus, document_id, candidate),
                corpus,
                candidate,
            )?);
            output.push(evaluate_decision(
                subset,
                &format!("structured-plus-zotero:{candidate}"),
                document_id,
                select_zotero_safety_net(corpus, document_id, candidate),
                corpus,
                candidate,
            )?);
            output.push(evaluate_decision(
                subset,
                &format!("comparative-safeguard:{candidate}"),
                document_id,
                select_comparative(corpus, document_id, candidate),
                corpus,
                candidate,
            )?);
        }
        output.push(evaluate_decision(
            subset,
            "lightweight-to-heavy:pdf-inspector-to-docling",
            document_id,
            select_tiered(corpus, document_id),
            corpus,
            "pdf-inspector",
        )?);
    }
    Ok(output)
}

fn select_single(corpus: &CorpusData, document_id: &str, candidate: &str) -> Decision {
    let key = AttemptKey::new(document_id, candidate, AUTO_PROFILE);
    let selected = corpus
        .observables
        .get(&key)
        .filter(|attempt| is_usable(attempt))
        .map(|attempt| attempt.key.clone());
    Decision {
        selected,
        supplemental: Vec::new(),
        fallback_used: false,
        reason: "single provider; accept its usable automatic result".to_owned(),
    }
}

fn select_zotero_safety_net(corpus: &CorpusData, document_id: &str, candidate: &str) -> Decision {
    let primary_key = AttemptKey::new(document_id, candidate, AUTO_PROFILE);
    let zotero_key = AttemptKey::new(document_id, "zotero", ZOTERO_PROFILE);
    if corpus.observables.get(&primary_key).is_some_and(is_usable) {
        let supplemental = corpus
            .observables
            .get(&zotero_key)
            .filter(|attempt| is_usable(attempt))
            .map(|attempt| vec![attempt.key.clone()])
            .unwrap_or_default();
        return Decision {
            selected: Some(primary_key),
            fallback_used: !supplemental.is_empty(),
            supplemental,
            reason: if corpus.observables.get(&zotero_key).is_some_and(is_usable) {
                "structured primary remained active with Zotero retained as a searchable safety net"
                    .to_owned()
            } else {
                "structured primary produced usable output; Zotero was unavailable".to_owned()
            },
        };
    }
    let selected = corpus
        .observables
        .get(&zotero_key)
        .filter(|attempt| is_usable(attempt))
        .map(|attempt| attempt.key.clone());
    Decision {
        fallback_used: selected.is_some(),
        selected,
        supplemental: Vec::new(),
        reason: "structured primary was unusable; use available Zotero text".to_owned(),
    }
}

fn select_tiered(corpus: &CorpusData, document_id: &str) -> Decision {
    let light_key = AttemptKey::new(document_id, "pdf-inspector", AUTO_PROFILE);
    let heavy_key = AttemptKey::new(document_id, "docling", AUTO_PROFILE);
    let light = corpus.observables.get(&light_key);
    let heavy = corpus.observables.get(&heavy_key);
    if light.is_some_and(|attempt| is_usable(attempt) && attempt.quality_status == "good") {
        return Decision {
            selected: Some(light_key),
            supplemental: Vec::new(),
            fallback_used: false,
            reason: "lightweight extraction was usable and good".to_owned(),
        };
    }
    if let (Some(light), Some(heavy)) = (light.filter(|value| is_usable(value)), heavy) {
        if is_usable(heavy) && observably_prefer_heavy(light, heavy) {
            return Decision {
                selected: Some(heavy_key),
                supplemental: Vec::new(),
                fallback_used: true,
                reason: "lightweight extraction was not good and the heavy result was observably preferable"
                    .to_owned(),
            };
        }
        return Decision {
            selected: Some(light_key),
            supplemental: Vec::new(),
            fallback_used: false,
            reason: "heavy result was not observably preferable; retain usable lightweight output"
                .to_owned(),
        };
    }
    if heavy.is_some_and(is_usable) {
        return Decision {
            selected: Some(heavy_key),
            supplemental: Vec::new(),
            fallback_used: true,
            reason: "lightweight extraction was unusable; use the usable heavy result".to_owned(),
        };
    }
    let selected = light
        .filter(|attempt| is_usable(attempt))
        .map(|attempt| attempt.key.clone());
    Decision {
        fallback_used: false,
        selected,
        supplemental: Vec::new(),
        reason: "heavy result was unusable; retain usable lightweight output if present".to_owned(),
    }
}

fn observably_prefer_heavy(light: &ObservableAttempt, heavy: &ObservableAttempt) -> bool {
    let light_rank = quality_rank(&light.quality_status);
    let heavy_rank = quality_rank(&heavy.quality_status);
    if heavy_rank != light_rank {
        return heavy_rank > light_rank;
    }
    if light.truncation_suspected != heavy.truncation_suspected {
        return !heavy.truncation_suspected;
    }
    let light_coverage = light.page_coverage_ratio.unwrap_or_default();
    let heavy_coverage = heavy.page_coverage_ratio.unwrap_or_default();
    if (heavy_coverage - light_coverage).abs() > f64::EPSILON {
        return heavy_coverage > light_coverage;
    }
    if heavy.replacement_character_count != light.replacement_character_count {
        return heavy.replacement_character_count < light.replacement_character_count;
    }
    heavy.token_count > light.token_count
}

fn quality_rank(status: &str) -> u8 {
    match status {
        "good" => 2,
        "degraded" => 1,
        _ => 0,
    }
}

fn select_comparative(corpus: &CorpusData, document_id: &str, candidate: &str) -> Decision {
    let primary_key = AttemptKey::new(document_id, candidate, AUTO_PROFILE);
    let zotero_key = AttemptKey::new(document_id, "zotero", ZOTERO_PROFILE);
    let primary = corpus.observables.get(&primary_key);
    let zotero = corpus
        .observables
        .get(&zotero_key)
        .filter(|value| is_usable(value));
    if primary.is_none_or(|attempt| !is_usable(attempt)) {
        return Decision {
            selected: zotero.map(|attempt| attempt.key.clone()),
            supplemental: Vec::new(),
            fallback_used: zotero.is_some(),
            reason: "primary was unusable; use available Zotero text".to_owned(),
        };
    }
    let primary = primary.expect("primary was checked above");
    if zotero.is_some() && has_comparative_warning(primary) {
        return Decision {
            selected: Some(primary_key),
            supplemental: vec![zotero_key],
            fallback_used: true,
            reason: comparative_warning_reason(primary),
        };
    }
    Decision {
        selected: Some(primary_key),
        supplemental: Vec::new(),
        fallback_used: false,
        reason: if zotero.is_some() {
            "primary passed frozen quality and comparison signals".to_owned()
        } else {
            "Zotero was unavailable; retain usable primary output".to_owned()
        },
    }
}

fn simulate_role_subset(
    subset: &str,
    corpus: &CorpusData,
) -> Result<Vec<RoleStrategyDecisionOutput>, String> {
    let mut output = Vec::new();
    for document_id in &corpus.documents {
        output.push(evaluate_role_strategy(
            subset,
            "role-based-content-v1",
            document_id,
            false,
            corpus,
        )?);
        output.push(evaluate_role_strategy(
            subset,
            "role-based-structure-demand-v1",
            document_id,
            true,
            corpus,
        )?);
    }
    Ok(output)
}

fn evaluate_role_strategy(
    subset: &str,
    policy_id: &str,
    document_id: &str,
    structure_demand: bool,
    corpus: &CorpusData,
) -> Result<RoleStrategyDecisionOutput, String> {
    let selection = select_role_strategy(document_id, structure_demand, &corpus.observables);
    let xberg_key = AttemptKey::new(document_id, "xberg", "native");
    let denominator = corpus
        .scores
        .get(&xberg_key)
        .or_else(|| {
            corpus
                .scores
                .values()
                .find(|score| score.document_id == document_id)
        })
        .ok_or_else(|| format!("no score row establishes anchors for {document_id}"))?;
    let required_anchors = denominator
        .anchor_results
        .iter()
        .filter(|anchor| anchor.required)
        .collect::<Vec<_>>();
    let before_zotero_keys = selection
        .accepted
        .iter()
        .filter(|key| key.candidate_id != "zotero" && key.candidate_id != "docling")
        .cloned()
        .collect::<Vec<_>>();
    let before_zotero = anchor_counts(&required_anchors, &before_zotero_keys, corpus);
    let after_zotero_keys = selection
        .accepted
        .iter()
        .filter(|key| key.candidate_id != "docling")
        .cloned()
        .collect::<Vec<_>>();
    let after_zotero = anchor_counts(&required_anchors, &after_zotero_keys, corpus);
    let before_structure = after_zotero;
    let final_counts = anchor_counts(&required_anchors, &selection.accepted, corpus);
    let xberg_only = anchor_counts(
        &required_anchors,
        &selection
            .accepted
            .iter()
            .filter(|key| key.candidate_id == "xberg")
            .cloned()
            .collect::<Vec<_>>(),
        corpus,
    );
    let ocr_recovered_anchor_count = before_zotero
        .projection
        .saturating_sub(xberg_only.projection);
    let zotero_protected_anchor_count = after_zotero
        .projection
        .saturating_sub(before_zotero.projection);
    let structure_gained_anchor_count = final_counts
        .structural
        .saturating_sub(before_structure.structural);
    let stage_quality_states = selection
        .activated
        .iter()
        .filter_map(|key| {
            corpus
                .qualities
                .get(key)
                .map(|quality| (attempt_label(key), quality.status.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let performance = route_performance(&selection.activated, corpus);
    Ok(RoleStrategyDecisionOutput {
        subset: subset.to_owned(),
        policy_id: policy_id.to_owned(),
        document_id: document_id.to_owned(),
        activated_stages: selection.activated.iter().map(attempt_label).collect(),
        accepted_attempts: selection.accepted.iter().map(attempt_label).collect(),
        ocr_fallback_activated: selection.ocr_fallback_activated,
        zotero_retained: selection.zotero_retained,
        structure_demand,
        trigger_reason: selection.trigger_reason,
        required_anchor_count: required_anchors.len(),
        projection_match_count: final_counts.projection,
        page_qualified_match_count: final_counts.page,
        structurally_scored_anchor_count: final_counts.structural_denominator,
        structural_match_count: final_counts.structural,
        silent_loss: final_counts.projection < required_anchors.len(),
        primary_quality_status: corpus
            .qualities
            .get(&xberg_key)
            .map(|quality| quality.status.clone()),
        stage_quality_states,
        zotero_protected_anchor_count,
        ocr_recovered_anchor_count,
        structure_gained_anchor_count,
        timeout_class: performance.timeout_class,
        cold_route_wall_time_ms: performance.cold_wall_time_ms,
        warm_route_wall_time_ms: performance.warm_wall_time_ms,
        cold_route_cpu_time_ms: performance.cold_cpu_time_ms,
        peak_sequential_process_tree_rss_bytes: performance.peak_rss_bytes,
        maximum_stage_thread_count: performance.max_threads,
        route_timed_out: performance.timed_out,
    })
}

fn select_role_strategy(
    document_id: &str,
    structure_demand: bool,
    observables: &BTreeMap<AttemptKey, ObservableAttempt>,
) -> RoleStrategySelection {
    let xberg_key = AttemptKey::new(document_id, "xberg", "native");
    let pdf_key = AttemptKey::new(document_id, "pdf-inspector", AUTO_PROFILE);
    let zotero_key = AttemptKey::new(document_id, "zotero", ZOTERO_PROFILE);
    let docling_key = AttemptKey::new(document_id, "docling", AUTO_PROFILE);
    let mut activated = vec![xberg_key.clone()];
    let mut accepted = Vec::new();
    if observables.get(&xberg_key).is_some_and(is_usable) {
        accepted.push(xberg_key.clone());
    }
    let trigger_reason = observables
        .get(&xberg_key)
        .and_then(ocr_fallback_reason)
        .or_else(|| {
            (!observables.get(&xberg_key).is_some_and(is_usable))
                .then(|| "Xberg native output was unusable".to_owned())
        });
    let ocr_fallback_activated = trigger_reason.is_some();
    if ocr_fallback_activated {
        activated.push(pdf_key.clone());
        if observables.get(&pdf_key).is_some_and(is_usable) {
            accepted.push(pdf_key);
        }
    }
    let zotero_retained = observables.get(&zotero_key).is_some_and(is_usable);
    if zotero_retained {
        activated.push(zotero_key.clone());
        accepted.push(zotero_key);
    }
    if structure_demand {
        activated.push(docling_key.clone());
        if observables.get(&docling_key).is_some_and(is_usable) {
            accepted.push(docling_key);
        }
    }
    deduplicate_keys(&mut accepted);
    deduplicate_keys(&mut activated);
    RoleStrategySelection {
        activated,
        accepted,
        ocr_fallback_activated,
        zotero_retained,
        trigger_reason,
    }
}

fn ocr_fallback_reason(attempt: &ObservableAttempt) -> Option<String> {
    if !is_usable(attempt) {
        return Some("Xberg native output was unusable".to_owned());
    }
    if attempt.token_count == 0 || attempt.outcome == "empty_success" {
        return Some("Xberg native output was empty".to_owned());
    }
    if attempt.page_coverage_ratio.is_some_and(|ratio| ratio < 1.0) {
        return Some("Xberg native page coverage was incomplete".to_owned());
    }
    if attempt.empty_page_count > 0 {
        return Some("Xberg native output contained empty pages".to_owned());
    }
    if attempt.suspicious_page_count > 0 {
        return Some("Xberg native output contained suspicious sparse pages".to_owned());
    }
    None
}

#[derive(Clone, Copy, Debug, Default)]
struct AnchorCounts {
    projection: usize,
    page: usize,
    structural_denominator: usize,
    structural: usize,
}

fn anchor_counts(
    anchors: &[&AnchorResult],
    accepted: &[AttemptKey],
    corpus: &CorpusData,
) -> AnchorCounts {
    let reports = accepted
        .iter()
        .filter_map(|key| corpus.scores.get(key))
        .collect::<Vec<_>>();
    AnchorCounts {
        projection: anchors
            .iter()
            .filter(|anchor| {
                any_selected_anchor(&reports, &anchor.anchor_id, |result| {
                    result.projection_matched
                })
            })
            .count(),
        page: anchors
            .iter()
            .filter(|anchor| {
                any_selected_anchor(&reports, &anchor.anchor_id, |result| {
                    result.page_qualified_matched
                })
            })
            .count(),
        structural_denominator: anchors
            .iter()
            .filter(|anchor| anchor.structural_matched.is_some())
            .count(),
        structural: anchors
            .iter()
            .filter(|anchor| {
                any_selected_anchor(&reports, &anchor.anchor_id, |result| {
                    result.structural_matched == Some(true)
                })
            })
            .count(),
    }
}

#[derive(Clone, Debug, Default)]
struct RoutePerformance {
    timeout_class: Option<String>,
    cold_wall_time_ms: Option<u64>,
    warm_wall_time_ms: Option<u64>,
    cold_cpu_time_ms: Option<u64>,
    peak_rss_bytes: Option<u64>,
    max_threads: Option<u64>,
    timed_out: bool,
}

fn route_performance(activated: &[AttemptKey], corpus: &CorpusData) -> RoutePerformance {
    let operations = activated
        .iter()
        .filter_map(|key| corpus.operations.get(key))
        .collect::<Vec<_>>();
    if operations.is_empty() {
        return RoutePerformance::default();
    }
    let cold_wall_time_ms = operations
        .iter()
        .map(|operation| operation.cold_start.wall_time_ms)
        .sum();
    let warm_wall_time_ms = operations
        .iter()
        .map(|operation| {
            operation
                .warm_extraction
                .as_ref()
                .map(|phase| phase.wall_time_ms)
        })
        .collect::<Option<Vec<_>>>()
        .map(|values| values.into_iter().sum());
    let cold_cpu_time_ms = operations
        .iter()
        .map(|operation| {
            Some(operation.cold_start.user_cpu_time_ms? + operation.cold_start.system_cpu_time_ms?)
        })
        .collect::<Option<Vec<_>>>()
        .map(|values| values.into_iter().sum());
    let peak_rss_bytes = operations
        .iter()
        .filter_map(|operation| operation.cold_start.peak_process_tree_rss_bytes)
        .max();
    let max_threads = operations
        .iter()
        .filter_map(|operation| operation.cold_start.peak_process_tree_thread_count)
        .max();
    let timed_out = operations.iter().any(|operation| {
        operation.cold_start.outcome == "timeout"
            || operation
                .warm_extraction
                .as_ref()
                .is_some_and(|phase| phase.outcome == "timeout")
    });
    RoutePerformance {
        timeout_class: route_timeout_class(&operations),
        cold_wall_time_ms: Some(cold_wall_time_ms),
        warm_wall_time_ms,
        cold_cpu_time_ms,
        peak_rss_bytes,
        max_threads,
        timed_out,
    }
}

fn route_timeout_class(operations: &[&OperationalReport]) -> Option<String> {
    if operations
        .iter()
        .any(|operation| operation.timeout_class == "extreme-stress")
    {
        return Some("extreme-stress".to_owned());
    }
    if operations
        .iter()
        .any(|operation| operation.timeout_class == "scanned-ocr")
    {
        return Some("scanned-ocr".to_owned());
    }
    operations
        .first()
        .map(|operation| operation.timeout_class.clone())
}

fn attempt_label(key: &AttemptKey) -> String {
    format!("{}/{}", key.candidate_id, key.profile)
}

fn deduplicate_keys(keys: &mut Vec<AttemptKey>) {
    let mut seen = BTreeSet::new();
    keys.retain(|key| seen.insert(key.clone()));
}

fn is_usable(attempt: &ObservableAttempt) -> bool {
    attempt.quality_usable
        && matches!(
            attempt.outcome.as_str(),
            "succeeded" | "partial" | "suspicious"
        )
}

fn has_comparative_warning(attempt: &ObservableAttempt) -> bool {
    attempt.quality_status != "good"
        || attempt.gross_coverage_warning
        || attempt.truncation_suspected
        || attempt.page_coverage_ratio.is_some_and(|ratio| ratio < 1.0)
        || attempt.replacement_character_count > 0
        || attempt.empty_page_count > 0
        || attempt.suspicious_page_count > 0
}

fn comparative_warning_reason(attempt: &ObservableAttempt) -> String {
    let mut reasons = Vec::new();
    if attempt.quality_status != "good" {
        reasons.push("quality was not good");
    }
    if attempt.gross_coverage_warning {
        reasons.push("frozen gross-coverage warning");
    }
    if attempt.truncation_suspected {
        reasons.push("truncation suspected");
    }
    if attempt.page_coverage_ratio.is_some_and(|ratio| ratio < 1.0) {
        reasons.push("incomplete page coverage");
    }
    if attempt.replacement_character_count > 0 {
        reasons.push("replacement characters observed");
    }
    if attempt.empty_page_count > 0 {
        reasons.push("empty pages observed");
    }
    if attempt.suspicious_page_count > 0 {
        reasons.push("suspicious pages observed");
    }
    let comparison = attempt
        .baseline_character_ratio
        .map(|ratio| format!("; candidate/Zotero character ratio {ratio:.3}"))
        .unwrap_or_default();
    format!(
        "comparison safeguard used Zotero: {}{}",
        reasons.join(", "),
        comparison
    )
}

fn evaluate_decision(
    subset: &str,
    policy_id: &str,
    document_id: &str,
    decision: Decision,
    corpus: &CorpusData,
    denominator_candidate: &str,
) -> Result<DecisionOutput, String> {
    let denominator_key = AttemptKey::new(document_id, denominator_candidate, AUTO_PROFILE);
    let denominator = corpus
        .scores
        .get(&denominator_key)
        .or_else(|| {
            corpus
                .scores
                .values()
                .find(|score| score.document_id == document_id)
        })
        .ok_or_else(|| format!("no score row establishes anchors for {document_id}"))?;
    let required_anchor_count = denominator
        .anchor_results
        .iter()
        .filter(|anchor| anchor.required)
        .count();
    let selected_score = decision
        .selected
        .as_ref()
        .and_then(|key| corpus.scores.get(key));
    let selected_quality = decision
        .selected
        .as_ref()
        .and_then(|key| corpus.qualities.get(key));
    let mut selected_keys = decision.selected.iter().collect::<Vec<_>>();
    selected_keys.extend(decision.supplemental.iter());
    let selected_reports = selected_keys
        .into_iter()
        .filter_map(|key| corpus.scores.get(key))
        .collect::<Vec<_>>();
    let required_anchors = denominator
        .anchor_results
        .iter()
        .filter(|anchor| anchor.required)
        .collect::<Vec<_>>();
    let projection_match_count = required_anchors
        .iter()
        .filter(|anchor| {
            any_selected_anchor(&selected_reports, &anchor.anchor_id, |result| {
                result.projection_matched
            })
        })
        .count();
    let page_qualified_match_count = required_anchors
        .iter()
        .filter(|anchor| {
            any_selected_anchor(&selected_reports, &anchor.anchor_id, |result| {
                result.page_qualified_matched
            })
        })
        .count();
    let structurally_scored_anchor_count = required_anchors
        .iter()
        .filter(|anchor| anchor.structural_matched.is_some())
        .count();
    let structural_match_count = required_anchors
        .iter()
        .filter(|anchor| {
            any_selected_anchor(&selected_reports, &anchor.anchor_id, |result| {
                result.structural_matched == Some(true)
            })
        })
        .count();
    let silent_loss = required_anchors.iter().any(|anchor| {
        !any_selected_anchor(&selected_reports, &anchor.anchor_id, |result| {
            result.projection_matched
        })
    });
    Ok(DecisionOutput {
        subset: subset.to_owned(),
        policy_id: policy_id.to_owned(),
        document_id: document_id.to_owned(),
        selected_candidate_id: decision
            .selected
            .as_ref()
            .map(|key| key.candidate_id.clone()),
        selected_profile: decision.selected.as_ref().map(|key| key.profile.clone()),
        supplemental_candidates: decision
            .supplemental
            .iter()
            .map(|key| key.candidate_id.clone())
            .collect(),
        selected_outcome: selected_score.map(|score| score.outcome.clone()),
        selected_quality_status: selected_quality.map(|quality| quality.status.clone()),
        fallback_used: decision.fallback_used,
        reason: decision.reason,
        required_anchor_count,
        projection_match_count,
        page_qualified_match_count,
        structurally_scored_anchor_count,
        structural_match_count,
        silent_loss,
        quality_classification: selected_quality.map(|quality| quality.classification.clone()),
    })
}

fn any_selected_anchor(
    reports: &[&ScoreReport],
    anchor_id: &str,
    predicate: impl Fn(&AnchorResult) -> bool,
) -> bool {
    reports.iter().any(|report| {
        report
            .anchor_results
            .iter()
            .any(|result| result.anchor_id == anchor_id && predicate(result))
    })
}

fn summarize(decisions: &[DecisionOutput]) -> Vec<PolicySummary> {
    let mut grouped = BTreeMap::<(String, String), PolicySummary>::new();
    for decision in decisions {
        let summary = grouped
            .entry((decision.subset.clone(), decision.policy_id.clone()))
            .or_insert_with(|| PolicySummary {
                subset: decision.subset.clone(),
                policy_id: decision.policy_id.clone(),
                ..PolicySummary::default()
            });
        summary.document_count += 1;
        summary.required_anchor_count += decision.required_anchor_count;
        summary.projection_match_count += decision.projection_match_count;
        summary.page_qualified_match_count += decision.page_qualified_match_count;
        summary.structurally_scored_anchor_count += decision.structurally_scored_anchor_count;
        summary.structural_match_count += decision.structural_match_count;
        if let Some(candidate) = &decision.selected_candidate_id {
            summary.selected_document_count += 1;
            *summary
                .selected_candidates
                .entry(candidate.clone())
                .or_default() += 1;
        }
        summary.fallback_document_count += usize::from(decision.fallback_used);
        summary.silent_loss_document_count += usize::from(decision.silent_loss);
        summary.false_good_document_count += usize::from(
            decision.selected_quality_status.as_deref() == Some("good") && decision.silent_loss,
        );
        summary.false_degraded_document_count += usize::from(
            decision.selected_quality_status.as_deref() == Some("degraded")
                && !decision.silent_loss,
        );
    }
    let mut summaries = grouped.into_values().collect::<Vec<_>>();
    for summary in &mut summaries {
        summary.projection_recall = ratio(
            summary.projection_match_count,
            summary.required_anchor_count,
        );
        summary.page_qualified_recall = ratio(
            summary.page_qualified_match_count,
            summary.required_anchor_count,
        );
        summary.structural_recall = (summary.structurally_scored_anchor_count > 0).then(|| {
            ratio(
                summary.structural_match_count,
                summary.structurally_scored_anchor_count,
            )
        });
    }
    summaries
}

fn summarize_role_strategies(
    decisions: &[RoleStrategyDecisionOutput],
    corpus: &CorpusData,
) -> Vec<RoleStrategySummary> {
    let mut grouped = BTreeMap::<(String, String), Vec<&RoleStrategyDecisionOutput>>::new();
    for decision in decisions {
        grouped
            .entry((decision.subset.clone(), decision.policy_id.clone()))
            .or_default()
            .push(decision);
    }
    grouped
        .into_iter()
        .map(|((subset, policy_id), rows)| {
            let required_anchor_count = rows
                .iter()
                .map(|row| row.required_anchor_count)
                .sum::<usize>();
            let projection_match_count = rows
                .iter()
                .map(|row| row.projection_match_count)
                .sum::<usize>();
            let page_qualified_match_count = rows
                .iter()
                .map(|row| row.page_qualified_match_count)
                .sum::<usize>();
            let structurally_scored_anchor_count = rows
                .iter()
                .map(|row| row.structurally_scored_anchor_count)
                .sum::<usize>();
            let structural_match_count = rows
                .iter()
                .map(|row| row.structural_match_count)
                .sum::<usize>();
            let mut stage_activation_counts = BTreeMap::new();
            let mut stage_quality_counts = BTreeMap::new();
            let mut activated_stages = BTreeSet::new();
            for row in &rows {
                for stage in &row.activated_stages {
                    *stage_activation_counts.entry(stage.clone()).or_default() += 1;
                    activated_stages.insert(stage.clone());
                }
                for (stage, status) in &row.stage_quality_states {
                    *stage_quality_counts
                        .entry(format!("{stage}:{status}"))
                        .or_default() += 1;
                }
            }
            let mut ordinary_cold = rows
                .iter()
                .filter(|row| row.timeout_class.as_deref() == Some("ordinary"))
                .filter_map(|row| row.cold_route_wall_time_ms)
                .collect::<Vec<_>>();
            let mut ordinary_warm = rows
                .iter()
                .filter(|row| row.timeout_class.as_deref() == Some("ordinary"))
                .filter_map(|row| row.warm_route_wall_time_ms)
                .collect::<Vec<_>>();
            let mut ordinary_cpu = rows
                .iter()
                .filter(|row| row.timeout_class.as_deref() == Some("ordinary"))
                .filter_map(|row| row.cold_route_cpu_time_ms)
                .collect::<Vec<_>>();
            ordinary_cold.sort_unstable();
            ordinary_warm.sort_unstable();
            ordinary_cpu.sort_unstable();
            let unique_adapter_binary_bytes = activated_stages
                .iter()
                .filter_map(|stage| operation_for_stage(corpus, stage))
                .map(|operation| operation.adapter_binary_bytes)
                .sum();
            let unique_prepared_asset_bytes = activated_stages
                .iter()
                .filter_map(|stage| operation_for_stage(corpus, stage))
                .map(|operation| operation.asset_logical_bytes)
                .sum();
            RoleStrategySummary {
                subset,
                policy_id,
                document_count: rows.len(),
                required_anchor_count,
                projection_match_count,
                page_qualified_match_count,
                structurally_scored_anchor_count,
                structural_match_count,
                projection_recall: ratio(projection_match_count, required_anchor_count),
                page_qualified_recall: ratio(page_qualified_match_count, required_anchor_count),
                structural_recall: (structurally_scored_anchor_count > 0)
                    .then(|| ratio(structural_match_count, structurally_scored_anchor_count)),
                silent_loss_document_count: rows.iter().filter(|row| row.silent_loss).count(),
                primary_false_good_document_count: rows
                    .iter()
                    .filter(|row| {
                        row.silent_loss && row.primary_quality_status.as_deref() == Some("good")
                    })
                    .count(),
                zotero_protected_anchor_count: rows
                    .iter()
                    .map(|row| row.zotero_protected_anchor_count)
                    .sum(),
                ocr_recovered_anchor_count: rows
                    .iter()
                    .map(|row| row.ocr_recovered_anchor_count)
                    .sum(),
                structure_gained_anchor_count: rows
                    .iter()
                    .map(|row| row.structure_gained_anchor_count)
                    .sum(),
                stage_activation_counts,
                stage_quality_counts,
                ordinary_document_count: ordinary_cold.len(),
                ordinary_cold_median_ms: percentile(&ordinary_cold, 0.5),
                ordinary_cold_p90_ms: percentile(&ordinary_cold, 0.9),
                ordinary_cold_worst_ms: ordinary_cold.last().copied(),
                ordinary_warm_median_ms: percentile(&ordinary_warm, 0.5),
                ordinary_warm_p90_ms: percentile(&ordinary_warm, 0.9),
                ordinary_warm_worst_ms: ordinary_warm.last().copied(),
                ordinary_cold_cpu_median_ms: percentile(&ordinary_cpu, 0.5),
                peak_sequential_process_tree_rss_bytes: rows
                    .iter()
                    .filter_map(|row| row.peak_sequential_process_tree_rss_bytes)
                    .max(),
                maximum_stage_thread_count: rows
                    .iter()
                    .filter_map(|row| row.maximum_stage_thread_count)
                    .max(),
                unique_adapter_binary_bytes,
                unique_prepared_asset_bytes,
                timeout_document_count: rows.iter().filter(|row| row.route_timed_out).count(),
            }
        })
        .collect()
}

fn operation_for_stage<'a>(corpus: &'a CorpusData, stage: &str) -> Option<&'a OperationalReport> {
    corpus
        .operations
        .iter()
        .find(|(key, _)| attempt_label(key) == stage)
        .map(|(_, operation)| operation)
}

fn percentile(values: &[u64], quantile: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let index = ((values.len() as f64 * quantile).ceil() as usize)
        .saturating_sub(1)
        .min(values.len() - 1);
    values.get(index).copied()
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn policy_definitions() -> Vec<PolicyDefinition> {
    vec![
        PolicyDefinition {
            id: "single-provider:<candidate>".to_owned(),
            description: "Use the candidate recommended-auto result when it is usable."
                .to_owned(),
            observable_inputs: vec!["outcome".to_owned(), "quality usability".to_owned()],
        },
        PolicyDefinition {
            id: "structured-plus-zotero:<candidate>".to_owned(),
            description: "Keep usable structured output and retain available Zotero text as an active searchable safety net; use Zotero alone when the primary is unusable."
                .to_owned(),
            observable_inputs: vec![
                "outcome".to_owned(),
                "quality usability".to_owned(),
                "Zotero availability".to_owned(),
            ],
        },
        PolicyDefinition {
            id: "lightweight-to-heavy:pdf-inspector-to-docling".to_owned(),
            description: "Accept good PDF Inspector output; otherwise choose Docling only when explicit quality and completeness fields make it preferable."
                .to_owned(),
            observable_inputs: vec![
                "outcome".to_owned(),
                "quality status".to_owned(),
                "quality usability".to_owned(),
                "truncation signal".to_owned(),
                "page coverage".to_owned(),
                "replacement characters".to_owned(),
                "candidate token coverage".to_owned(),
            ],
        },
        PolicyDefinition {
            id: "comparative-safeguard:<candidate>".to_owned(),
            description: "Retain Zotero beside the structured primary when frozen quality or comparison signals warn; use Zotero alone when the primary is unusable."
                .to_owned(),
            observable_inputs: vec![
                "outcome".to_owned(),
                "quality status and usability".to_owned(),
                "gross coverage warning".to_owned(),
                "truncation signal".to_owned(),
                "page coverage".to_owned(),
                "replacement characters".to_owned(),
                "empty or suspicious pages".to_owned(),
                "candidate/Zotero character ratio for reporting".to_owned(),
            ],
        },
        PolicyDefinition {
            id: "role-based-content-v1".to_owned(),
            description: "Run Xberg native as the low-cost primary, retain usable Zotero text in parallel, and invoke PDF Inspector recommended-auto only when Xberg native output has observable empty, missing, sparse, or unusable content.".to_owned(),
            observable_inputs: vec![
                "Xberg native outcome and usability".to_owned(),
                "Xberg native token count".to_owned(),
                "Xberg native page coverage".to_owned(),
                "Xberg native empty and suspicious pages".to_owned(),
                "Zotero availability".to_owned(),
            ],
        },
        PolicyDefinition {
            id: "role-based-structure-demand-v1".to_owned(),
            description: "Apply the content route and additionally run Docling automatic extraction only in an explicit structure-demand workflow; no document property or anchor silently requests this stage.".to_owned(),
            observable_inputs: vec![
                "all role-based-content-v1 inputs".to_owned(),
                "explicit user structure-demand request".to_owned(),
            ],
        },
    ]
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "output has no parent directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = temporary_path(path);
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes)
        .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| {
        format!(
            "cannot publish {} as {}: {error}",
            temporary.display(),
            path.display()
        )
    })
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observable(candidate: &str, status: &str, usable: bool) -> ObservableAttempt {
        ObservableAttempt {
            key: AttemptKey::new("paper", candidate, AUTO_PROFILE),
            outcome: "succeeded".to_owned(),
            quality_status: status.to_owned(),
            quality_usable: usable,
            page_coverage_ratio: Some(1.0),
            baseline_character_ratio: Some(1.0),
            gross_coverage_warning: false,
            truncation_suspected: false,
            replacement_character_count: 0,
            empty_page_count: 0,
            suspicious_page_count: 0,
            token_count: 100,
        }
    }

    fn corpus_with(attempts: Vec<ObservableAttempt>) -> CorpusData {
        CorpusData {
            scores: BTreeMap::new(),
            qualities: BTreeMap::new(),
            observables: attempts
                .into_iter()
                .map(|attempt| (attempt.key.clone(), attempt))
                .collect(),
            operations: BTreeMap::new(),
            documents: BTreeSet::from(["paper".to_owned()]),
        }
    }

    #[test]
    fn safety_net_keeps_primary_and_retains_available_zotero() {
        let primary = observable("xberg", "degraded", true);
        let mut zotero = observable("zotero", "degraded", true);
        zotero.key.profile = ZOTERO_PROFILE.to_owned();
        let corpus = corpus_with(vec![primary, zotero]);
        let decision = select_zotero_safety_net(&corpus, "paper", "xberg");
        assert_eq!(decision.selected.expect("selection").candidate_id, "xberg");
        assert_eq!(decision.supplemental[0].candidate_id, "zotero");
        assert!(decision.fallback_used);
    }

    #[test]
    fn tiered_policy_uses_heavy_output_when_lightweight_is_not_good() {
        let light = observable("pdf-inspector", "degraded", true);
        let heavy = observable("docling", "good", true);
        let corpus = corpus_with(vec![light, heavy]);
        let decision = select_tiered(&corpus, "paper");
        assert_eq!(
            decision.selected.expect("selection").candidate_id,
            "docling"
        );
        assert!(decision.fallback_used);
    }

    #[test]
    fn comparative_policy_retains_zotero_for_a_frozen_coverage_warning() {
        let mut primary = observable("xberg", "good", true);
        primary.gross_coverage_warning = true;
        primary.baseline_character_ratio = Some(0.5);
        let mut zotero = observable("zotero", "degraded", true);
        zotero.key.profile = ZOTERO_PROFILE.to_owned();
        let corpus = corpus_with(vec![primary, zotero]);
        let decision = select_comparative(&corpus, "paper", "xberg");
        assert_eq!(decision.selected.expect("selection").candidate_id, "xberg");
        assert_eq!(decision.supplemental[0].candidate_id, "zotero");
        assert!(decision.fallback_used);
    }

    #[test]
    fn decision_function_has_no_anchor_input() {
        let primary = observable("xberg", "good", true);
        let corpus = corpus_with(vec![primary]);
        let decision = select_single(&corpus, "paper", "xberg");
        assert_eq!(decision.selected.expect("selection").candidate_id, "xberg");
    }

    #[test]
    fn role_ocr_trigger_uses_only_native_extraction_observables() {
        let clean = observable("xberg", "good", true);
        assert_eq!(ocr_fallback_reason(&clean), None);
        let mut empty_page = clean.clone();
        empty_page.empty_page_count = 1;
        assert_eq!(
            ocr_fallback_reason(&empty_page).as_deref(),
            Some("Xberg native output contained empty pages")
        );
    }

    #[test]
    fn role_selection_accepts_only_observable_attempts() {
        let mut primary = observable("xberg", "good", true);
        primary.key.profile = "native".to_owned();
        let mut zotero = observable("zotero", "degraded", true);
        zotero.key.profile = ZOTERO_PROFILE.to_owned();
        let observables = [primary, zotero]
            .into_iter()
            .map(|attempt| (attempt.key.clone(), attempt))
            .collect::<BTreeMap<_, _>>();
        let selection = select_role_strategy("paper", false, &observables);
        assert_eq!(
            selection.accepted,
            vec![
                AttemptKey::new("paper", "xberg", "native"),
                AttemptKey::new("paper", "zotero", ZOTERO_PROFILE),
            ]
        );
        assert!(!selection.ocr_fallback_activated);
        assert!(selection.zotero_retained);
    }

    #[test]
    fn route_timeout_class_uses_the_most_restrictive_activated_stage() {
        let phase = OperationalPhase {
            outcome: "succeeded".to_owned(),
            wall_time_ms: 1,
            user_cpu_time_ms: Some(1),
            system_cpu_time_ms: Some(0),
            peak_process_tree_rss_bytes: Some(1),
            peak_process_tree_thread_count: Some(1),
        };
        let ordinary = OperationalReport {
            document_id: "paper".to_owned(),
            candidate_id: "xberg".to_owned(),
            profile: "native".to_owned(),
            timeout_class: "ordinary".to_owned(),
            adapter_binary_bytes: 1,
            asset_logical_bytes: 1,
            cold_start: phase.clone(),
            warm_extraction: Some(phase.clone()),
        };
        let mut scanned = ordinary.clone();
        scanned.timeout_class = "scanned-ocr".to_owned();
        assert_eq!(
            route_timeout_class(&[&ordinary, &scanned]).as_deref(),
            Some("scanned-ocr")
        );
    }
}
