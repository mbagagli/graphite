use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::canonical::{render_plain_text, validate_canonical_document};
use crate::model::{
    AggregateResult, AnchorMatcher, AnchorSet, AnchorVerificationStatus,
    BenchmarkExtractionEnvelopeV1, CorpusManifest, OperationalResultSet, ANCHOR_SCHEMA_VERSION,
    BENCHMARK_SCHEMA_VERSION,
};

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))
}

pub fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("output {} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("output {} has no UTF-8 filename", path.display()))?;
    let temporary = parent.join(format!(".{file_name}.tmp"));
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

pub fn validate_manifest(manifest: &CorpusManifest, root: &Path) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    if manifest.schema_version != 1 {
        errors.push("corpus schemaVersion must be 1".to_owned());
    }
    if manifest.corpus_id.trim().is_empty() {
        errors.push("corpusId must not be empty".to_owned());
    }
    let real_smoke = manifest.corpus_id == "graphite-scientific-pdf-smoke-real-v1";
    if real_smoke && manifest.documents.len() != 10 {
        errors.push("real smoke corpus must contain exactly 10 documents".to_owned());
    }
    let mut ids = HashSet::new();
    for (index, document) in manifest.documents.iter().enumerate() {
        if real_smoke {
            let expected_id = format!("paper{:02}", index + 1);
            if document.id != expected_id {
                errors.push(format!(
                    "real smoke document {} must use neutral id {expected_id}",
                    document.id
                ));
            }
            if document.file != format!("local/{expected_id}.pdf")
                || document.anchors != format!("anchors/{expected_id}.json")
            {
                errors.push(format!(
                    "real smoke document {expected_id} must use neutral relative aliases"
                ));
            }
        }
        if !ids.insert(document.id.as_str()) {
            errors.push(format!("duplicate document id {}", document.id));
        }
        for (label, value) in [
            ("file", document.file.as_str()),
            ("anchors", document.anchors.as_str()),
        ] {
            if let Err(error) = validate_relative_path(value) {
                errors.push(format!("document {} {label}: {error}", document.id));
            }
        }
        if let Some(value) = &document.zotero_baseline {
            if let Err(error) = validate_relative_path(value) {
                errors.push(format!("document {} zoteroBaseline: {error}", document.id));
            }
        }
        if !valid_sha256(&document.sha256) || !valid_sha256(&document.anchors_sha256) {
            errors.push(format!("document {} has invalid SHA-256", document.id));
        }
        if document.zotero_baseline.is_some() != document.zotero_baseline_sha256.is_some() {
            errors.push(format!(
                "document {} must pair zoteroBaseline with zoteroBaselineSha256",
                document.id
            ));
        }
        if document
            .zotero_baseline_sha256
            .as_deref()
            .is_some_and(|value| !valid_sha256(value))
        {
            errors.push(format!(
                "document {} has invalid Zotero baseline SHA-256",
                document.id
            ));
        }
        if document.license.trim().is_empty() {
            errors.push(format!("document {} has no declared license", document.id));
        }
        if document
            .source_url
            .as_deref()
            .is_some_and(|value| !value.starts_with("https://"))
        {
            errors.push(format!("document {} sourceUrl must use HTTPS", document.id));
        }
        if document.categories.is_empty() {
            errors.push(format!("document {} has no categories", document.id));
        }
        if document.expected_ocr_pages.contains(&0) {
            errors.push(format!(
                "document {} has zero-indexed OCR page",
                document.id
            ));
        }
        if document.expected_pages.is_some_and(|page_count| {
            document
                .expected_ocr_pages
                .iter()
                .any(|page| *page > page_count)
        }) {
            errors.push(format!(
                "document {} has an OCR page beyond its expected page count",
                document.id
            ));
        }
        validate_locked_file(
            root,
            &document.file,
            &document.sha256,
            &document.id,
            "PDF",
            &mut errors,
        );
        validate_locked_file(
            root,
            &document.anchors,
            &document.anchors_sha256,
            &document.id,
            "anchors",
            &mut errors,
        );
        if let (Some(path), Some(hash)) =
            (&document.zotero_baseline, &document.zotero_baseline_sha256)
        {
            validate_locked_file(
                root,
                path,
                hash,
                &document.id,
                "Zotero baseline",
                &mut errors,
            );
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_locked_file(
    root: &Path,
    relative: &str,
    expected_hash: &str,
    document_id: &str,
    label: &str,
    errors: &mut Vec<String>,
) {
    let path = root.join(relative);
    match fs::read(&path) {
        Ok(bytes) if content_hash(&bytes) != expected_hash => errors.push(format!(
            "document {document_id} {label} checksum does not match {}",
            path.display()
        )),
        Ok(_) => {}
        Err(error) => errors.push(format!(
            "cannot read document {document_id} {label} {}: {error}",
            path.display()
        )),
    }
}

pub fn validate_anchor_set(set: &AnchorSet, document_id: &str) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    if set.schema_version != ANCHOR_SCHEMA_VERSION {
        errors.push(format!(
            "anchor schemaVersion must be {ANCHOR_SCHEMA_VERSION}"
        ));
    }
    if set.document_id != document_id {
        errors.push(format!(
            "anchor documentId {} does not match {}",
            set.document_id, document_id
        ));
    }
    let mut ids = HashSet::new();
    for anchor in &set.anchors {
        if !ids.insert(anchor.id.as_str()) {
            errors.push(format!("duplicate anchor id {}", anchor.id));
        }
        if anchor.expected_pages.contains(&0) {
            errors.push(format!("anchor {} uses zero-indexed page", anchor.id));
        }
        if anchor.verification_note.trim().is_empty() {
            errors.push(format!("anchor {} has no verification note", anchor.id));
        }
        match &anchor.matcher {
            AnchorMatcher::Exact { value } | AnchorMatcher::Normalized { value }
                if value.is_empty() =>
            {
                errors.push(format!("anchor {} has an empty matcher", anchor.id));
            }
            AnchorMatcher::OrderedTokens { tokens, .. } if tokens.is_empty() => {
                errors.push(format!("anchor {} has no ordered tokens", anchor.id));
            }
            AnchorMatcher::Alternatives { values } if values.is_empty() => {
                errors.push(format!("anchor {} has no alternatives", anchor.id));
            }
            AnchorMatcher::NumericContext { value, .. } if value.parse::<f64>().is_err() => {
                errors.push(format!("anchor {} has a non-numeric value", anchor.id));
            }
            _ => {}
        }
    }
    for anchor in &set.anchors {
        for (label, related_id) in [
            ("expectedAfter", anchor.expected_after.as_ref()),
            ("expectedBefore", anchor.expected_before.as_ref()),
        ] {
            let Some(related_id) = related_id else {
                continue;
            };
            if related_id == &anchor.id {
                errors.push(format!(
                    "anchor {} cannot reference itself through {label}",
                    anchor.id
                ));
            } else if !ids.contains(related_id.as_str()) {
                errors.push(format!(
                    "anchor {} {label} references unknown anchor {related_id}",
                    anchor.id
                ));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn validate_scoring_anchors(set: &AnchorSet) -> Result<(), Vec<String>> {
    let verified_ids = set
        .anchors
        .iter()
        .filter(|anchor| anchor.verification_status == AnchorVerificationStatus::HumanVerified)
        .map(|anchor| anchor.id.as_str())
        .collect::<HashSet<_>>();
    let mut errors = set
        .anchors
        .iter()
        .filter(|anchor| {
            anchor.verification_status == AnchorVerificationStatus::SuggestedUnverified
        })
        .map(|anchor| {
            format!(
                "anchor {} is suggested-unverified; verify or reject it before scoring",
                anchor.id
            )
        })
        .collect::<Vec<_>>();
    for anchor in &set.anchors {
        if anchor.verification_status != AnchorVerificationStatus::HumanVerified {
            continue;
        }
        for related_id in [
            anchor.expected_after.as_deref(),
            anchor.expected_before.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if !verified_ids.contains(related_id) {
                errors.push(format!(
                    "verified anchor {} has a reading-order relation to non-verified anchor {}",
                    anchor.id, related_id
                ));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn validate_extraction(value: &BenchmarkExtractionEnvelopeV1) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    if value.schema_version != BENCHMARK_SCHEMA_VERSION {
        errors.push(format!(
            "unsupported extraction schema version {}",
            value.schema_version
        ));
    }
    if !valid_sha256(&value.input_sha256) {
        errors.push("extraction inputSha256 is invalid".to_owned());
    }
    if value.outcome.has_output() && value.searchable_projection.is_none() {
        errors.push("output-bearing extraction has no searchable projection".to_owned());
    }
    if let Some(canonical) = &value.canonical_document {
        if let Err(error) = validate_canonical_document(canonical) {
            errors.push(error);
        }
        if value.searchable_projection.as_deref() != Some(render_plain_text(canonical).as_str()) {
            errors.push("canonical projection differs from searchable projection".to_owned());
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn validate_aggregate_privacy(value: &AggregateResult) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    match serde_json::to_value(value) {
        Ok(serialized) => validate_private_value(&serialized, "$", &mut errors),
        Err(error) => errors.push(format!("aggregate result cannot be inspected: {error}")),
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn validate_operational_privacy(value: &OperationalResultSet) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    match serde_json::to_value(value) {
        Ok(serialized) => validate_private_value(&serialized, "$", &mut errors),
        Err(error) => errors.push(format!("operational result cannot be inspected: {error}")),
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        return Err("path must be relative".to_owned());
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("path must remain inside the corpus root".to_owned());
    }
    if value.contains('\\') {
        return Err("path must use forward slashes".to_owned());
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_private_value(value: &serde_json::Value, location: &str, errors: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, child) in fields {
                if matches!(
                    key.as_str(),
                    "inputPath" | "searchableProjection" | "canonicalDocument"
                ) {
                    errors.push(format!(
                        "aggregate result contains forbidden field {location}.{key}"
                    ));
                }
                validate_private_value(child, &format!("{location}.{key}"), errors);
            }
        }
        serde_json::Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                validate_private_value(child, &format!("{location}[{index}]"), errors);
            }
        }
        serde_json::Value::String(text)
            if text.starts_with("file://") || resembles_absolute_path(text) =>
        {
            errors.push(format!(
                "aggregate result contains an absolute path at {location}"
            ));
        }
        _ => {}
    }
}

fn resembles_absolute_path(value: &str) -> bool {
    value.split_whitespace().any(|token| {
        let candidate = token.trim_matches(|character: char| {
            matches!(
                character,
                '\'' | '"' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';'
            )
        });
        Path::new(candidate).is_absolute()
            || candidate.starts_with("\\\\")
            || (candidate.len() >= 3
                && candidate.as_bytes()[0].is_ascii_alphabetic()
                && candidate.as_bytes()[1] == b':'
                && matches!(candidate.as_bytes()[2], b'\\' | b'/'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Anchor, AnchorKind, AnchorVerificationStatus, CorpusDocument, TextLayerKind,
    };

    #[test]
    fn manifest_rejects_absolute_paths() {
        let manifest = CorpusManifest {
            schema_version: 1,
            corpus_id: "test".to_owned(),
            documents: vec![CorpusDocument {
                id: "doc".to_owned(),
                file: "/private/doc.pdf".to_owned(),
                sha256: "0".repeat(64),
                source_url: None,
                license: "generated".to_owned(),
                redistributable: false,
                categories: vec!["native".to_owned()],
                language: "en".to_owned(),
                expected_pages: Some(1),
                text_layer: TextLayerKind::Native,
                expected_ocr_pages: Vec::new(),
                anchors: "anchors/doc.json".to_owned(),
                anchors_sha256: "0".repeat(64),
                zotero_baseline: None,
                zotero_baseline_sha256: None,
            }],
        };
        assert!(validate_manifest(&manifest, Path::new(".")).is_err());
    }

    #[test]
    fn anchor_set_rejects_unknown_reading_order_reference() {
        let set = AnchorSet {
            schema_version: ANCHOR_SCHEMA_VERSION,
            document_id: "doc".to_owned(),
            anchors: vec![Anchor {
                id: "first".to_owned(),
                kind: AnchorKind::ReadingOrderPair,
                required: true,
                verification_status: AnchorVerificationStatus::HumanVerified,
                verification_note: "verified test fixture".to_owned(),
                expected_pages: vec![1],
                matcher: AnchorMatcher::Exact {
                    value: "first".to_owned(),
                },
                expected_after: Some("missing".to_owned()),
                expected_before: None,
            }],
        };

        let errors = validate_anchor_set(&set, "doc").expect_err("reference must fail");
        assert!(errors.iter().any(|error| error.contains("unknown anchor")));
    }

    #[test]
    fn scoring_rejects_suggested_anchor_truth() {
        let set = AnchorSet {
            schema_version: ANCHOR_SCHEMA_VERSION,
            document_id: "doc".to_owned(),
            anchors: vec![Anchor {
                id: "suggestion".to_owned(),
                kind: AnchorKind::TextFact,
                required: true,
                verification_status: AnchorVerificationStatus::SuggestedUnverified,
                verification_note: "candidate for human review".to_owned(),
                expected_pages: vec![1],
                matcher: AnchorMatcher::Exact {
                    value: "candidate".to_owned(),
                },
                expected_after: None,
                expected_before: None,
            }],
        };

        let errors = validate_scoring_anchors(&set).expect_err("suggestion must not score");
        assert!(errors[0].contains("suggested-unverified"));
    }

    #[test]
    fn privacy_inspection_rejects_any_absolute_path() {
        for path in [
            "/tmp/private.pdf",
            "C:\\private\\paper.pdf",
            "parser failed while reading /tmp/private.pdf: denied",
        ] {
            let value = serde_json::json!({"candidateId": path});
            let mut errors = Vec::new();
            validate_private_value(&value, "$", &mut errors);
            assert!(!errors.is_empty());
        }
    }
}
