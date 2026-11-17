//! Preserve exact table, opening and explanatory evidence with page provenance.

use rusqlite::{params, OptionalExtension};

use crate::database::ResearchDatabase;
use crate::error::{AppError, AppResult};

use super::model::AgentEvidence;

const OPENING_SCAN_CHUNKS: usize = 12;
const OPENING_EVIDENCE_CHUNKS: usize = 2;
const MAX_OVERVIEW_CHARACTERS: usize = 3_600;
const MAX_EXPLANATION_CHARACTERS: usize = 3_600;

const MAX_TABLE_EVIDENCE_CHARACTERS: usize = 16_000;
const MAX_TABLE_CAPTION_CHARACTERS: usize = 500;

/// Find the complete table and its adjacent caption without rewriting source text.
fn table_range(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let mut offset = 0;
    let lines: Vec<_> = text
        .split_inclusive('\n')
        .map(|line| {
            let position = offset;
            offset += line.len();
            (position, line)
        })
        .collect();
    for (index, (position, line)) in lines.iter().enumerate() {
        if !line.trim_start().starts_with('|') {
            continue;
        }
        let separator = lines.get(index + 1)?.1.trim();
        if !separator.contains('-')
            || !separator.starts_with('|')
            || !separator
                .chars()
                .all(|c| matches!(c, '|' | ':' | '-' | ' '))
        {
            continue;
        }
        let mut table_end = *position;
        for (next, row) in &lines[index..] {
            if !row.trim_start().starts_with('|') {
                break;
            }
            table_end = next + row.trim_end().len();
        }
        let mut table_start = *position;
        let prefix = text[..*position].trim_end();
        let caption_start = prefix.rfind("\n\n").map_or(0, |p| p + 2);
        let caption = &prefix[caption_start..];
        if caption.to_lowercase().starts_with("table ")
            && caption.chars().count() <= MAX_TABLE_CAPTION_CHARACTERS
        {
            table_start = caption_start;
        }
        let low = text[..table_start].chars().count();
        let high = text[..table_end].chars().count();
        if low < end && high > start && high - low <= MAX_TABLE_EVIDENCE_CHARACTERS {
            return Some((low, high));
        }
    }
    None
}

/// Verify the active revision and expand table hits with conservative page ranges.
pub(super) fn complete_passage(
    database: &ResearchDatabase,
    source: &mut AgentEvidence,
) -> AppResult<()> {
    let connection = database.connect()?;
    let text = active_text(&connection, source)?;
    let characters: Vec<_> = text.chars().collect();
    let excerpt = source.excerpt.as_deref().unwrap_or_default();
    let mut start = source.character_start.unwrap_or_default() as usize;
    // Older oversized chunks can point at leading whitespace trimmed from text.
    while characters.get(start).is_some_and(|c| c.is_whitespace()) {
        start += 1;
    }
    let end = start.checked_add(excerpt.chars().count()).ok_or_else(|| {
        AppError::Agent("The indexed passage range is invalid. Reindex this paper.".into())
    })?;
    if end > characters.len() || characters[start..end].iter().collect::<String>() != excerpt {
        return Err(AppError::Agent(
            "The retrieved passage does not match its source revision. Reindex this paper.".into(),
        ));
    }
    source.character_start = Some(start as u64);
    source.character_end = Some(end as u64);
    let Some((low, high)) = table_range(&text, start, end) else {
        return Ok(());
    };
    apply_range(&connection, &characters, source, low, high)
}

/// Apply an exact source range and conservatively retain all overlapping pages.
fn apply_range(
    connection: &rusqlite::Connection,
    characters: &[char],
    source: &mut AgentEvidence,
    low: usize,
    high: usize,
) -> AppResult<()> {
    let mut statement = connection.prepare(
        "SELECT character_start, character_end, page_start, page_end
         FROM semantic_chunks WHERE document_id=?1 AND document_revision_id=?2
           AND status='active' AND character_start<?3 AND character_end>?4",
    )?;
    let ranges = statement
        .query_map(
            params![
                source.document_id,
                source.document_revision_id,
                high as i64,
                low as i64
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<u32>>(2)?,
                    row.get::<_, Option<u32>>(3)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let pages_known = !ranges.is_empty()
        && ranges.iter().all(|r| r.2.is_some() && r.3.is_some())
        && ranges.iter().any(|r| r.0 <= low as i64)
        && ranges.iter().any(|r| r.1 >= high as i64);
    source.page_start = pages_known
        .then(|| ranges.iter().filter_map(|r| r.2).min())
        .flatten();
    source.page_end = pages_known
        .then(|| ranges.iter().filter_map(|r| r.3).max())
        .flatten();
    // Chunk ranges can include adjacent pages outside the expanded passage.
    let text: String = characters.iter().collect();
    if let Some(document_id) = source.document_id.as_deref() {
        let spans = crate::extraction::pages::active_text_spans(connection, document_id, &text)?;
        if !spans.is_empty() {
            let pages = crate::extraction::pages::range_pages(&spans, low, high);
            source.page_start = pages.map(|range| range.0);
            source.page_end = pages.map(|range| range.1);
        }
    }
    source.character_start = Some(low as u64);
    source.character_end = Some(high as u64);
    source.excerpt = Some(characters[low..high].iter().collect());
    Ok(())
}

/// Read only the active text revision already identified by the retrieved source.
fn active_text(connection: &rusqlite::Connection, source: &AgentEvidence) -> AppResult<String> {
    let text: Option<String> = connection
        .query_row(
            "SELECT dt.content FROM document_texts dt
         JOIN document_revisions dr ON dr.document_id=dt.document_id
           AND dr.source_version=dt.source_version
           AND (dr.content_hash IS NULL OR dr.content_hash = ''
                OR dr.content_hash=dt.content_hash)
         WHERE dt.document_id=?1 AND dr.id=?2 AND dt.is_active=1",
            params![source.document_id, source.document_revision_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(text) = text else {
        return Err(AppError::Agent(
            "The retrieved passage has no active source revision. Reindex this paper.".into(),
        ));
    };
    Ok(text)
}

/// Load bounded neighboring chunks from the same active document and profiles.
fn related_chunks(
    database: &ResearchDatabase,
    source: &AgentEvidence,
    opening: bool,
) -> AppResult<Vec<AgentEvidence>> {
    let connection = database.connect()?;
    let mut statement = connection.prepare(
        "SELECT sc.id, sc.text, sc.character_start, sc.character_end,
                sc.page_start, sc.page_end
         FROM semantic_chunks sc
         JOIN semantic_chunks anchor ON anchor.id=?1
         JOIN semantic_vector_entries sve ON sve.vector_key=sc.vector_key
         WHERE anchor.status='active' AND sc.status='active' AND sve.active=1
           AND anchor.item_id=?4 AND anchor.document_id=?5
           AND anchor.document_revision_id=?6
           AND sc.document_id=anchor.document_id
           AND sc.item_id=anchor.item_id
           AND sc.document_revision_id=anchor.document_revision_id
           AND sc.embedding_profile_id=anchor.embedding_profile_id
           AND sc.chunking_profile_id=anchor.chunking_profile_id
           AND ((?2=1 AND sc.ordinal<?3)
             OR (?2=0 AND sc.ordinal=anchor.ordinal-1))
         ORDER BY sc.ordinal, sc.id LIMIT ?3",
    )?;
    let rows = statement.query_map(
        params![
            source.chunk_id,
            opening,
            OPENING_SCAN_CHUNKS as i64,
            source.item_id,
            source.document_id,
            source.document_revision_id
        ],
        |row| {
            let mut related = source.clone();
            related.chunk_id = Some(row.get(0)?);
            related.excerpt = Some(row.get(1)?);
            let offset = |index| -> rusqlite::Result<u64> {
                row.get::<_, i64>(index)?.try_into().map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        index,
                        rusqlite::types::Type::Integer,
                        Box::new(error),
                    )
                })
            };
            related.character_start = Some(offset(2)?);
            related.character_end = Some(offset(3)?);
            related.page_start = row.get(4)?;
            related.page_end = row.get(5)?;
            related.source_id.clear();
            related.source_fingerprint.clear();
            related.lexical_rank = None;
            related.semantic_rank = None;
            related.retrieval_modes = vec![if opening {
                "document-opening"
            } else {
                "preceding-context"
            }
            .into()];
            Ok(related)
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Prefer a bounded abstract opening, skipping publisher covers when identifiable.
pub(super) fn opening_passages(
    database: &ResearchDatabase,
    source: &AgentEvidence,
) -> AppResult<Vec<AgentEvidence>> {
    let chunks = related_chunks(database, source, true)?;
    let heading = regex::Regex::new(r"(?im)(?:^|\n)\s*abstract\b[:.\s]*")
        .expect("static abstract heading regex");
    let abstract_start = chunks.iter().enumerate().find_map(|(index, chunk)| {
        heading
            .find(chunk.excerpt.as_deref().unwrap_or_default())
            .map(|matched| (index, matched.end()))
    });
    let (mut first, mut offset) = abstract_start.unwrap_or((0, 0));
    if chunks.get(first).is_some_and(|chunk| {
        chunk.excerpt.as_deref().unwrap_or_default()[offset..]
            .trim()
            .is_empty()
    }) {
        first += 1;
        offset = 0;
    }
    let mut selected = Vec::new();
    for (index, mut chunk) in chunks
        .into_iter()
        .enumerate()
        .skip(first)
        .take(OPENING_EVIDENCE_CHUNKS)
    {
        complete_passage(database, &mut chunk)?;
        if index == first && offset > 0 {
            let text = chunk.excerpt.as_ref().expect("verified chunk text");
            let offset = heading.find(text).map_or(0, |matched| matched.end());
            let prefix = text[..offset].chars().count() as u64;
            chunk.character_start = Some(chunk.character_start.unwrap_or_default() + prefix);
            chunk.excerpt = Some(text[offset..].to_owned());
        }
        selected.push(chunk);
    }
    Ok(selected)
}

/// Keep a split opening together as one bounded, exact citation passage.
pub(super) fn overview_passages(
    database: &ResearchDatabase,
    source: &AgentEvidence,
) -> AppResult<Vec<AgentEvidence>> {
    let mut passages = opening_passages(database, source)?;
    if passages.len() != 2 {
        return Ok(passages);
    }
    let low = passages[0].character_start.unwrap_or_default() as usize;
    let first_end = passages[0].character_end.unwrap_or_default() as usize;
    let second_start = passages[1].character_start.unwrap_or_default() as usize;
    let high = passages[1].character_end.unwrap_or_default() as usize;
    if high <= low || high - low > MAX_OVERVIEW_CHARACTERS {
        return Ok(passages);
    }
    let connection = database.connect()?;
    let characters: Vec<_> = active_text(&connection, source)?.chars().collect();
    // Do not bridge omitted content or reorder the original source.
    if second_start < low
        || high < first_end
        || first_end < second_start
            && !characters[first_end..second_start]
                .iter()
                .all(|c| c.is_whitespace())
    {
        return Ok(passages);
    }
    apply_range(&connection, &characters, &mut passages[0], low, high)?;
    passages.truncate(1);
    Ok(passages)
}

/// Restore a preceding chunk for explanations without rewriting or guessing text.
pub(super) fn include_preceding_context(
    database: &ResearchDatabase,
    source: &mut AgentEvidence,
) -> AppResult<()> {
    let Some(mut previous) = related_chunks(database, source, false)?.into_iter().next() else {
        return Ok(());
    };
    complete_passage(database, &mut previous)?;
    let low = previous.character_start.unwrap_or_default() as usize;
    let high = source.character_end.unwrap_or_default() as usize;
    if high <= low || high - low > MAX_EXPLANATION_CHARACTERS {
        return Ok(());
    }
    let connection = database.connect()?;
    let characters: Vec<_> = active_text(&connection, source)?.chars().collect();
    if high > characters.len() {
        return Err(AppError::Agent(
            "The explanation source range is invalid. Reindex this paper.".into(),
        ));
    }
    apply_range(&connection, &characters, source, low, high)?;
    source.retrieval_modes.push("preceding-context".into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrowed_table_uses_canonical_pages_not_neighbor_chunk_pages() {
        use crate::extraction::model::{
            CanonicalBlockContent, CanonicalDocumentV1, CanonicalPage, PageRange,
        };
        use crate::extraction::projection::render_plain_text;
        let mut canonical: CanonicalDocumentV1 = serde_json::from_str(include_str!(
            "../../../tools/pdf-benchmark/fixtures/canonical-unstructured.json"
        ))
        .unwrap();
        canonical.blocks[0].content = CanonicalBlockContent::Other {
            label: "text".into(),
            text: "Previous page.".into(),
        };
        canonical.blocks[0].page_range = Some(PageRange { start: 1, end: 1 });
        let mut table = canonical.blocks[0].clone();
        table.id = "table".into();
        table.source_order = 1;
        table.content = CanonicalBlockContent::Other {
            label: "table-markdown".into(),
            text: "Table 1. Values.\n\n| value |\n|---|\n| 10 |".into(),
        };
        table.page_range = Some(PageRange { start: 2, end: 2 });
        canonical.reading_order.push(table.id.clone());
        canonical.blocks.push(table);
        canonical.pages = (1..=2)
            .map(|number| CanonicalPage {
                id: format!("page-{number}"),
                physical_page_number: number,
                printed_label: None,
                width: None,
                height: None,
                rotation_degrees: None,
                coordinate_space: None,
            })
            .collect();
        let text = render_plain_text(&canonical);
        let low = text.find("Table 1").unwrap();
        let high = text.chars().count();
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE semantic_chunks(document_id TEXT,
             document_revision_id INTEGER, status TEXT, character_start INTEGER,
             character_end INTEGER, page_start INTEGER, page_end INTEGER);
             CREATE TABLE document_texts(document_id TEXT, extraction_id TEXT,
             is_active INTEGER);
             CREATE TABLE document_extractions(id TEXT, canonical_json TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO semantic_chunks VALUES('doc',1,'active',0,?1,1,2)",
                [high as i64],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO document_extractions VALUES('saved',?1)",
                [serde_json::to_string(&canonical).unwrap()],
            )
            .unwrap();
        connection
            .execute("INSERT INTO document_texts VALUES('doc','saved',1)", [])
            .unwrap();
        let mut source: AgentEvidence = serde_json::from_value(serde_json::json!({
            "sourceId":"S1", "kind":"document-passage", "itemTitle":"Test",
            "documentId":"doc", "documentRevisionId":1, "retrievalModes":[],
            "provider":"local", "sourceFingerprint":"test",
            "eligibleForScientificClaim":true, "stale":false,
        }))
        .unwrap();
        let chars: Vec<_> = text.chars().collect();
        apply_range(&connection, &chars, &mut source, low, high).unwrap();
        assert_eq!((source.page_start, source.page_end), (Some(2), Some(2)));
        assert!(source.excerpt.as_ref().unwrap().starts_with("Table 1"));
        connection
            .execute("DELETE FROM document_texts", [])
            .unwrap();
        apply_range(&connection, &chars, &mut source, low, high).unwrap();
        assert_eq!((source.page_start, source.page_end), (Some(1), Some(2)));
    }

    #[test]
    fn table_row_retains_caption_headers_and_unicode_offsets() {
        let text = "\u{03b1} Background.\n\nTable 4. Results.\n\n| n | value |\n|---|---|\n| 8 | 0.6281 |\n| 9 | 0.7123 |\n\nAfterwards.";
        let start = text[..text.find("0.6281").unwrap()].chars().count();
        let (low, high) = table_range(text, start, start + 6).unwrap();
        let evidence: String = text.chars().skip(low).take(high - low).collect();
        assert!(evidence.starts_with("Table 4."));
        assert!(evidence.contains("| n | value |"));
        assert!(evidence.ends_with("| 9 | 0.7123 |"));
        assert_eq!(table_range(text, 0, 2), None);
        let oversized = format!(
            "| Header |\n|---|\n| {} |",
            "x".repeat(MAX_TABLE_EVIDENCE_CHARACTERS)
        );
        assert_eq!(table_range(&oversized, 0, 10), None);
    }
}
