//! Map canonical page provenance onto verified scientific chunk ranges.

use super::model::{CanonicalBlockContent, CanonicalDocumentV1};
use super::projection::render_plain_text;
use crate::error::{AppError, AppResult};
use crate::semantic::chunking::{DocumentSource, ScientificChunk};
use rusqlite::OptionalExtension;

pub(crate) struct TextSpan {
    start: usize,
    end: usize,
    pages: Option<(u32, u32)>,
}

pub(crate) fn active_spans(
    connection: &rusqlite::Connection,
    source: &DocumentSource,
) -> AppResult<Vec<TextSpan>> {
    active_text_spans(connection, &source.document_id, &source.text)
}

/// Map the verified active canonical text for a document.
pub(crate) fn active_text_spans(
    connection: &rusqlite::Connection,
    document_id: &str,
    text: &str,
) -> AppResult<Vec<TextSpan>> {
    let canonical: Option<String> = connection
        .query_row(
            "SELECT e.canonical_json FROM document_texts t
         JOIN document_extractions e ON e.id=t.extraction_id
         WHERE t.document_id=?1 AND t.is_active=1",
            [document_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let Some(canonical) = canonical else {
        return Ok(Vec::new());
    };
    let document: CanonicalDocumentV1 = serde_json::from_str(&canonical)?;
    if render_plain_text(&document) != text {
        return Err(AppError::Semantic(
            "Canonical text differs from active extraction".into(),
        ));
    }
    let mut spans = Vec::new();
    let mut offset = 0;
    // Reuse the frozen projection without cloning the full document per block.
    let mut single = document.clone();
    single.blocks.clear();
    single.reading_order.clear();
    for id in &document.reading_order {
        let block = document
            .blocks
            .iter()
            .find(|b| &b.id == id)
            .ok_or_else(|| AppError::Semantic("Unknown canonical block".into()))?;
        if block.content.plain_text().is_none()
            && !matches!(block.content, CanonicalBlockContent::Table { .. })
        {
            continue;
        }
        single.blocks.clear();
        single.blocks.push(block.clone());
        single.reading_order.clear();
        single.reading_order.push(id.clone());
        let text = render_plain_text(&single);
        if !spans.is_empty() {
            offset += 2;
        }
        let end = offset + text.chars().count();
        spans.push(TextSpan {
            start: offset,
            end,
            pages: block.page_range.as_ref().map(|p| (p.start, p.end)),
        });
        offset = end;
    }
    Ok(spans)
}

type ChunkLocation = (usize, usize, Option<(u32, u32)>);

pub(crate) fn chunk_location(
    source: &DocumentSource,
    chunk: &ScientificChunk,
    spans: &[TextSpan],
) -> AppResult<ChunkLocation> {
    if spans.is_empty() {
        return Ok((chunk.character_start, chunk.character_end, None));
    }
    let chars: Vec<_> = source.text.chars().collect();
    let mut start = chunk.character_start;
    while chars.get(start).is_some_and(|c| c.is_whitespace()) {
        start += 1;
    }
    let end = start + chunk.text.chars().count();
    if end > chars.len() || chars[start..end].iter().collect::<String>() != chunk.text {
        return Err(AppError::Semantic(
            "Chunk does not match its canonical source; extraction was not indexed".into(),
        ));
    }
    Ok((start, end, range_pages(spans, start, end)))
}

/// Preserve only pages overlapping the exact displayed character range.
pub(crate) fn range_pages(spans: &[TextSpan], start: usize, end: usize) -> Option<(u32, u32)> {
    let mut pages = None;
    for span in spans.iter().filter(|s| s.start < end && s.end > start) {
        let (low, high) = span.pages?;
        pages = Some(pages.map_or((low, high), |(a, b): (u32, u32)| (a.min(low), b.max(high))));
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_ranges_keep_cross_page_and_unknown_provenance_conservative() {
        let source = DocumentSource {
            item_id: "item".into(),
            library_id: "library".into(),
            document_id: "document".into(),
            document_revision_id: 1,
            provider: "local".into(),
            text: " First.\n\nSecond.".into(),
            body_end_byte: 16,
            bibliography_excluded: false,
        };
        let chunk = ScientificChunk {
            id: "chunk".into(),
            ordinal: 0,
            text: "First.\n\nSecond.".into(),
            character_start: 0,
            character_end: 16,
            token_count: 3,
            fingerprint: "hash".into(),
            vector_key: 1,
        };
        let mut spans = vec![
            TextSpan {
                start: 0,
                end: 7,
                pages: Some((1, 1)),
            },
            TextSpan {
                start: 9,
                end: 16,
                pages: Some((2, 2)),
            },
        ];
        assert_eq!(
            chunk_location(&source, &chunk, &spans).unwrap(),
            (1, 16, Some((1, 2)))
        );
        // A narrowed passage must not inherit the preceding chunk's page.
        assert_eq!(range_pages(&spans, 9, 16), Some((2, 2)));
        assert_eq!(range_pages(&spans, 7, 9), None);
        spans[1].pages = None;
        assert_eq!(chunk_location(&source, &chunk, &spans).unwrap().2, None);
        let mut wrong = chunk;
        wrong.text = "Invented evidence".into();
        assert!(chunk_location(&source, &wrong, &spans).is_err());
    }
}
