use std::sync::OnceLock;

use regex::Regex;

pub const EXTRACTOR_PROVIDER: &str = "graphite-conservative-text";
pub const EXTRACTOR_VERSION: &str = "1.0.0";
pub const RESOLVER_VERSION: &str = "1.0.0";

const MIN_REFERENCE_COUNT: usize = 4;
const MAX_REFERENCE_LENGTH: usize = 3_000;

#[derive(Clone, Debug)]
pub struct ExtractionInput<'a> {
    pub text: &'a str,
    pub completeness: &'a str,
}

#[derive(Clone, Debug)]
pub struct ExtractedReference {
    pub ordinal: u32,
    pub raw_text: String,
    pub original_doi: Option<String>,
    pub normalized_doi: Option<String>,
    pub title: Option<String>,
    pub year: Option<i32>,
    pub first_author: Option<String>,
    pub creators: Vec<String>,
    pub venue: Option<String>,
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub confidence: String,
}

#[derive(Clone, Debug)]
pub struct ReferenceExtraction {
    pub status: String,
    pub section_heading: Option<String>,
    pub section_start: Option<u64>,
    pub section_end: Option<u64>,
    pub references: Vec<ExtractedReference>,
    pub diagnostic: Option<String>,
}

pub trait ReferenceExtractionProvider: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn extract(&self, input: ExtractionInput<'_>) -> ReferenceExtraction;
}

#[derive(Clone, Default)]
pub struct ConservativeTextExtractor;

impl ReferenceExtractionProvider for ConservativeTextExtractor {
    fn provider_id(&self) -> &'static str {
        EXTRACTOR_PROVIDER
    }

    fn version(&self) -> &'static str {
        EXTRACTOR_VERSION
    }

    fn extract(&self, input: ExtractionInput<'_>) -> ReferenceExtraction {
        extract_references(input)
    }
}

pub fn extract_references(input: ExtractionInput<'_>) -> ReferenceExtraction {
    if input.text.trim().is_empty() {
        return unavailable_extraction("The indexed document text is empty");
    }

    let section = detect_reference_section(input.text);
    let Some(section) = section else {
        return ReferenceExtraction {
            status: if input.completeness == "partial" {
                "partial-source-text".to_owned()
            } else {
                "no-reference-section".to_owned()
            },
            section_heading: None,
            section_start: None,
            section_end: None,
            references: Vec::new(),
            diagnostic: Some(
                "No sufficiently convincing bibliography section was detected".to_owned(),
            ),
        };
    };

    let section_text = &input.text[section.content_start..section.end];
    let segmented = segment_entries(section_text, section.content_start);
    let references = segmented
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| parse_entry(index as u32 + 1, entry))
        .collect::<Vec<_>>();
    let enough_evidence = references.len() >= MIN_REFERENCE_COUNT;
    let status = if !enough_evidence {
        "low-confidence-section"
    } else if input.completeness == "partial" {
        "partial-source-text"
    } else {
        "references-extracted"
    };
    ReferenceExtraction {
        status: status.to_owned(),
        section_heading: section.heading,
        section_start: Some(section.content_start as u64),
        section_end: Some(section.end as u64),
        references,
        diagnostic: (!enough_evidence).then(|| {
            "A possible bibliography was found, but too few reliable entries were segmented"
                .to_owned()
        }),
    }
}

pub fn normalize_doi(value: &str) -> Option<String> {
    let captures = doi_regex().captures(value.trim())?;
    let mut doi = captures.get(1)?.as_str().trim().to_lowercase();
    doi = doi
        .trim_end_matches(['.', ',', ';', ':', ']', '}', '>', '"', '\''])
        .to_owned();
    while doi.ends_with(')') && doi.matches(')').count() > doi.matches('(').count() {
        doi.pop();
    }
    (!doi.is_empty()).then_some(doi)
}

pub fn normalize_title(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_lowercase().collect::<String>()
            } else {
                " ".to_owned()
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn normalize_author_surname(value: &str) -> String {
    value
        .trim()
        .trim_matches(|character: char| !character.is_alphanumeric())
        .to_lowercase()
}

pub fn title_token_similarity(reference_text: &str, candidate_title: &str) -> f64 {
    let reference = normalize_title(reference_text);
    let candidate = normalize_title(candidate_title);
    let candidate_tokens = significant_tokens(&candidate);
    if candidate_tokens.is_empty() {
        return 0.0;
    }
    if reference.contains(&candidate) && candidate.len() >= 12 {
        return 1.0;
    }
    let matched = candidate_tokens
        .iter()
        .filter(|token| reference.split_whitespace().any(|word| word == **token))
        .count();
    matched as f64 / candidate_tokens.len() as f64
}

fn significant_tokens(value: &str) -> Vec<&str> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "for", "from", "in", "of", "on", "the", "to", "using", "with",
    ];
    value
        .split_whitespace()
        .filter(|token| token.chars().count() > 2 && !STOP_WORDS.contains(token))
        .collect()
}

#[derive(Clone, Debug)]
struct Section {
    heading: Option<String>,
    content_start: usize,
    end: usize,
}

#[derive(Clone, Debug)]
struct SegmentedEntry {
    raw: String,
    start: usize,
    end: usize,
}

fn detect_reference_section(text: &str) -> Option<Section> {
    let lines = lines_with_offsets(text);
    let mut candidates = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let heading = line.text.trim();
        if !heading_regex().is_match(heading) {
            continue;
        }
        let end = next_section_boundary(&lines, index + 1, text.len());
        let content_start = line.end;
        let candidate_text = &text[content_start..end];
        let evidence = reference_like_line_count(candidate_text);
        if evidence >= MIN_REFERENCE_COUNT {
            candidates.push((evidence, line.start, heading.to_owned(), content_start, end));
        }
    }
    candidates
        .into_iter()
        .max_by_key(|(evidence, position, _, _, _)| (*evidence, *position))
        .map(|(_, _, heading, content_start, end)| Section {
            heading: Some(heading),
            content_start,
            end,
        })
        .or_else(|| detect_unheaded_numbered_section(text, &lines))
}

#[derive(Clone, Copy)]
struct TextLine<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

fn lines_with_offsets(text: &str) -> Vec<TextLine<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;
    for segment in text.split_inclusive('\n') {
        let end = start + segment.len();
        lines.push(TextLine {
            text: segment.trim_end_matches(['\r', '\n']),
            start,
            end,
        });
        start = end;
    }
    if start < text.len() {
        lines.push(TextLine {
            text: &text[start..],
            start,
            end: text.len(),
        });
    }
    lines
}

fn next_section_boundary(lines: &[TextLine<'_>], start: usize, fallback: usize) -> usize {
    lines[start..]
        .iter()
        .find(|line| end_heading_regex().is_match(line.text.trim()))
        .map(|line| line.start)
        .unwrap_or(fallback)
}

fn detect_unheaded_numbered_section(text: &str, lines: &[TextLine<'_>]) -> Option<Section> {
    let tail_start = text.len() * 55 / 100;
    let markers = lines
        .iter()
        .filter(|line| line.start >= tail_start)
        .filter_map(|line| {
            numbered_start_regex()
                .captures(line.text)
                .and_then(|captures| captures.get(1).or_else(|| captures.get(2)))
                .and_then(|value| value.as_str().parse::<u32>().ok())
                .map(|number| (number, line.start))
        })
        .collect::<Vec<_>>();
    if markers.len() < 6 {
        return None;
    }
    let sequential = markers
        .windows(2)
        .filter(|pair| pair[1].0 == pair[0].0 + 1)
        .count();
    if sequential * 4 < (markers.len() - 1) * 3 {
        return None;
    }
    Some(Section {
        heading: None,
        content_start: markers[0].1,
        end: text.len(),
    })
}

fn reference_like_line_count(value: &str) -> usize {
    value
        .lines()
        .filter(|line| {
            numbered_start_regex().is_match(line) || author_year_start_regex().is_match(line.trim())
        })
        .take(500)
        .count()
}

fn segment_entries(text: &str, base_offset: usize) -> Vec<SegmentedEntry> {
    let numbered = numbered_start_regex()
        .find_iter(text)
        .map(|matched| matched.start())
        .collect::<Vec<_>>();
    if numbered.len() >= MIN_REFERENCE_COUNT {
        return split_at_offsets(text, base_offset, &numbered);
    }

    let lines = lines_with_offsets(text);
    let starts = lines
        .iter()
        .filter(|line| author_year_start_regex().is_match(line.text.trim()))
        .map(|line| line.start)
        .collect::<Vec<_>>();
    if starts.len() >= MIN_REFERENCE_COUNT {
        return split_at_offsets(text, base_offset, &starts);
    }
    Vec::new()
}

fn split_at_offsets(text: &str, base_offset: usize, starts: &[usize]) -> Vec<SegmentedEntry> {
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(text.len());
            SegmentedEntry {
                raw: collapse_whitespace(&text[*start..end]),
                start: base_offset + *start,
                end: base_offset + end,
            }
        })
        .collect()
}

fn parse_entry(ordinal: u32, entry: SegmentedEntry) -> Option<ExtractedReference> {
    let raw = entry.raw.trim().to_owned();
    if raw.len() < 24 || raw.len() > MAX_REFERENCE_LENGTH || is_navigation_noise(&raw) {
        return None;
    }
    let doi_match = doi_regex()
        .find(&raw)
        .map(|value| value.as_str().to_owned());
    let normalized_doi = normalize_doi(&raw);
    let year = year_regex()
        .find(&raw)
        .and_then(|value| value.as_str().parse::<i32>().ok());
    let first_author = parse_first_author(&raw, year);
    if normalized_doi.is_none() && (year.is_none() || first_author.is_none()) {
        return None;
    }
    let title = parse_title(&raw);
    let venue = parse_venue(&raw, title.as_deref());
    let confidence = if normalized_doi.is_some()
        || (year.is_some() && first_author.is_some() && title.is_some())
    {
        "high"
    } else {
        "medium"
    };
    Some(ExtractedReference {
        ordinal,
        raw_text: raw,
        original_doi: doi_match,
        normalized_doi,
        title,
        year,
        creators: first_author.clone().into_iter().collect(),
        first_author,
        venue,
        start: Some(entry.start as u64),
        end: Some(entry.end as u64),
        confidence: confidence.to_owned(),
    })
}

fn parse_first_author(value: &str, year: Option<i32>) -> Option<String> {
    let without_marker = numbered_start_regex().replace(value, "");
    let prefix = year
        .and_then(|year| without_marker.find(&year.to_string()))
        .map(|index| &without_marker[..index])
        .unwrap_or(&without_marker);
    let candidate = prefix
        .split([',', ';'])
        .next()
        .unwrap_or(prefix)
        .split_whitespace()
        .find(|token| token.chars().any(char::is_alphabetic))?;
    let normalized = normalize_author_surname(candidate);
    (normalized.chars().count() >= 2).then_some(normalized)
}

fn parse_title(value: &str) -> Option<String> {
    let without_marker = numbered_start_regex().replace(value, "");
    let parts = sentence_split_regex()
        .split(&without_marker)
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    parts.into_iter().skip(1).find_map(|part| {
        let normalized = normalize_title(part);
        let token_count = normalized.split_whitespace().count();
        let invalid = part.to_lowercase().contains("doi")
            || year_regex().is_match(part) && token_count < 5
            || part.len() > 360;
        (!invalid && token_count >= 3).then(|| part.to_owned())
    })
}

fn parse_venue(value: &str, title: Option<&str>) -> Option<String> {
    let title = title?;
    let after_title = value
        .split_once(title)?
        .1
        .trim_start_matches(['.', ',', ' ']);
    let venue = sentence_split_regex()
        .split(after_title)
        .next()
        .unwrap_or_default()
        .trim();
    let tokens = venue.split_whitespace().count();
    (tokens >= 2 && venue.len() <= 180 && !venue.to_lowercase().contains("doi"))
        .then(|| venue.to_owned())
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_navigation_noise(value: &str) -> bool {
    let lower = value.to_lowercase();
    [
        "view record in scopus",
        "google scholar",
        "recommended articles",
        "copyright ©",
        "privacy policy",
    ]
    .iter()
    .any(|noise| lower.starts_with(noise))
}

fn unavailable_extraction(message: &str) -> ReferenceExtraction {
    ReferenceExtraction {
        status: "failed".to_owned(),
        section_heading: None,
        section_start: None,
        section_end: None,
        references: Vec::new(),
        diagnostic: Some(message.to_owned()),
    }
}

fn heading_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*(?:\d+[.)]?\s*)?(?:references(?:\s+and\s+notes)?|bibliography|literature\s+cited|works\s+cited)\s*$",
        )
        .expect("valid reference heading regex")
    })
}

fn end_heading_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(
            r"(?i)^\s*(?:appendix(?:\s+[a-z0-9]+)?|supplementary\s+(?:material|information)|author\s+contributions|acknowledg(?:e)?ments?)\s*$",
        )
        .expect("valid section end regex")
    })
}

fn numbered_start_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?m)^\s*(?:\[(\d{1,4})\]|(\d{1,4})[.)])\s+")
            .expect("valid numbered reference regex")
    })
}

fn author_year_start_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)^\s*\p{L}[\p{L}\p{M}'’-]*(?:\s+et\s+al\.|\s*,|\s+[A-Z].{0,70})?.{0,90}\b(?:18|19|20)\d{2}[a-z]?\b")
            .expect("valid author-year reference regex")
    })
}

fn doi_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:https?://(?:dx\.)?doi\.org/|doi\s*:\s*)?(10\.\d{4,9}/[-._;()/:A-Z0-9]+)",
        )
        .expect("valid DOI regex")
    })
}

fn year_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b(?:18|19|20)\d{2}\b").expect("valid year regex"))
}

fn sentence_split_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\.\s+").expect("valid sentence split regex"))
}

#[cfg(test)]
mod tests {
    use super::{extract_references, normalize_doi, title_token_similarity, ExtractionInput};

    #[test]
    fn extracts_numbered_multiline_references_and_normalizes_dois() {
        let text = r#"
Introduction
The DOI 10.9999/not-a-reference occurs in the introduction.

References
[1] Bensen, G. D., et al. Processing seismic ambient noise data.
    Geophysical Journal International 169, 1239-1260 (2007).
    https://doi.org/10.1111/j.1365-246X.2007.03374.x.
[2] Aki, K. Maximum likelihood estimate of b values. BSSA 43, 1965.
[3] Brenguier, F. Monitoring velocity changes using noise. Science 321, 2008.
[4] Sánchez-Sesma, F. J. Seismic interferometry and energy. GJI 180, 2010.
Appendix A
Not part of the bibliography.
"#;
        let extraction = extract_references(ExtractionInput {
            text,
            completeness: "full",
        });
        assert_eq!(extraction.status, "references-extracted");
        assert_eq!(extraction.references.len(), 4);
        assert_eq!(
            extraction.references[0].normalized_doi.as_deref(),
            Some("10.1111/j.1365-246x.2007.03374.x")
        );
        assert!(!extraction.references[0]
            .raw_text
            .contains("not-a-reference"));
    }

    #[test]
    fn extracts_author_year_bibliography_with_unicode_names() {
        let text = r#"
Methods and observations occupy most of this article.
Bibliography
Aki, K. (1965). Maximum likelihood estimate of b in the formula. BSSA 43, 237-239.
Brenguier, F., Campillo, M. (2008). Towards forecasting volcanic eruptions. Nature Geoscience.
Sánchez-Sesma, F. J. (2010). Energy partitions in seismic interferometry. GJI.
Šílený, J. (2012). Moment tensor inversion with uncertainty. Tectonophysics.
"#;
        let extraction = extract_references(ExtractionInput {
            text,
            completeness: "full",
        });
        assert_eq!(extraction.status, "references-extracted");
        assert_eq!(extraction.references.len(), 4);
        assert_eq!(
            extraction.references[2].first_author.as_deref(),
            Some("sánchez-sesma")
        );
    }

    #[test]
    fn accepts_a_strong_unheaded_sequential_bibliography() {
        let body = (1..=7)
            .map(|index| {
                format!(
                    "[{index}] Author{index}, A. A sufficiently long research title number {index}. Journal {index}, 20{index:02}."
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!(
            "Long body without a bibliography heading.\n{}\n{body}",
            "x ".repeat(500)
        );
        let extraction = extract_references(ExtractionInput {
            text: &text,
            completeness: "full",
        });
        assert_eq!(extraction.status, "references-extracted");
        assert!(extraction.section_heading.is_none());
    }

    #[test]
    fn rejects_body_dois_titles_and_false_reference_headings() {
        let text = r#"
References
This section references the control parameters used in Figure 2.
The title Ambient Noise Tomography is mentioned with DOI 10.5555/body.only.
No bibliography follows this heading.
Methods
Two papers share an author and publication year, but that is not evidence.
"#;
        let extraction = extract_references(ExtractionInput {
            text,
            completeness: "full",
        });
        assert_eq!(extraction.status, "no-reference-section");
        assert!(extraction.references.is_empty());
    }

    #[test]
    fn marks_partial_text_without_authoritative_edges() {
        let text = r#"
References
[1] Aki, K. A sufficiently descriptive seismic paper. BSSA, 1965.
[2] Bensen, G. A sufficiently descriptive noise paper. GJI, 2007.
[3] Campillo, M. A sufficiently descriptive correlation paper. Science, 2008.
[4] Sánchez, F. A sufficiently descriptive interferometry paper. GJI, 2010.
"#;
        let extraction = extract_references(ExtractionInput {
            text,
            completeness: "partial",
        });
        assert_eq!(extraction.status, "partial-source-text");
        assert_eq!(extraction.references.len(), 4);
    }

    #[test]
    fn normalizes_common_doi_forms_without_trailing_punctuation() {
        for value in [
            "10.1000/ABC.12",
            "doi:10.1000/ABC.12.",
            "https://doi.org/10.1000/ABC.12",
            "http://dx.doi.org/10.1000/ABC.12)",
        ] {
            assert_eq!(normalize_doi(value).as_deref(), Some("10.1000/abc.12"));
        }
    }

    #[test]
    fn scores_title_evidence_without_treating_similar_words_as_identity() {
        assert_eq!(
            title_token_similarity(
                "Bensen et al. Processing seismic ambient noise data. GJI.",
                "Processing seismic ambient noise data"
            ),
            1.0
        );
        assert!(
            title_token_similarity("Ambient noise observations", "Ambient noise tomography") < 0.75
        );
    }
}
