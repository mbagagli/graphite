use super::{stable_semantic_id, text_fingerprint, EmbeddingProvider};
use crate::error::AppResult;

pub(crate) const CHUNKING_PROFILE_ID: &str = "scientific-paragraph-v1";
pub(crate) const CHUNKING_STRATEGY: &str = "paragraph-sentence";
pub(crate) const CHUNKING_VERSION: &str = "1.0.0";
pub(crate) const TARGET_TOKENS: usize = 220;
pub(crate) const OVERLAP_TOKENS: usize = 36;
pub(crate) const MAXIMUM_TOKENS: usize = 360;
pub(crate) const MAXIMUM_CHUNKS: usize = 512;
pub(crate) const BOUNDARY_POLICY: &str = "paragraph-first; sentence fallback; no mid-word cuts";

#[derive(Clone, Debug)]
pub(crate) struct DocumentSource {
    pub(crate) item_id: String,
    pub(crate) library_id: String,
    pub(crate) document_id: String,
    pub(crate) document_revision_id: i64,
    pub(crate) provider: String,
    pub(crate) text: String,
    pub(crate) body_end_byte: usize,
    pub(crate) bibliography_excluded: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ScientificChunk {
    pub(crate) id: String,
    pub(crate) ordinal: usize,
    pub(crate) text: String,
    pub(crate) character_start: usize,
    pub(crate) character_end: usize,
    pub(crate) token_count: usize,
    pub(crate) fingerprint: String,
    pub(crate) vector_key: u64,
}

#[derive(Clone, Copy, Debug)]
struct TextUnit {
    start: usize,
    end: usize,
}

pub(crate) fn scientific_chunks(
    provider: &dyn EmbeddingProvider,
    source: &DocumentSource,
) -> AppResult<Vec<ScientificChunk>> {
    scientific_chunks_with_maximum(provider, source, Some(MAXIMUM_CHUNKS))
}

pub(crate) fn scientific_chunks_with_maximum(
    provider: &dyn EmbeddingProvider,
    source: &DocumentSource,
    maximum_chunks: Option<usize>,
) -> AppResult<Vec<ScientificChunk>> {
    let body = &source.text[..source.body_end_byte];
    let mut units = natural_units(body);
    let mut expanded = Vec::new();
    for unit in units.drain(..) {
        let token_count = provider.token_count(body[unit.start..unit.end].trim())?;
        if token_count <= MAXIMUM_TOKENS {
            expanded.push(unit);
        } else {
            expanded.extend(split_oversized_unit(provider, body, unit, MAXIMUM_TOKENS)?);
        }
    }
    let mut chunks = Vec::new();
    let mut current: Vec<TextUnit> = Vec::new();
    for unit in expanded {
        let mut candidate = current.clone();
        candidate.push(unit);
        let candidate_text = unit_text(body, &candidate);
        let candidate_tokens = provider.token_count(&candidate_text)?;
        if !current.is_empty() && candidate_tokens > TARGET_TOKENS {
            push_chunk(provider, source, body, &current, &mut chunks)?;
            if maximum_chunks.is_some_and(|maximum| chunks.len() >= maximum) {
                break;
            }
            current = overlap_units(provider, body, &current, OVERLAP_TOKENS)?;
        }
        current.push(unit);
        while provider.token_count(&unit_text(body, &current))? > MAXIMUM_TOKENS
            && current.len() > 1
        {
            let first = current.remove(0);
            if current.is_empty() {
                current.push(first);
                break;
            }
        }
    }
    if !current.is_empty() && maximum_chunks.is_none_or(|maximum| chunks.len() < maximum) {
        push_chunk(provider, source, body, &current, &mut chunks)?;
    }
    chunks.dedup_by(|left, right| left.fingerprint == right.fingerprint);
    for (ordinal, chunk) in chunks.iter_mut().enumerate() {
        chunk.ordinal = ordinal;
        chunk.id = stable_semantic_id(
            "semantic-chunk",
            &[
                &source.document_revision_id.to_string(),
                CHUNKING_PROFILE_ID,
                &ordinal.to_string(),
                &chunk.fingerprint,
            ],
        );
    }
    Ok(chunks)
}

fn natural_units(text: &str) -> Vec<TextUnit> {
    let mut units = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let paragraph_break =
            bytes[index] == b'\n' && bytes.get(index + 1).is_some_and(|value| *value == b'\n');
        let sentence_break = matches!(bytes[index], b'.' | b'?' | b'!')
            && bytes
                .get(index + 1)
                .is_none_or(|value| value.is_ascii_whitespace());
        if paragraph_break || sentence_break {
            let end = if paragraph_break { index } else { index + 1 };
            if let Some(unit) = trimmed_unit(text, start, end) {
                units.push(unit);
            }
            start = if paragraph_break {
                index + 2
            } else {
                index + 1
            };
            if paragraph_break {
                index += 1;
            }
        }
        index += 1;
    }
    if let Some(unit) = trimmed_unit(text, start, text.len()) {
        units.push(unit);
    }
    units
}

fn split_oversized_unit(
    provider: &dyn EmbeddingProvider,
    text: &str,
    unit: TextUnit,
    maximum_tokens: usize,
) -> AppResult<Vec<TextUnit>> {
    let mut result = Vec::new();
    let mut start = unit.start;
    let mut last_boundary = start;
    for (offset, character) in text[unit.start..unit.end].char_indices() {
        if character.is_whitespace() {
            let boundary = unit.start + offset;
            if provider.token_count(text[start..boundary].trim())? >= maximum_tokens
                && last_boundary > start
            {
                result.push(TextUnit {
                    start,
                    end: last_boundary,
                });
                start = last_boundary;
            }
            last_boundary = boundary;
        }
    }
    if let Some(remainder) = trimmed_unit(text, start, unit.end) {
        result.push(remainder);
    }
    Ok(result)
}

fn trimmed_unit(text: &str, start: usize, end: usize) -> Option<TextUnit> {
    let value = text.get(start..end)?;
    let leading = value.len() - value.trim_start().len();
    let trailing = value.trim_end().len();
    (trailing > leading).then_some(TextUnit {
        start: start + leading,
        end: start + trailing,
    })
}

fn unit_text(text: &str, units: &[TextUnit]) -> String {
    let Some(first) = units.first() else {
        return String::new();
    };
    let end = units.last().map(|unit| unit.end).unwrap_or(first.end);
    text[first.start..end].trim().to_owned()
}

fn overlap_units(
    provider: &dyn EmbeddingProvider,
    text: &str,
    units: &[TextUnit],
    overlap_tokens: usize,
) -> AppResult<Vec<TextUnit>> {
    let mut overlap = Vec::new();
    for unit in units.iter().rev() {
        let mut candidate = vec![*unit];
        candidate.extend(overlap.iter().copied());
        if provider.token_count(&unit_text(text, &candidate))? > overlap_tokens {
            break;
        }
        overlap = candidate;
    }
    Ok(overlap)
}

fn push_chunk(
    provider: &dyn EmbeddingProvider,
    source: &DocumentSource,
    body: &str,
    units: &[TextUnit],
    output: &mut Vec<ScientificChunk>,
) -> AppResult<()> {
    let Some(first) = units.first() else {
        return Ok(());
    };
    let last = units.last().unwrap_or(first);
    let text = body[first.start..last.end].trim().to_owned();
    if text.is_empty() {
        return Ok(());
    }
    let character_start = body[..first.start].chars().count();
    let character_end = character_start + text.chars().count();
    let token_count = provider.token_count(&text)?;
    output.push(ScientificChunk {
        id: String::new(),
        ordinal: 0,
        fingerprint: text_fingerprint(&[&source.document_revision_id.to_string(), &text]),
        text,
        character_start,
        character_end,
        token_count,
        vector_key: 0,
    });
    Ok(())
}
