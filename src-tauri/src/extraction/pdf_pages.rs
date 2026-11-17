//! Prepare current local PDF text without changing source documents.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::database::{content_hash, ResearchDatabase};
use crate::error::{AppError, AppResult};

use super::model::{
    CanonicalBlock, CanonicalBlockContent, CanonicalDocumentMetadata, CanonicalDocumentV1,
    CanonicalPage, EvaluatedExtractionAttempt, ExtractionAttempt, ExtractionAttemptOutcome,
    ExtractionInputDescriptor, ExtractionObservations, ExtractionRequest, ExtractionRoutingProfile,
    ExtractionSelectionDecision, ExtractionSignal, PageRange, RoutedExtraction,
    CANONICAL_SCHEMA_VERSION,
};
use super::projection::render_plain_text;
use super::provider::DocumentExtractionProvider;
use super::quality::ExtractionQualityEvaluator;
use super::store::ExtractionStore;
use super::zotero_text::ZoteroTextExtractionProvider;

const PROFILE: &str = "pdfjs-pages-v1";
const MAX_TEXT_BYTES: usize = 10 * 1024 * 1024;
const MAX_PAGE_BYTES: usize = 1024 * 1024;
const MAX_PAGES: usize = 10_000;
const MAX_PDF_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparationDocument {
    pub item_id: String,
    pub title: String,
    pub document_id: Option<String>,
    pub has_usable_text: bool,
    pub media_type: Option<String>,
}

/// List the active library's documents and papers missing attachments.
pub(crate) fn preparation_documents(
    database: &ResearchDatabase,
    library_id: &str,
) -> AppResult<Vec<PreparationDocument>> {
    let connection = database.connect()?;
    require_library(&connection, library_id)?;
    let mut statement = connection.prepare(
        "SELECT i.id,i.title,d.id,d.media_type
         FROM research_items i LEFT JOIN documents d
           ON d.research_item_id=i.id AND (
             d.media_type='application/pdf' OR EXISTS(
               SELECT 1 FROM document_texts t WHERE t.document_id=d.id
               AND t.is_active=1 AND length(trim(t.content))>0))
         WHERE i.library_id=?1 AND i.is_trashed=0
         ORDER BY i.title,i.id,d.id",
    )?;
    let mut rows = statement
        .query_map([library_id], |row| {
            Ok(PreparationDocument {
                item_id: row.get(0)?,
                title: row.get(1)?,
                document_id: row.get(2)?,
                has_usable_text: false,
                media_type: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for row in &mut rows {
        if let Some(document_id) = &row.document_id {
            row.has_usable_text = has_usable_text(&connection, document_id)?;
        }
    }
    Ok(rows)
}

fn require_library(connection: &Connection, library_id: &str) -> AppResult<()> {
    let active: Option<String> = connection
        .query_row(
            "SELECT value FROM app_meta WHERE key='active_library_id'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if active.as_deref() != Some(library_id) {
        return Err(failure(
            "The active library changed. Restart preparation for the selected library.",
        ));
    }
    Ok(())
}

fn has_usable_text(connection: &Connection, document_id: &str) -> AppResult<bool> {
    let ready: bool = connection.query_row(
        "SELECT EXISTS(
          SELECT 1 FROM document_texts t
          LEFT JOIN document_extractions e ON e.id=t.extraction_id
          LEFT JOIN document_revisions r ON r.id=e.artifact_revision_id
          LEFT JOIN document_locators l ON l.document_id=t.document_id
          LEFT JOIN document_revisions current_revision
            ON current_revision.document_id=t.document_id
              AND current_revision.revision_key=l.revision_key
          LEFT JOIN active_extraction_selections s
            ON s.artifact_id=t.document_id AND s.extraction_id=t.extraction_id
          LEFT JOIN extraction_quality_reports q ON q.id=s.quality_report_id
          LEFT JOIN artifact_extraction_state a ON a.artifact_id=t.document_id
          WHERE t.document_id=?1 AND t.is_active=1
            AND length(trim(t.content))>0
            AND COALESCE(a.execution_state,'')<>'stale'
            AND (t.extraction_id IS NULL OR (q.usable=1 AND (
              r.revision_kind<>'artifact-content'
              OR r.content_hash=current_revision.content_hash))))",
        [document_id],
        |row| row.get(0),
    )?;
    if !ready {
        return Ok(false);
    }
    let local: Option<(String, i64, Option<i64>)> = connection
        .query_row(
            "SELECT l.canonical_path,l.file_size,l.modified_ms
         FROM document_texts t JOIN document_extractions e ON e.id=t.extraction_id
         JOIN document_revisions r ON r.id=e.artifact_revision_id
         JOIN document_locators l ON l.document_id=t.document_id
         WHERE t.document_id=?1 AND t.is_active=1
           AND r.revision_kind='artifact-content'",
            [document_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some((path, size, modified)) = local {
        if let Ok(metadata) = Path::new(&path).metadata() {
            let current_modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as i64);
            return Ok(metadata.is_file()
                && metadata.len() == size as u64
                && modified.is_none_or(|expected| current_modified == Some(expected)));
        }
        // Cached extracted text remains usable when the original is offline.
    }
    Ok(true)
}

/// Validate the current source revision before accepting extracted pages.
pub(crate) fn store_document_pages(
    database: &ResearchDatabase,
    document_id: &str,
    revision_key: &str,
    pages: Vec<String>,
) -> AppResult<()> {
    validate_pages(&pages)?;
    let connection = database.connect()?;
    let row: Option<(String, String, String, i64, String, i64)> = connection
        .query_row(
            "SELECT d.library_id,l.canonical_path,l.revision_key,r.id,r.content_hash,l.file_size
         FROM documents d JOIN document_locators l ON l.document_id=d.id
         JOIN document_revisions r ON r.document_id=d.id AND r.revision_key=l.revision_key
         WHERE d.id=?1 AND d.media_type='application/pdf'
           AND r.revision_kind='artifact-content'",
            [document_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((library_id, path, current, revision, hash, size)) = row else {
        return Err(failure(
            "Resolve the current local PDF before preparing its text.",
        ));
    };
    require_library(&connection, &library_id)?;
    if current != revision_key {
        return Err(failure(
            "The PDF changed while preparing text. Retry preparation.",
        ));
    }
    let size = u64::try_from(size)
        .map_err(|_| failure("The stored PDF size is invalid. Resolve the attachment again."))?;
    verify_original(Path::new(&path), size, &hash)?;
    require_library(&connection, &library_id)?;
    if library_id == crate::demo::catalog::LIBRARY_ID {
        let paper_key: String = connection.query_row(
            "SELECT i.zotero_key FROM documents d
             JOIN research_items i ON i.id=d.research_item_id WHERE d.id=?1",
            [document_id],
            |row| row.get(0),
        )?;
        let paper = crate::demo::catalog::manifest()?
            .papers
            .into_iter()
            .find(|paper| paper.id == paper_key)
            .ok_or_else(|| failure("Unknown demo PDF."))?;
        if hash != paper.sha256 || size != paper.pdf_bytes {
            return Err(failure(
                "The demo PDF changed. Import its verified original.",
            ));
        }
        return crate::demo::text::store_pages(database, document_id, &paper, pages);
    }
    if has_usable_text(&connection, document_id)? {
        return Ok(());
    }
    let request = ExtractionRequest {
        artifact_id: document_id.into(),
        artifact_revision_id: revision,
        input_content_hash: hash,
        input: ExtractionInputDescriptor {
            media_type: "application/pdf".into(),
            has_local_file: true,
            has_bounded_reader: true,
            has_source_provided_text: false,
        },
        source_text: None,
    };
    persist_pages(database, &request, pages, PROFILE)
}

fn verify_original(path: &Path, expected_size: u64, expected_hash: &str) -> AppResult<()> {
    if !(5..=MAX_PDF_BYTES).contains(&expected_size) {
        return Err(failure("The local PDF exceeds preparation limits."));
    }
    let file = File::open(path)
        .map_err(|_| failure("The local PDF is unavailable. Restore it in Zotero and retry."))?;
    let mut bounded = file.take(expected_size + 1);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut count = 0_u64;
    loop {
        let read = bounded
            .read(&mut buffer)
            .map_err(|_| failure("The local PDF could not be read. Retry preparation."))?;
        if read == 0 {
            break;
        }
        if count == 0 && (read < 5 || &buffer[..5] != b"%PDF-") {
            return Err(failure("The attachment is not a readable PDF."));
        }
        count += read as u64;
        hasher.update(&buffer[..read]);
    }
    let actual = hex(&hasher.finalize());
    if count != expected_size || actual != expected_hash {
        return Err(failure(
            "The PDF changed while preparing text. Retry preparation.",
        ));
    }
    Ok(())
}

fn validate_pages(pages: &[String]) -> AppResult<()> {
    if pages.is_empty()
        || pages.len() > MAX_PAGES
        || pages
            .iter()
            .any(|page| page.len() > MAX_PAGE_BYTES || page.contains('\0'))
        || pages.iter().map(String::len).sum::<usize>() > MAX_TEXT_BYTES
    {
        return Err(failure("The PDF page text exceeds preparation limits."));
    }
    if pages.iter().all(|page| page.trim().is_empty()) {
        return Err(failure(
            "No selectable text was found. This PDF may need OCR.",
        ));
    }
    Ok(())
}

/// Persist verified page text through the shared canonical selection store.
pub(crate) fn persist_pages(
    database: &ResearchDatabase,
    request: &ExtractionRequest,
    pages: Vec<String>,
    profile: &str,
) -> AppResult<()> {
    validate_pages(&pages)?;
    let page_count = pages.len() as u32;
    let connection = database.connect()?;
    if has_usable_text(&connection, &request.artifact_id)? {
        return Ok(());
    }
    let manual: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM active_extraction_selections
         WHERE artifact_id=?1 AND routing_profile_id='manual-extraction-v1')",
        [&request.artifact_id],
        |row| row.get(0),
    )?;
    if manual {
        return Err(failure(
            "The selected extraction needs review in the reader. It was kept unchanged.",
        ));
    }
    let mut descriptor = ZoteroTextExtractionProvider.descriptor();
    descriptor.profile_id = profile.into();
    descriptor.provider_id = "graphite-pdfjs".into();
    descriptor.provider_version = "6.2.108".into();
    descriptor.mapping_profile = "pdfjs-page-text-v1".into();
    descriptor.required_input_forms = vec!["verified-local-pdf-pages".into()];
    descriptor.native_text_capability = true;
    descriptor.page_provenance_capability = true;
    let mut document = CanonicalDocumentV1 {
        schema_version: CANONICAL_SCHEMA_VERSION.into(),
        artifact_id: request.artifact_id.clone(),
        artifact_revision_id: request.artifact_revision_id,
        extraction_profile_id: profile.into(),
        metadata: CanonicalDocumentMetadata {
            reported_page_count_hint: Some(page_count),
            ..Default::default()
        },
        pages: Vec::new(),
        blocks: Vec::new(),
        reading_order: Vec::new(),
        relationships: Vec::new(),
        assets: Vec::new(),
    };
    for (index, text) in pages.iter().enumerate() {
        let page = index as u32 + 1;
        document.pages.push(CanonicalPage {
            id: format!("page-{page}"),
            physical_page_number: page,
            printed_label: None,
            width: None,
            height: None,
            rotation_degrees: None,
            coordinate_space: None,
        });
        if text.trim().is_empty() {
            continue;
        }
        let id = format!("page-text-{page}");
        document.blocks.push(CanonicalBlock {
            id: id.clone(),
            parent_id: None,
            source_order: index as u32,
            section_path: Vec::new(),
            page_range: Some(PageRange {
                start: page,
                end: page,
            }),
            regions: Vec::new(),
            content: CanonicalBlockContent::Other {
                label: "pdfjs-page-text".into(),
                text: text.clone(),
            },
        });
        document.reading_order.push(id);
    }
    let plain_text = render_plain_text(&document);
    let mut observations = ExtractionObservations::unreported(
        profile,
        &request.artifact_id,
        request.artifact_revision_id,
        request.input_content_hash.clone(),
    );
    observations.reported_page_count = ExtractionSignal::observed(page_count, profile);
    observations.extracted_page_count = ExtractionSignal::observed(page_count, profile);
    observations.pages_with_text =
        ExtractionSignal::observed(document.blocks.len() as u32, profile);
    observations.characters_by_page = ExtractionSignal::observed(
        pages.iter().map(|p| p.chars().count() as u64).collect(),
        profile,
    );
    observations.plain_text_character_count =
        ExtractionSignal::observed(plain_text.chars().count() as u64, profile);
    observations.plain_text_hash = ExtractionSignal::observed(content_hash(&plain_text), profile);
    observations.ocr_used = ExtractionSignal::observed(false, profile);
    let attempt = ExtractionAttempt {
        provider: descriptor,
        canonical_document: Some(document),
        provenance: BTreeMap::from([
            (
                "originalPdfSha256".into(),
                serde_json::json!(request.input_content_hash),
            ),
            (
                "extractor".into(),
                serde_json::json!("bundled PDF.js page text"),
            ),
        ]),
        observations,
        warnings: vec![
            "PDF.js text order is retained; layout, figures, tables and OCR are not inferred."
                .into(),
        ],
        error: None,
    };
    let evaluator = ExtractionQualityEvaluator::default_v1();
    let quality = evaluator.evaluate(&attempt);
    if !quality.usable {
        return Err(failure(
            "The extracted PDF text did not pass validation. Check the document in the reader.",
        ));
    }
    let routed = RoutedExtraction {
        attempts: vec![EvaluatedExtractionAttempt {
            attempt,
            quality,
            outcome: ExtractionAttemptOutcome::Succeeded,
        }],
        historical_active: None,
        selection: ExtractionSelectionDecision::NewAttempt(0),
        selection_reason: "User prepared the current local PDF with the bundled page extractor."
            .into(),
        fallback_used: false,
        fallback_reason: None,
    };
    let routing = ExtractionRoutingProfile {
        id: profile.into(),
        version: "1".into(),
        provider_order: vec![profile.into()],
        compare_degraded: false,
        allow_usable_degraded: true,
    };
    ExtractionStore::new(database.clone())
        .persist_routed_extraction(request, &routing, &evaluator, &routed)?;
    let connection = database.connect()?;
    connection.execute(
        "UPDATE document_texts SET completeness='full',indexed_pages=?2,
         total_pages=?2,indexed_chars=?3,total_chars=?3
         WHERE document_id=?1 AND is_active=1 AND extraction_id IN (
           SELECT id FROM document_extractions WHERE artifact_id=?1
           AND provider_profile_id=?4)",
        params![
            request.artifact_id,
            page_count,
            plain_text.chars().count() as i64,
            profile
        ],
    )?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn failure(message: &str) -> AppError {
    AppError::Extraction(message.into())
}

#[cfg(test)]
mod tests;
