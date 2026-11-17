//! Persist bounded PDF.js page text with explicit page provenance.

use crate::database::ResearchDatabase;
use crate::error::AppResult;
use crate::extraction::model::{ExtractionInputDescriptor, ExtractionRequest};
use crate::extraction::pdf_pages::persist_pages;

use super::catalog::Paper;
use super::failure;

const PROFILE: &str = "demo-pdfjs-pages-v1";
const MAX_TEXT_BYTES: usize = 10 * 1024 * 1024;

pub(crate) fn store_pages(
    database: &ResearchDatabase,
    document_id: &str,
    paper: &Paper,
    pages: Vec<String>,
) -> AppResult<()> {
    if pages.len() != paper.pdf_pages as usize
        || pages
            .iter()
            .any(|page| page.len() > 1024 * 1024 || page.contains('\0'))
        || pages.iter().map(String::len).sum::<usize>() > MAX_TEXT_BYTES
        || pages.iter().map(String::len).sum::<usize>() < 1000
    {
        return Err(failure(
            "Demo PDF text has an invalid page count or text size.",
        ));
    }
    let normalize = |value: &str| {
        value
            .to_ascii_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
    };
    if !normalize(&pages.iter().take(2).cloned().collect::<Vec<_>>().join(" "))
        .contains(&normalize(&paper.doi))
    {
        return Err(failure(
            "The extracted opening pages do not identify the expected demo paper.",
        ));
    }
    let connection = database.connect()?;
    let revision: i64=connection.query_row("SELECT id FROM document_revisions WHERE document_id=?1 AND content_hash=?2 AND revision_kind='artifact-content' ORDER BY id DESC LIMIT 1",
        rusqlite::params![document_id,paper.sha256],|row|row.get(0))?;
    let request = ExtractionRequest {
        artifact_id: document_id.into(),
        artifact_revision_id: revision,
        input_content_hash: paper.sha256.clone(),
        input: ExtractionInputDescriptor {
            media_type: "application/pdf".into(),
            has_local_file: true,
            has_bounded_reader: true,
            has_source_provided_text: false,
        },
        source_text: None,
    };
    persist_pages(database, &request, pages, PROFILE)?;
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO fulltext_sync_state
        (library_id,successful_version,status,last_synced_at,changed_documents,warning_count)
        SELECT ?1,1,CASE WHEN COUNT(*)=?2 THEN 'ready' ELSE 'partial' END,
            CURRENT_TIMESTAMP,COUNT(*),0
        FROM document_texts WHERE library_id=?1 AND is_active=1 AND completeness='full'
        ON CONFLICT(library_id) DO UPDATE SET status=excluded.status,
            last_synced_at=excluded.last_synced_at,changed_documents=excluded.changed_documents,
            successful_version=1,warning_count=0,last_error=NULL",
        rusqlite::params![
            super::catalog::LIBRARY_ID,
            super::catalog::manifest()?.papers.len() as i64
        ],
    )?;
    transaction.commit()?;
    Ok(())
}
