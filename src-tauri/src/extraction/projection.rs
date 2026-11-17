use std::collections::{HashMap, HashSet};

use crate::database::content_hash;
use crate::error::{AppError, AppResult};

use super::model::{
    CanonicalBlock, CanonicalBlockContent, CanonicalDocumentV1, NormalizedBoundingBox,
    CANONICAL_SCHEMA_VERSION,
};

pub fn validate_canonical_document(document: &CanonicalDocumentV1) -> AppResult<()> {
    if document.schema_version != CANONICAL_SCHEMA_VERSION {
        return Err(AppError::ApplicationData(format!(
            "unsupported canonical schema version {}",
            document.schema_version
        )));
    }
    if document.artifact_id.is_empty() || document.artifact_revision_id <= 0 {
        return Err(AppError::ApplicationData(
            "canonical document identity is incomplete".to_owned(),
        ));
    }
    let block_ids = document
        .blocks
        .iter()
        .map(|block| block.id.as_str())
        .collect::<HashSet<_>>();
    if block_ids.len() != document.blocks.len() {
        return Err(AppError::ApplicationData(
            "canonical block identifiers are not unique".to_owned(),
        ));
    }
    if document
        .reading_order
        .iter()
        .any(|block_id| !block_ids.contains(block_id.as_str()))
    {
        return Err(AppError::ApplicationData(
            "canonical reading order references an unknown block".to_owned(),
        ));
    }
    let page_ids = document
        .pages
        .iter()
        .map(|page| page.id.as_str())
        .collect::<HashSet<_>>();
    for block in &document.blocks {
        if block
            .parent_id
            .as_deref()
            .is_some_and(|parent| !block_ids.contains(parent))
        {
            return Err(AppError::ApplicationData(format!(
                "canonical block {} has an unknown parent",
                block.id
            )));
        }
        if block
            .section_path
            .iter()
            .any(|heading_id| !block_ids.contains(heading_id.as_str()))
        {
            return Err(AppError::ApplicationData(format!(
                "canonical block {} has an unknown section heading",
                block.id
            )));
        }
        for region in &block.regions {
            if !page_ids.contains(region.page_id.as_str()) {
                return Err(AppError::ApplicationData(format!(
                    "canonical block {} references an unknown page",
                    block.id
                )));
            }
            validate_bounding_box(&region.bounding_box)?;
        }
    }
    Ok(())
}

fn validate_bounding_box(value: &NormalizedBoundingBox) -> AppResult<()> {
    let coordinates = [value.x, value.y, value.width, value.height];
    if coordinates
        .iter()
        .any(|coordinate| !coordinate.is_finite() || !(0.0..=1.0).contains(coordinate))
        || value.x + value.width > 1.0
        || value.y + value.height > 1.0
    {
        return Err(AppError::ApplicationData(
            "canonical bounding box is outside normalized page coordinates".to_owned(),
        ));
    }
    Ok(())
}

pub fn canonical_json(document: &CanonicalDocumentV1) -> AppResult<String> {
    validate_canonical_document(document)?;
    serde_json::to_string(document).map_err(|error| AppError::ApplicationData(error.to_string()))
}

pub fn canonical_hash(document: &CanonicalDocumentV1) -> AppResult<String> {
    Ok(content_hash(&canonical_json(document)?))
}

pub fn render_plain_text(document: &CanonicalDocumentV1) -> String {
    if let Some(text) = exact_unstructured_text(document) {
        return text.to_owned();
    }
    let blocks = blocks_by_id(document);
    document
        .reading_order
        .iter()
        .filter_map(|block_id| blocks.get(block_id.as_str()))
        .filter_map(|block| block_text(block))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn render_markdown(document: &CanonicalDocumentV1) -> String {
    if let Some(text) = exact_unstructured_text(document) {
        return text.to_owned();
    }
    let blocks = blocks_by_id(document);
    document
        .reading_order
        .iter()
        .filter_map(|block_id| blocks.get(block_id.as_str()))
        .map(|block| markdown_block(block))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn exact_unstructured_text(document: &CanonicalDocumentV1) -> Option<&str> {
    if document.blocks.len() != 1 || document.reading_order.len() != 1 {
        return None;
    }
    let block = &document.blocks[0];
    if document.reading_order[0] != block.id {
        return None;
    }
    match &block.content {
        CanonicalBlockContent::Other { label, text } if label == "unstructured-text" => Some(text),
        _ => None,
    }
}

fn blocks_by_id(document: &CanonicalDocumentV1) -> HashMap<&str, &CanonicalBlock> {
    document
        .blocks
        .iter()
        .map(|block| (block.id.as_str(), block))
        .collect()
}

fn block_text(block: &CanonicalBlock) -> Option<String> {
    match &block.content {
        CanonicalBlockContent::Table { cells, .. } => Some(
            cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<Vec<_>>()
                .join("\t"),
        ),
        CanonicalBlockContent::List { .. } => None,
        content => content.plain_text().map(str::to_owned),
    }
}

fn markdown_block(block: &CanonicalBlock) -> String {
    match &block.content {
        CanonicalBlockContent::Heading { text, level } => {
            format!("{} {text}", "#".repeat((*level).clamp(1, 6) as usize))
        }
        CanonicalBlockContent::List { ordered } => {
            if *ordered {
                "1.".to_owned()
            } else {
                "-".to_owned()
            }
        }
        CanonicalBlockContent::ListItem { text } => format!("- {text}"),
        CanonicalBlockContent::Table {
            row_count,
            column_count,
            cells,
        } => render_table(*row_count, *column_count, cells),
        CanonicalBlockContent::Formula {
            plain_text,
            latex,
            mathml,
            display,
        } => {
            if let Some(latex) = latex {
                if *display {
                    format!("$$\n{latex}\n$$")
                } else {
                    format!("${latex}$")
                }
            } else {
                plain_text
                    .as_deref()
                    .or(mathml.as_deref())
                    .unwrap_or_default()
                    .to_owned()
            }
        }
        CanonicalBlockContent::Figure { asset_id, alt_text } => {
            format!("![{}]({asset_id})", alt_text.as_deref().unwrap_or(""))
        }
        content => content.plain_text().unwrap_or_default().to_owned(),
    }
}

fn render_table(
    row_count: u32,
    column_count: u32,
    cells: &[super::model::CanonicalTableCell],
) -> String {
    if row_count == 0 || column_count == 0 {
        return String::new();
    }
    let mut rows = vec![vec![String::new(); column_count as usize]; row_count as usize];
    for cell in cells {
        if let Some(slot) = rows
            .get_mut(cell.row_index as usize)
            .and_then(|row| row.get_mut(cell.column_index as usize))
        {
            *slot = cell.text.replace('|', "\\|");
        }
    }
    let mut output = Vec::with_capacity(rows.len() + 1);
    output.push(format!("| {} |", rows[0].join(" | ")));
    output.push(format!(
        "| {} |",
        vec!["---"; column_count as usize].join(" | ")
    ));
    output.extend(
        rows.into_iter()
            .skip(1)
            .map(|row| format!("| {} |", row.join(" | "))),
    );
    output.join("\n")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::extraction::model::{
        CanonicalAssetReference, CanonicalBlockRelationship, CanonicalDocumentMetadata,
        CanonicalPage, CanonicalTableCell, PageRange, PageRegion,
    };

    #[test]
    fn round_trips_every_canonical_block_type() {
        let kinds = vec![
            CanonicalBlockContent::Heading {
                text: "Methods".to_owned(),
                level: 2,
            },
            CanonicalBlockContent::Paragraph {
                text: "We invert phase velocities.".to_owned(),
            },
            CanonicalBlockContent::List { ordered: true },
            CanonicalBlockContent::ListItem {
                text: "Prepare observations".to_owned(),
            },
            CanonicalBlockContent::Table {
                row_count: 1,
                column_count: 1,
                cells: vec![CanonicalTableCell {
                    row_index: 0,
                    column_index: 0,
                    row_span: 1,
                    column_span: 1,
                    is_header: true,
                    text: "Period (s)".to_owned(),
                }],
            },
            CanonicalBlockContent::Formula {
                plain_text: Some("c = lambda f".to_owned()),
                latex: Some("c = \\lambda f".to_owned()),
                mathml: None,
                display: true,
            },
            CanonicalBlockContent::Figure {
                asset_id: "asset-1".to_owned(),
                alt_text: Some("Dispersion curve".to_owned()),
            },
            CanonicalBlockContent::Caption {
                text: "Figure 1.".to_owned(),
            },
            CanonicalBlockContent::Reference {
                text: "A. Author (2020).".to_owned(),
                identifiers: BTreeMap::from([("doi".to_owned(), "10.1000/example".to_owned())]),
            },
            CanonicalBlockContent::Other {
                label: "footnote".to_owned(),
                text: "Supplementary detail".to_owned(),
            },
        ];
        let blocks = kinds
            .into_iter()
            .enumerate()
            .map(|(index, content)| CanonicalBlock {
                id: format!("block-{index}"),
                parent_id: None,
                source_order: index as u32,
                section_path: Vec::new(),
                page_range: Some(PageRange { start: 1, end: 1 }),
                regions: vec![PageRegion {
                    page_id: "page-1".to_owned(),
                    bounding_box: NormalizedBoundingBox {
                        x: 0.1,
                        y: 0.1,
                        width: 0.8,
                        height: 0.1,
                    },
                }],
                content,
            })
            .collect::<Vec<_>>();
        let reading_order = blocks.iter().map(|block| block.id.clone()).collect();
        let document = CanonicalDocumentV1 {
            schema_version: CANONICAL_SCHEMA_VERSION.to_owned(),
            artifact_id: "artifact-1".to_owned(),
            artifact_revision_id: 4,
            extraction_profile_id: "typed-fixture-v1".to_owned(),
            metadata: CanonicalDocumentMetadata {
                extracted_title: Some("Typed fixture".to_owned()),
                language: Some("en".to_owned()),
                reported_page_count_hint: Some(1),
                document_identifiers: BTreeMap::new(),
            },
            pages: vec![CanonicalPage {
                id: "page-1".to_owned(),
                physical_page_number: 1,
                printed_label: Some("1".to_owned()),
                width: Some(612.0),
                height: Some(792.0),
                rotation_degrees: Some(0),
                coordinate_space: Some("normalized-top-left".to_owned()),
            }],
            blocks,
            reading_order,
            relationships: vec![CanonicalBlockRelationship {
                source_block_id: "block-7".to_owned(),
                target_block_id: "block-6".to_owned(),
                relation: "caption-of".to_owned(),
            }],
            assets: vec![CanonicalAssetReference {
                id: "asset-1".to_owned(),
                media_type: "image/png".to_owned(),
                cache_relative_key: Some("assets/figure-1.png".to_owned()),
                content_hash: Some("fixture-hash".to_owned()),
                byte_count: Some(100),
                metadata: BTreeMap::new(),
            }],
        };
        let json = canonical_json(&document).expect("canonical JSON");
        let decoded: CanonicalDocumentV1 =
            serde_json::from_str(&json).expect("canonical round trip");
        assert_eq!(decoded, document);
        assert_eq!(canonical_json(&decoded).expect("stable JSON"), json);
        assert_eq!(decoded.blocks.len(), 10);
        assert!(render_markdown(&decoded).contains("## Methods"));
    }

    #[test]
    fn benchmark_contract_fixture_matches_production_canonical_v1() {
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tools/pdf-benchmark/fixtures/canonical-unstructured.json"
        ));
        let document: CanonicalDocumentV1 =
            serde_json::from_str(fixture).expect("benchmark canonical fixture");
        validate_canonical_document(&document).expect("production canonical validation");
        assert_eq!(
            render_plain_text(&document),
            "Line one.\n\n  Indented 3.2e-4\tvalue.\n"
        );
        assert_eq!(render_markdown(&document), render_plain_text(&document));
    }

    #[test]
    fn benchmark_mapped_fixture_matches_production_canonical_v1() {
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tools/pdf-benchmark/fixtures/canonical-mapped.json"
        ));
        let document: CanonicalDocumentV1 =
            serde_json::from_str(fixture).expect("benchmark mapped fixture");
        validate_canonical_document(&document).expect("production canonical validation");
        assert_eq!(document.pages.len(), 1);
        assert_eq!(document.blocks.len(), 3);
        assert_eq!(document.blocks[1].regions.len(), 1);
    }
}
