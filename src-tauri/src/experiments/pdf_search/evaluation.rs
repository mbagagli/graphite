use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use serde::Deserialize;

use crate::error::{AppError, AppResult};

use super::model::sha256;

const EVALUATION_SET_ID: &str = "m6-c2-retrieval-qa-v1";
const SUGGESTED: &str = "suggested-unverified";
const VERIFIED: &str = "human-verified";
const REJECTED: &str = "human-rejected";
const PRIMARY_ITEM_QUERY_COUNT: usize = 20;
const PRIMARY_CORPUS_QUERY_COUNT: usize = 20;
const OCR_ADJUNCT_QUERY_COUNT: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluationBundleSummary {
    pub query_count: usize,
    pub primary_item_query_count: usize,
    pub primary_corpus_query_count: usize,
    pub ocr_adjunct_query_count: usize,
    pub qa_count: usize,
    pub suggested_entry_count: usize,
    pub human_verified_entry_count: usize,
    pub human_rejected_entry_count: usize,
    pub experiment_config_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrievalQueryManifest {
    schema_version: u32,
    evaluation_set_id: String,
    corpus_id: String,
    queries: Vec<RetrievalQuery>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrievalQuery {
    query_id: String,
    query_text: String,
    scope: QueryScope,
    item_ids: Vec<String>,
    query_category: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum QueryScope {
    Item,
    Corpus,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluationKey {
    schema_version: u32,
    evaluation_set_id: String,
    entries: Vec<EvaluationEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluationEntry {
    query_id: String,
    verification_status: String,
    relevant_anchor_ids: Vec<String>,
    relevant_item_ids: Vec<String>,
    relevant_document_ids: Vec<String>,
    acceptable_deterministic_alternatives: Vec<String>,
    required_evidence: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QaManifest {
    schema_version: u32,
    evaluation_set_id: String,
    entries: Vec<QaEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QaEntry {
    qa_id: String,
    query_id: String,
    question: String,
    verification_status: String,
    expected_answer: String,
    acceptable_deterministic_alternatives: Vec<String>,
    required_anchor_ids: Vec<String>,
    required_evidence: Vec<String>,
    acceptable_document_ids: Vec<String>,
    numeric_values: Vec<NumericExpectation>,
    abstention_acceptable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NumericExpectation {
    value: String,
    unit: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DoclingSubset {
    schema_version: u32,
    subset_id: String,
    profile: String,
    provider_profile: String,
    selection_rule: String,
    maximum_documents: usize,
    documents: Vec<DoclingDocument>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DoclingDocument {
    document_id: String,
    frozen_categories: Vec<String>,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct CorpusManifest {
    #[serde(rename = "corpusId")]
    corpus_id: String,
    documents: Vec<CorpusDocument>,
}

#[derive(Debug, Deserialize)]
struct CorpusDocument {
    id: String,
    categories: Vec<String>,
    anchors: String,
}

#[derive(Debug, Deserialize)]
struct AnchorFile {
    #[serde(rename = "documentId")]
    document_id: String,
    anchors: Vec<Anchor>,
}

#[derive(Debug, Deserialize)]
struct Anchor {
    id: String,
    #[serde(rename = "verificationStatus")]
    verification_status: String,
}

#[derive(Debug, Deserialize)]
struct ExperimentConfig {
    schema_version: u32,
    experiment_config_id: String,
    evaluation_set_id: String,
    query_scope_id_namespace: String,
    review_state: String,
    scored_run_allowed: bool,
    primary_corpus: PrimaryCorpus,
    query_partitions: QueryPartitions,
    modes: Vec<String>,
    candidate_profiles: BTreeMap<String, String>,
    evaluation_rules: EvaluationRules,
    frozen_contracts: serde_json::Value,
    ocr_policy: OcrPolicy,
    input_files: BTreeMap<String, LockedInput>,
}

#[derive(Debug, Deserialize)]
struct PrimaryCorpus {
    corpus_id: String,
    document_ids: Vec<String>,
    manifest_path: String,
    manifest_sha256: String,
}

#[derive(Debug, Deserialize)]
struct QueryPartitions {
    primary_item_scoped: Vec<String>,
    primary_corpus_wide: Vec<String>,
    ocr_adjunct: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OcrPolicy {
    automatic_route: String,
    human_anchors_may_trigger_ocr: bool,
    adjunct: OcrAdjunct,
    manual_deep_read_counterfactual: ManualDeepRead,
}

#[derive(Debug, Deserialize)]
struct EvaluationRules {
    retrieval_ground_truth: String,
    qa_ground_truth: String,
    llm_judge_allowed_as_sole_ground_truth: bool,
    ground_truth_available_to_retrieval_or_routing: bool,
    retrieval_outputs_sealed_before_key_loading: bool,
    retrieval_tuning_during_scored_run: bool,
}

#[derive(Debug, Deserialize)]
struct OcrAdjunct {
    document_id: String,
    aggregate: String,
    mode: String,
    smoke_manifest_path: String,
    smoke_manifest_sha256: String,
}

#[derive(Debug, Deserialize)]
struct ManualDeepRead {
    document_id: String,
    mode: String,
    included_in_automatic_policy_aggregates: bool,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct LockedInput {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChecksumManifest {
    schema_version: u32,
    evaluation_set_id: String,
    hash_algorithm: String,
    hash_input: String,
    files: Vec<ChecksumEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChecksumEntry {
    path: String,
    sha256: String,
}

pub fn validate_tracked_evaluation_bundle(repo_root: &Path) -> AppResult<EvaluationBundleSummary> {
    let evaluation_root = repo_root.join("evaluation/pdf-search-v1");
    let query_manifest: RetrievalQueryManifest = read_json(
        &evaluation_root.join("retrieval-queries-v1.json"),
        "retrieval query manifest",
    )?;
    let evaluation_key: EvaluationKey = read_json(
        &evaluation_root.join("retrieval-evaluation-key-v1.json"),
        "retrieval evaluation key",
    )?;
    let qa_manifest: QaManifest =
        read_json(&evaluation_root.join("qa-manifest-v1.json"), "QA manifest")?;
    let docling_subset: DoclingSubset = read_json(
        &evaluation_root.join("docling-subset-v1.json"),
        "Docling subset",
    )?;
    let config_path = evaluation_root.join("experiment-config-v1.json");
    let experiment_config: ExperimentConfig = read_json(&config_path, "experiment configuration")?;
    let checksums: ChecksumManifest = read_json(
        &evaluation_root.join("checksums-v1.json"),
        "evaluation checksum manifest",
    )?;
    validate_version_and_set_ids(
        &query_manifest,
        &evaluation_key,
        &qa_manifest,
        &experiment_config,
        &checksums,
    )?;
    let representative: CorpusManifest = read_json(
        &repo_root.join("tools/pdf-benchmark/corpus/representative-manifest.json"),
        "representative corpus manifest",
    )?;
    let smoke: CorpusManifest = read_json(
        &repo_root.join("tools/pdf-benchmark/corpus/smoke-manifest.json"),
        "smoke corpus manifest",
    )?;
    let corpus = CorpusEvidence::load(repo_root, &representative, &smoke)?;
    let counts = validate_queries(&query_manifest, &corpus)?;
    let query_review = validate_evaluation_key(&evaluation_key, &query_manifest, &corpus)?;
    let qa_review = validate_qa(&qa_manifest, &evaluation_key, &query_manifest, &corpus)?;
    validate_docling_subset(&docling_subset, &corpus)?;
    validate_experiment_config(
        repo_root,
        &experiment_config,
        &query_manifest,
        &representative,
        query_review.suggested + qa_review.suggested,
    )?;
    validate_checksums(repo_root, &checksums)?;
    let experiment_config_sha256 = sha256(&read_file(&config_path, "experiment config")?);
    Ok(EvaluationBundleSummary {
        query_count: query_manifest.queries.len(),
        primary_item_query_count: counts.primary_item,
        primary_corpus_query_count: counts.primary_corpus,
        ocr_adjunct_query_count: counts.ocr_adjunct,
        qa_count: qa_manifest.entries.len(),
        suggested_entry_count: query_review.suggested + qa_review.suggested,
        human_verified_entry_count: query_review.verified + qa_review.verified,
        human_rejected_entry_count: query_review.rejected + qa_review.rejected,
        experiment_config_sha256,
    })
}

struct CorpusEvidence {
    primary_documents: BTreeSet<String>,
    all_documents: BTreeSet<String>,
    categories: BTreeMap<String, BTreeSet<String>>,
    verified_anchors: BTreeMap<String, String>,
}

impl CorpusEvidence {
    fn load(
        repo_root: &Path,
        representative: &CorpusManifest,
        smoke: &CorpusManifest,
    ) -> AppResult<Self> {
        let mut primary_documents = BTreeSet::new();
        let mut all_documents = BTreeSet::new();
        let mut categories = BTreeMap::new();
        let mut verified_anchors = BTreeMap::new();
        for (manifest, primary) in [(representative, true), (smoke, false)] {
            for document in &manifest.documents {
                if primary {
                    primary_documents.insert(document.id.clone());
                }
                all_documents.insert(document.id.clone());
                categories.insert(
                    document.id.clone(),
                    document.categories.iter().cloned().collect(),
                );
                let anchor_path = repo_root
                    .join("tools/pdf-benchmark/corpus")
                    .join(&document.anchors);
                let anchors: AnchorFile = read_json(&anchor_path, "anchor file")?;
                if anchors.document_id != document.id {
                    return invalid(format!(
                        "Anchor file document {} does not match {}",
                        anchors.document_id, document.id
                    ));
                }
                for anchor in anchors.anchors {
                    if anchor.verification_status == "human-verified"
                        && verified_anchors
                            .insert(anchor.id.clone(), document.id.clone())
                            .is_some()
                    {
                        return invalid(format!("Duplicate verified anchor ID {}", anchor.id));
                    }
                }
            }
        }
        Ok(Self {
            primary_documents,
            all_documents,
            categories,
            verified_anchors,
        })
    }
}

struct QueryCounts {
    primary_item: usize,
    primary_corpus: usize,
    ocr_adjunct: usize,
}

fn validate_queries(
    manifest: &RetrievalQueryManifest,
    corpus: &CorpusEvidence,
) -> AppResult<QueryCounts> {
    let mut ids = BTreeSet::new();
    let mut primary_item = 0;
    let mut primary_corpus = 0;
    let mut ocr_adjunct = 0;
    for query in &manifest.queries {
        require_nonempty(&query.query_id, "query ID")?;
        require_nonempty(&query.query_text, "query text")?;
        require_nonempty(&query.query_category, "query category")?;
        if !ids.insert(query.query_id.clone()) {
            return invalid(format!("Duplicate query ID {}", query.query_id));
        }
        match query.scope {
            QueryScope::Item => {
                if query.item_ids.len() != 1 {
                    return invalid(format!(
                        "Item-scoped query {} must name exactly one item",
                        query.query_id
                    ));
                }
                let item_id = &query.item_ids[0];
                if query.query_category == "ocr_adjunct" {
                    if item_id != "paper05" {
                        return invalid("Only paper05 may be an OCR-adjunct query");
                    }
                    ocr_adjunct += 1;
                } else {
                    if !corpus.primary_documents.contains(item_id) {
                        return invalid(format!(
                            "Primary query {} references non-primary item {item_id}",
                            query.query_id
                        ));
                    }
                    primary_item += 1;
                }
            }
            QueryScope::Corpus => {
                if !query.item_ids.is_empty() {
                    return invalid(format!(
                        "Corpus query {} must not contain item IDs",
                        query.query_id
                    ));
                }
                if query.query_category == "ocr_adjunct" {
                    return invalid("OCR-adjunct queries must be item scoped");
                }
                primary_corpus += 1;
            }
        }
    }
    if primary_item != PRIMARY_ITEM_QUERY_COUNT
        || primary_corpus != PRIMARY_CORPUS_QUERY_COUNT
        || ocr_adjunct != OCR_ADJUNCT_QUERY_COUNT
    {
        return invalid(format!(
            "Query partition counts are {primary_item}/{primary_corpus}/{ocr_adjunct}, expected {PRIMARY_ITEM_QUERY_COUNT}/{PRIMARY_CORPUS_QUERY_COUNT}/{OCR_ADJUNCT_QUERY_COUNT}"
        ));
    }
    Ok(QueryCounts {
        primary_item,
        primary_corpus,
        ocr_adjunct,
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct ReviewCounts {
    suggested: usize,
    verified: usize,
    rejected: usize,
}

fn validate_evaluation_key(
    key: &EvaluationKey,
    queries: &RetrievalQueryManifest,
    corpus: &CorpusEvidence,
) -> AppResult<ReviewCounts> {
    let query_ids = queries
        .queries
        .iter()
        .map(|query| query.query_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut key_ids = BTreeSet::new();
    let mut review = ReviewCounts::default();
    for entry in &key.entries {
        review.add(&entry.verification_status, &entry.query_id)?;
        if !key_ids.insert(entry.query_id.as_str()) {
            return invalid(format!("Duplicate evaluation query ID {}", entry.query_id));
        }
        require_nonempty_list(&entry.relevant_anchor_ids, "relevant anchors")?;
        require_nonempty_list(&entry.relevant_item_ids, "relevant items")?;
        require_nonempty_list(&entry.relevant_document_ids, "relevant documents")?;
        require_nonempty_list(
            &entry.acceptable_deterministic_alternatives,
            "deterministic alternatives",
        )?;
        require_nonempty_list(&entry.required_evidence, "required evidence")?;
        for document in entry
            .relevant_item_ids
            .iter()
            .chain(&entry.relevant_document_ids)
        {
            if !corpus.all_documents.contains(document) {
                return invalid(format!(
                    "Evaluation entry {} references unknown document {document}",
                    entry.query_id
                ));
            }
        }
        for anchor in &entry.relevant_anchor_ids {
            let document = corpus.verified_anchors.get(anchor).ok_or_else(|| {
                AppError::ApplicationData(format!(
                    "Evaluation anchor {anchor} is not human verified"
                ))
            })?;
            if !entry.relevant_document_ids.contains(document) {
                return invalid(format!(
                    "Evaluation anchor {anchor} is not associated with its document {document}"
                ));
            }
        }
    }
    if query_ids != key_ids {
        return invalid("The query manifest and evaluation key IDs differ");
    }
    Ok(review)
}

fn validate_qa(
    qa: &QaManifest,
    key: &EvaluationKey,
    queries: &RetrievalQueryManifest,
    corpus: &CorpusEvidence,
) -> AppResult<ReviewCounts> {
    if !(15..=20).contains(&qa.entries.len()) {
        return invalid("The QA manifest must contain between 15 and 20 entries");
    }
    let query_ids = queries
        .queries
        .iter()
        .map(|query| query.query_id.as_str())
        .collect::<BTreeSet<_>>();
    let key_by_query = key
        .entries
        .iter()
        .map(|entry| (entry.query_id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut qa_ids = BTreeSet::new();
    let mut qa_queries = BTreeSet::new();
    let mut review = ReviewCounts::default();
    for entry in &qa.entries {
        review.add(&entry.verification_status, &entry.qa_id)?;
        if !qa_ids.insert(entry.qa_id.as_str()) {
            return invalid(format!("Duplicate QA ID {}", entry.qa_id));
        }
        if !qa_queries.insert(entry.query_id.as_str()) {
            return invalid(format!("Duplicate QA query ID {}", entry.query_id));
        }
        if !query_ids.contains(entry.query_id.as_str()) {
            return invalid(format!("QA entry {} has an unknown query", entry.qa_id));
        }
        require_nonempty(&entry.question, "QA question")?;
        require_nonempty(&entry.expected_answer, "QA expected answer")?;
        require_nonempty_list(
            &entry.acceptable_deterministic_alternatives,
            "QA deterministic alternatives",
        )?;
        require_nonempty_list(&entry.required_anchor_ids, "QA required anchors")?;
        require_nonempty_list(&entry.required_evidence, "QA required evidence")?;
        require_nonempty_list(&entry.acceptable_document_ids, "QA documents")?;
        let evaluation = key_by_query[entry.query_id.as_str()];
        if entry.verification_status == VERIFIED && evaluation.verification_status != VERIFIED {
            return invalid(format!(
                "Verified QA entry {} requires a verified retrieval query",
                entry.qa_id
            ));
        }
        for anchor in &entry.required_anchor_ids {
            if !evaluation.relevant_anchor_ids.contains(anchor)
                || !corpus.verified_anchors.contains_key(anchor)
            {
                return invalid(format!(
                    "QA entry {} uses an anchor outside its verified evaluation key",
                    entry.qa_id
                ));
            }
        }
        for document in &entry.acceptable_document_ids {
            if !evaluation.relevant_document_ids.contains(document) {
                return invalid(format!(
                    "QA entry {} uses a document outside its evaluation key",
                    entry.qa_id
                ));
            }
        }
        for numeric in &entry.numeric_values {
            require_nonempty(&numeric.value, "QA numeric value")?;
            if let Some(unit) = &numeric.unit {
                require_nonempty(unit, "QA numeric unit")?;
            }
        }
        let _ = entry.abstention_acceptable;
    }
    Ok(review)
}

impl ReviewCounts {
    fn add(&mut self, status: &str, identity: &str) -> AppResult<()> {
        match status {
            SUGGESTED => self.suggested += 1,
            VERIFIED => self.verified += 1,
            REJECTED => self.rejected += 1,
            _ => {
                return invalid(format!(
                    "Entry {identity} has invalid review status {status}"
                ))
            }
        }
        Ok(())
    }
}

fn validate_docling_subset(subset: &DoclingSubset, corpus: &CorpusEvidence) -> AppResult<()> {
    if subset.schema_version != 1
        || subset.subset_id != "m6-c2-docling-flat-v1"
        || subset.profile != "experimental-docling-flat"
        || subset.provider_profile.is_empty()
        || subset.selection_rule.is_empty()
    {
        return invalid("The Docling subset header is invalid");
    }
    if subset.maximum_documents > 3
        || subset.documents.is_empty()
        || subset.documents.len() > subset.maximum_documents
    {
        return invalid("The Docling subset exceeds its three-document limit");
    }
    let mut documents = BTreeSet::new();
    for document in &subset.documents {
        if !documents.insert(document.document_id.as_str()) {
            return invalid(format!(
                "Duplicate Docling document {}",
                document.document_id
            ));
        }
        if !corpus.primary_documents.contains(&document.document_id) {
            return invalid(format!(
                "Docling document {} is outside the primary corpus",
                document.document_id
            ));
        }
        let expected = &corpus.categories[&document.document_id];
        let actual = document
            .frozen_categories
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if &actual != expected {
            return invalid(format!(
                "Docling categories changed for {}",
                document.document_id
            ));
        }
        require_nonempty(&document.reason, "Docling selection reason")?;
    }
    Ok(())
}

fn validate_experiment_config(
    repo_root: &Path,
    config: &ExperimentConfig,
    queries: &RetrievalQueryManifest,
    representative: &CorpusManifest,
    suggested_entry_count: usize,
) -> AppResult<()> {
    if config.schema_version != 1
        || config.experiment_config_id != "m6-c2-controlled-search-v1"
        || config.evaluation_set_id != EVALUATION_SET_ID
        || config.query_scope_id_namespace != "neutral-corpus-document-id"
    {
        return invalid("The experiment configuration header is invalid");
    }
    let expected_review_state = if suggested_entry_count == 0 {
        "human-reviewed"
    } else {
        SUGGESTED
    };
    if config.review_state != expected_review_state
        || config.scored_run_allowed != (suggested_entry_count == 0)
    {
        return invalid("The experiment configuration does not match its review gate");
    }
    if config.primary_corpus.corpus_id != representative.corpus_id
        || config.primary_corpus.document_ids
            != representative
                .documents
                .iter()
                .map(|document| document.id.clone())
                .collect::<Vec<_>>()
    {
        return invalid("The experiment primary corpus differs from the representative manifest");
    }
    validate_locked_path(
        repo_root,
        &config.primary_corpus.manifest_path,
        &config.primary_corpus.manifest_sha256,
    )?;
    let partition_ids = config
        .query_partitions
        .primary_item_scoped
        .iter()
        .chain(&config.query_partitions.primary_corpus_wide)
        .chain(&config.query_partitions.ocr_adjunct)
        .cloned()
        .collect::<BTreeSet<_>>();
    let query_ids = queries
        .queries
        .iter()
        .map(|query| query.query_id.clone())
        .collect::<BTreeSet<_>>();
    if partition_ids != query_ids
        || config.query_partitions.primary_item_scoped.len() != PRIMARY_ITEM_QUERY_COUNT
        || config.query_partitions.primary_corpus_wide.len() != PRIMARY_CORPUS_QUERY_COUNT
        || config.query_partitions.ocr_adjunct.len() != OCR_ADJUNCT_QUERY_COUNT
    {
        return invalid("The experiment query partitions differ from the query manifest");
    }
    let required_modes = [
        "production-zotero",
        "experimental-xberg",
        "experimental-zotero-plus-xberg",
        "experimental-zotero-plus-xberg-ocr",
        "experimental-docling-flat",
        "experimental-manual-deep-read-ocr",
    ];
    if config.modes != required_modes {
        return invalid("The experiment mode set is not frozen as specified");
    }
    if config.candidate_profiles.len() != 3 {
        return invalid("The experiment must pin exactly three candidate profiles");
    }
    if config
        .evaluation_rules
        .llm_judge_allowed_as_sole_ground_truth
        || config
            .evaluation_rules
            .ground_truth_available_to_retrieval_or_routing
        || !config
            .evaluation_rules
            .retrieval_outputs_sealed_before_key_loading
        || config.evaluation_rules.retrieval_tuning_during_scored_run
    {
        return invalid("The evaluation result-sealing rules are invalid");
    }
    require_nonempty(
        &config.evaluation_rules.retrieval_ground_truth,
        "retrieval ground truth",
    )?;
    require_nonempty(&config.evaluation_rules.qa_ground_truth, "QA ground truth")?;
    if config.ocr_policy.human_anchors_may_trigger_ocr
        || config.ocr_policy.adjunct.document_id != "paper05"
        || config.ocr_policy.adjunct.aggregate != "ocr-adjunct-only"
        || config.ocr_policy.adjunct.mode != "experimental-zotero-plus-xberg-ocr"
        || config
            .ocr_policy
            .manual_deep_read_counterfactual
            .document_id
            != "paper30"
        || config.ocr_policy.manual_deep_read_counterfactual.mode
            != "experimental-manual-deep-read-ocr"
        || config
            .ocr_policy
            .manual_deep_read_counterfactual
            .included_in_automatic_policy_aggregates
    {
        return invalid("The frozen OCR and manual deep-read policy is invalid");
    }
    require_nonempty(&config.ocr_policy.automatic_route, "automatic OCR route")?;
    require_nonempty(
        &config.ocr_policy.manual_deep_read_counterfactual.reason,
        "manual deep-read reason",
    )?;
    validate_locked_path(
        repo_root,
        &config.ocr_policy.adjunct.smoke_manifest_path,
        &config.ocr_policy.adjunct.smoke_manifest_sha256,
    )?;
    for input in config.input_files.values() {
        validate_locked_path(repo_root, &input.path, &input.sha256)?;
    }
    validate_embedded_path_hashes(repo_root, &config.frozen_contracts)?;
    Ok(())
}

fn validate_checksums(repo_root: &Path, manifest: &ChecksumManifest) -> AppResult<()> {
    if manifest.schema_version != 1
        || manifest.evaluation_set_id != EVALUATION_SET_ID
        || manifest.hash_algorithm != "sha256"
        || manifest.hash_input != "exact raw file bytes"
        || manifest.files.len() != 5
    {
        return invalid("The evaluation checksum manifest header is invalid");
    }
    let mut paths = BTreeSet::new();
    for entry in &manifest.files {
        if !paths.insert(entry.path.as_str()) {
            return invalid(format!("Duplicate checksum path {}", entry.path));
        }
        validate_locked_path(repo_root, &entry.path, &entry.sha256)?;
    }
    Ok(())
}

fn validate_version_and_set_ids(
    queries: &RetrievalQueryManifest,
    key: &EvaluationKey,
    qa: &QaManifest,
    config: &ExperimentConfig,
    checksums: &ChecksumManifest,
) -> AppResult<()> {
    if queries.schema_version != 1
        || key.schema_version != 1
        || qa.schema_version != 1
        || config.schema_version != 1
        || checksums.schema_version != 1
    {
        return invalid("An evaluation input has an unsupported schema version");
    }
    for value in [
        &queries.evaluation_set_id,
        &key.evaluation_set_id,
        &qa.evaluation_set_id,
        &config.evaluation_set_id,
        &checksums.evaluation_set_id,
    ] {
        if value != EVALUATION_SET_ID {
            return invalid("Evaluation set IDs do not agree");
        }
    }
    require_nonempty(&queries.corpus_id, "query corpus ID")
}

fn validate_embedded_path_hashes(repo_root: &Path, value: &serde_json::Value) -> AppResult<()> {
    match value {
        serde_json::Value::Object(object) => {
            for (key, path) in object {
                let Some(path) = path.as_str() else {
                    continue;
                };
                let sha_key = if key == "path" {
                    Some("sha256".to_owned())
                } else {
                    key.strip_suffix("_path")
                        .map(|prefix| format!("{prefix}_sha256"))
                };
                if let Some(sha_key) = sha_key {
                    let expected = object.get(&sha_key).and_then(serde_json::Value::as_str);
                    if let Some(expected) = expected {
                        validate_locked_path(repo_root, path, expected)?;
                    }
                }
            }
            for child in object.values() {
                validate_embedded_path_hashes(repo_root, child)?;
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                validate_embedded_path_hashes(repo_root, child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_locked_path(repo_root: &Path, relative: &str, expected: &str) -> AppResult<()> {
    if !is_sha256(expected) {
        return invalid(format!(
            "Locked hash for {relative} is not a complete SHA-256"
        ));
    }
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return invalid(format!("Locked path {relative} is not repository relative"));
    }
    let bytes = read_file(&repo_root.join(relative_path), "locked evaluation input")?;
    let actual = sha256(&bytes);
    if actual != expected {
        return invalid(format!("Checksum mismatch for {relative}"));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn require_nonempty(value: &str, label: &str) -> AppResult<()> {
    if value.trim().is_empty() {
        return invalid(format!("{label} must not be empty"));
    }
    Ok(())
}

fn require_nonempty_list(values: &[String], label: &str) -> AppResult<()> {
    if values.is_empty() || values.iter().any(|value| value.trim().is_empty()) {
        return invalid(format!("{label} must contain non-empty values"));
    }
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path, label: &str) -> AppResult<T> {
    let bytes = read_file(path, label)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AppError::ApplicationData(format!("Invalid {label}: {error}")))
}

fn read_file(path: &Path, label: &str) -> AppResult<Vec<u8>> {
    fs::read(path).map_err(|error| {
        AppError::ApplicationData(format!("The {label} could not be read: {error}"))
    })
}

fn invalid<T>(message: impl Into<String>) -> AppResult<T> {
    Err(AppError::ApplicationData(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracked_evaluation_bundle_is_valid_and_review_gated() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let summary =
            validate_tracked_evaluation_bundle(&repo_root).expect("valid evaluation bundle");
        assert_eq!(summary.query_count, 42);
        assert_eq!(summary.primary_item_query_count, 20);
        assert_eq!(summary.primary_corpus_query_count, 20);
        assert_eq!(summary.ocr_adjunct_query_count, 2);
        assert_eq!(summary.qa_count, 18);
        assert_eq!(summary.suggested_entry_count, 60);
        assert_eq!(summary.human_verified_entry_count, 0);
        assert_eq!(summary.human_rejected_entry_count, 0);
        assert_eq!(summary.experiment_config_sha256.len(), 64);
        let definition =
            super::super::model::ExperimentDefinition::frozen("corpus", "embedding", "artifact")
                .bind_evaluation_bundle(&summary.experiment_config_sha256)
                .expect("evaluation-bound definition");
        assert_eq!(
            definition.evaluation_bundle_sha256,
            Some(summary.experiment_config_sha256)
        );
    }

    #[test]
    fn retriever_facing_manifest_rejects_ground_truth_fields() {
        let invalid = r#"{
            "schema_version": 1,
            "evaluation_set_id": "m6-c2-retrieval-qa-v1",
            "corpus_id": "corpus",
            "queries": [{
                "query_id": "query",
                "query_text": "text",
                "scope": "corpus",
                "item_ids": [],
                "query_category": "fact",
                "relevant_anchor_ids": ["forbidden"]
            }]
        }"#;
        let error = serde_json::from_str::<RetrievalQueryManifest>(invalid)
            .expect_err("ground truth must not enter the query manifest");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn indexing_retrieval_and_context_sources_do_not_load_the_sealed_key() {
        for source in [
            include_str!("index.rs"),
            include_str!("retrieval.rs"),
            include_str!("coordinator.rs"),
            include_str!("snapshot.rs"),
            include_str!("../../agent/context.rs"),
        ] {
            assert!(!source.contains("retrieval-evaluation-key-v1"));
            assert!(!source.contains("qa-manifest-v1"));
        }
    }
}
