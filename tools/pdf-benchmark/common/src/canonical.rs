use std::collections::{BTreeMap, HashMap, HashSet};

use sha2::{Digest, Sha256};

use crate::model::{
    BenchmarkDocumentV1, BenchmarkElementKind, CanonicalAssetReference, CanonicalBlock,
    CanonicalBlockContent, CanonicalBlockRelationship, CanonicalDocumentMetadata,
    CanonicalDocumentV1, CanonicalPage, CanonicalTableCell, NormalizedBoundingBox, PageRange,
    PageRegion, CANONICAL_SCHEMA_VERSION,
};

pub fn canonical_unstructured(
    artifact_id: &str,
    artifact_revision_id: i64,
    extraction_profile_id: &str,
    text: &str,
    page_count: Option<u32>,
) -> CanonicalDocumentV1 {
    let block_id = stable_id(&[
        "canonical-block",
        artifact_id,
        &artifact_revision_id.to_string(),
        extraction_profile_id,
        CANONICAL_SCHEMA_VERSION,
        "0",
        "other",
    ]);
    CanonicalDocumentV1 {
        schema_version: CANONICAL_SCHEMA_VERSION.to_owned(),
        artifact_id: artifact_id.to_owned(),
        artifact_revision_id,
        extraction_profile_id: extraction_profile_id.to_owned(),
        metadata: CanonicalDocumentMetadata {
            reported_page_count_hint: page_count,
            ..CanonicalDocumentMetadata::default()
        },
        pages: Vec::new(),
        blocks: vec![CanonicalBlock {
            id: block_id.clone(),
            parent_id: None,
            source_order: 0,
            section_path: Vec::new(),
            page_range: None,
            regions: Vec::new(),
            content: CanonicalBlockContent::Other {
                label: "unstructured-text".to_owned(),
                text: text.to_owned(),
            },
        }],
        reading_order: vec![block_id],
        relationships: Vec::new(),
        assets: Vec::new(),
    }
}

pub fn canonical_from_benchmark(
    artifact_id: &str,
    artifact_revision_id: i64,
    extraction_profile_id: &str,
    source: &BenchmarkDocumentV1,
    fallback_text: &str,
) -> CanonicalDocumentV1 {
    if source.elements.is_empty() {
        return canonical_unstructured(
            artifact_id,
            artifact_revision_id,
            extraction_profile_id,
            fallback_text,
            source.reported_page_count,
        );
    }
    let pages = source
        .pages
        .iter()
        .map(|page| CanonicalPage {
            id: page_id(artifact_id, extraction_profile_id, page.page_number),
            physical_page_number: page.page_number,
            printed_label: page.label.clone(),
            width: page.width,
            height: page.height,
            rotation_degrees: page.rotation_degrees,
            coordinate_space: Some("normalized-top-left-after-rotation".to_owned()),
        })
        .collect::<Vec<_>>();
    let page_ids = pages
        .iter()
        .map(|page| (page.physical_page_number, page.id.clone()))
        .collect::<HashMap<_, _>>();
    let mut element_to_block = HashMap::new();
    for element in &source.elements {
        element_to_block.insert(
            element.id.clone(),
            stable_id(&[
                "canonical-block",
                artifact_id,
                &artifact_revision_id.to_string(),
                extraction_profile_id,
                CANONICAL_SCHEMA_VERSION,
                &element.source_order.to_string(),
                element_kind_name(&element.kind),
            ]),
        );
    }
    let elements_by_id = source
        .elements
        .iter()
        .map(|element| (element.id.as_str(), element))
        .collect::<HashMap<_, _>>();
    let blocks = source
        .elements
        .iter()
        .map(|element| {
            let regions = match (&element.region, element.page_number) {
                (Some(region), Some(number)) => page_ids
                    .get(&number)
                    .map(|page_id| {
                        vec![PageRegion {
                            page_id: page_id.clone(),
                            bounding_box: NormalizedBoundingBox {
                                x: region.x,
                                y: region.y,
                                width: region.width,
                                height: region.height,
                            },
                        }]
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            CanonicalBlock {
                id: element_to_block[&element.id].clone(),
                parent_id: element
                    .parent_id
                    .as_ref()
                    .and_then(|parent| element_to_block.get(parent))
                    .cloned(),
                source_order: element.source_order,
                section_path: heading_ancestors(element, &elements_by_id, &element_to_block),
                page_range: element.page_number.map(|page| PageRange {
                    start: page,
                    end: page,
                }),
                regions,
                content: match &element.kind {
                    BenchmarkElementKind::Heading => CanonicalBlockContent::Heading {
                        text: element.text.clone(),
                        level: element.heading_level.unwrap_or(1).clamp(1, 6),
                    },
                    BenchmarkElementKind::Paragraph => CanonicalBlockContent::Paragraph {
                        text: element.text.clone(),
                    },
                    BenchmarkElementKind::List => CanonicalBlockContent::List {
                        ordered: element.list_ordered.unwrap_or(false),
                    },
                    BenchmarkElementKind::ListItem => CanonicalBlockContent::ListItem {
                        text: element.text.clone(),
                    },
                    BenchmarkElementKind::Table if element.table.is_some() => {
                        let table = element.table.as_ref().expect("checked table");
                        CanonicalBlockContent::Table {
                            row_count: table.row_count,
                            column_count: table.column_count,
                            cells: table
                                .cells
                                .iter()
                                .map(|cell| CanonicalTableCell {
                                    row_index: cell.row_index,
                                    column_index: cell.column_index,
                                    row_span: cell.row_span,
                                    column_span: cell.column_span,
                                    is_header: cell.is_header,
                                    text: cell.text.clone(),
                                })
                                .collect(),
                        }
                    }
                    BenchmarkElementKind::Formula => CanonicalBlockContent::Formula {
                        plain_text: (!element.text.is_empty()).then(|| element.text.clone()),
                        latex: element.formula_latex.clone(),
                        mathml: None,
                        display: true,
                    },
                    BenchmarkElementKind::Figure => CanonicalBlockContent::Figure {
                        asset_id: element.figure_asset_id.clone().unwrap_or_else(|| {
                            stable_id(&[
                                "canonical-asset",
                                artifact_id,
                                extraction_profile_id,
                                &element.id,
                            ])
                        }),
                        alt_text: (!element.text.is_empty()).then(|| element.text.clone()),
                    },
                    BenchmarkElementKind::Caption => CanonicalBlockContent::Caption {
                        text: element.text.clone(),
                    },
                    BenchmarkElementKind::Reference => CanonicalBlockContent::Reference {
                        text: element.text.clone(),
                        identifiers: BTreeMap::new(),
                    },
                    other => CanonicalBlockContent::Other {
                        label: element_kind_name(other).to_owned(),
                        text: element.text.clone(),
                    },
                },
            }
        })
        .collect::<Vec<_>>();
    let reading_order = if source.reading_order.is_empty() {
        blocks.iter().map(|block| block.id.clone()).collect()
    } else {
        source
            .reading_order
            .iter()
            .filter_map(|id| element_to_block.get(id))
            .cloned()
            .collect()
    };
    let relationships = source
        .relationships
        .iter()
        .filter_map(|relationship| {
            Some(CanonicalBlockRelationship {
                source_block_id: element_to_block
                    .get(&relationship.source_element_id)?
                    .clone(),
                target_block_id: element_to_block
                    .get(&relationship.target_element_id)?
                    .clone(),
                relation: relationship.relation.clone(),
            })
        })
        .collect();
    let assets = source
        .elements
        .iter()
        .filter(|element| element.kind == BenchmarkElementKind::Figure)
        .map(|element| CanonicalAssetReference {
            id: element.figure_asset_id.clone().unwrap_or_else(|| {
                stable_id(&[
                    "canonical-asset",
                    artifact_id,
                    extraction_profile_id,
                    &element.id,
                ])
            }),
            media_type: "application/octet-stream".to_owned(),
            cache_relative_key: None,
            content_hash: None,
            byte_count: None,
            metadata: BTreeMap::new(),
        })
        .collect();
    CanonicalDocumentV1 {
        schema_version: CANONICAL_SCHEMA_VERSION.to_owned(),
        artifact_id: artifact_id.to_owned(),
        artifact_revision_id,
        extraction_profile_id: extraction_profile_id.to_owned(),
        metadata: CanonicalDocumentMetadata {
            extracted_title: source.title.clone(),
            language: source.language.clone(),
            reported_page_count_hint: source.reported_page_count,
            document_identifiers: BTreeMap::new(),
        },
        pages,
        blocks,
        reading_order,
        relationships,
        assets,
    }
}

pub fn validate_canonical_document(document: &CanonicalDocumentV1) -> Result<(), String> {
    if document.schema_version != CANONICAL_SCHEMA_VERSION {
        return Err(format!(
            "unsupported canonical schema version {}",
            document.schema_version
        ));
    }
    if document.artifact_id.is_empty() || document.artifact_revision_id <= 0 {
        return Err("canonical document identity is incomplete".to_owned());
    }
    let block_ids = document
        .blocks
        .iter()
        .map(|block| block.id.as_str())
        .collect::<HashSet<_>>();
    if block_ids.len() != document.blocks.len() {
        return Err("canonical block identifiers are not unique".to_owned());
    }
    if document
        .reading_order
        .iter()
        .any(|block_id| !block_ids.contains(block_id.as_str()))
    {
        return Err("canonical reading order references an unknown block".to_owned());
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
            return Err(format!(
                "canonical block {} has an unknown parent",
                block.id
            ));
        }
        if block
            .section_path
            .iter()
            .any(|heading_id| !block_ids.contains(heading_id.as_str()))
        {
            return Err(format!(
                "canonical block {} has an unknown section heading",
                block.id
            ));
        }
        for region in &block.regions {
            if !page_ids.contains(region.page_id.as_str()) {
                return Err(format!(
                    "canonical block {} references an unknown page",
                    block.id
                ));
            }
            validate_box(&region.bounding_box)?;
        }
    }
    Ok(())
}

pub fn render_plain_text(document: &CanonicalDocumentV1) -> String {
    if document.blocks.len() == 1 && document.reading_order.len() == 1 {
        let block = &document.blocks[0];
        if document.reading_order[0] == block.id {
            if let CanonicalBlockContent::Other { label, text } = &block.content {
                if label == "unstructured-text" {
                    return text.clone();
                }
            }
        }
    }
    let blocks = document
        .blocks
        .iter()
        .map(|block| (block.id.as_str(), block))
        .collect::<HashMap<_, _>>();
    document
        .reading_order
        .iter()
        .filter_map(|id| blocks.get(id.as_str()))
        .filter_map(|block| block_text(&block.content))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn canonical_hash(document: &CanonicalDocumentV1) -> Result<String, String> {
    validate_canonical_document(document)?;
    let value = serde_json::to_vec(document).map_err(|error| error.to_string())?;
    Ok(sha256_hex(&value))
}

pub fn stable_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hex_encode(&hasher.finalize())
}

fn page_id(artifact_id: &str, profile: &str, page: u32) -> String {
    stable_id(&["canonical-page", artifact_id, profile, &page.to_string()])
}

fn element_kind_name(kind: &BenchmarkElementKind) -> &'static str {
    match kind {
        BenchmarkElementKind::Heading => "heading",
        BenchmarkElementKind::Paragraph => "paragraph",
        BenchmarkElementKind::List => "list",
        BenchmarkElementKind::ListItem => "list-item",
        BenchmarkElementKind::Table => "table",
        BenchmarkElementKind::Formula => "formula",
        BenchmarkElementKind::Figure => "figure",
        BenchmarkElementKind::Caption => "caption",
        BenchmarkElementKind::Reference => "reference",
        BenchmarkElementKind::Other => "other",
    }
}

fn heading_ancestors(
    element: &crate::model::BenchmarkElement,
    elements: &HashMap<&str, &crate::model::BenchmarkElement>,
    block_ids: &HashMap<String, String>,
) -> Vec<String> {
    let mut ancestors = Vec::new();
    let mut parent = element.parent_id.as_deref();
    let mut visited = HashSet::new();
    while let Some(parent_id) = parent {
        if !visited.insert(parent_id) {
            break;
        }
        let Some(parent_element) = elements.get(parent_id) else {
            break;
        };
        if parent_element.kind == BenchmarkElementKind::Heading {
            if let Some(block_id) = block_ids.get(parent_id) {
                ancestors.push(block_id.clone());
            }
        }
        parent = parent_element.parent_id.as_deref();
    }
    ancestors.reverse();
    ancestors
}

fn validate_box(value: &NormalizedBoundingBox) -> Result<(), String> {
    let coordinates = [value.x, value.y, value.width, value.height];
    if coordinates
        .iter()
        .any(|coordinate| !coordinate.is_finite() || !(0.0..=1.0).contains(coordinate))
        || value.x + value.width > 1.0
        || value.y + value.height > 1.0
    {
        return Err("canonical bounding box is outside normalized page coordinates".to_owned());
    }
    Ok(())
}

fn block_text(content: &CanonicalBlockContent) -> Option<String> {
    match content {
        CanonicalBlockContent::Heading { text, .. }
        | CanonicalBlockContent::Paragraph { text }
        | CanonicalBlockContent::ListItem { text }
        | CanonicalBlockContent::Caption { text }
        | CanonicalBlockContent::Reference { text, .. }
        | CanonicalBlockContent::Other { text, .. } => Some(text.clone()),
        CanonicalBlockContent::Formula {
            plain_text,
            latex,
            mathml,
            ..
        } => plain_text
            .clone()
            .or_else(|| latex.clone())
            .or_else(|| mathml.clone()),
        CanonicalBlockContent::Figure { alt_text, .. } => alt_text.clone(),
        CanonicalBlockContent::Table { cells, .. } => Some(
            cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<Vec<_>>()
                .join("\t"),
        ),
        CanonicalBlockContent::List { .. } => None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unstructured_projection_is_byte_faithful() {
        let source = "  alpha\n\nmu = 3.2e-4\n";
        let document = canonical_unstructured("artifact", 1, "profile", source, Some(2));
        assert_eq!(render_plain_text(&document).as_bytes(), source.as_bytes());
        validate_canonical_document(&document).expect("valid canonical document");
    }

    #[test]
    fn shared_contract_fixture_validates_in_benchmark_model() {
        let fixture = include_str!("../../fixtures/canonical-unstructured.json");
        let document: CanonicalDocumentV1 =
            serde_json::from_str(fixture).expect("benchmark canonical fixture");
        validate_canonical_document(&document).expect("benchmark canonical validation");
        assert_eq!(
            render_plain_text(&document),
            "Line one.\n\n  Indented 3.2e-4\tvalue.\n"
        );
        assert_eq!(canonical_hash(&document).expect("canonical hash").len(), 64);
    }

    #[test]
    fn shared_mapped_fixture_validates_in_benchmark_model() {
        let fixture = include_str!("../../fixtures/canonical-mapped.json");
        let document: CanonicalDocumentV1 =
            serde_json::from_str(fixture).expect("benchmark mapped fixture");
        validate_canonical_document(&document).expect("benchmark mapped validation");
        assert_eq!(document.pages.len(), 1);
        assert_eq!(document.blocks.len(), 3);
    }
}
