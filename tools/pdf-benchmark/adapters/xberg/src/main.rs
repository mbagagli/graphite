use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::time::Instant;

use graphite_pdf_benchmark_common::canonical::canonical_from_benchmark;
use graphite_pdf_benchmark_common::{
    render_plain_text, run_adapter, Adapter, BenchmarkDocumentV1, BenchmarkElement,
    BenchmarkElementKind, BenchmarkExtractionEnvelopeV1, BenchmarkPage, BenchmarkProfile,
    BenchmarkRelationship, BenchmarkTable, BenchmarkTableCell, CandidateDescriptor,
    ExtractionObservations, ExtractionOutcome, ExtractionRequest, MappingLoss, TimingObservation,
    BENCHMARK_SCHEMA_VERSION,
};
use xberg::{
    extract, DocumentStructure, ElementType, ExtractInput, ExtractionConfig, ExtractionMethod,
    NodeContent, PageConfig, RelationshipKind, ResultFormat,
};

fn main() {
    if let Err(error) = run_adapter(&mut XbergAdapter::new()) {
        eprintln!("xberg benchmark adapter: {error}");
        std::process::exit(1);
    }
}

struct XbergAdapter {
    runtime: tokio::runtime::Runtime,
}

impl XbergAdapter {
    fn new() -> Self {
        Self {
            runtime: tokio::runtime::Runtime::new().expect("create Xberg adapter runtime"),
        }
    }
}

impl Adapter for XbergAdapter {
    fn descriptor(&self) -> CandidateDescriptor {
        let profiles = if cfg!(feature = "ocr") {
            vec![
                BenchmarkProfile::Native,
                BenchmarkProfile::RecommendedAuto,
                BenchmarkProfile::ForcedOcr,
            ]
        } else {
            vec![BenchmarkProfile::Native]
        };
        CandidateDescriptor {
            candidate_id: "xberg".to_owned(),
            version: "1.0.14".to_owned(),
            revision: "b6660a759d31089075dd89112b77b918f872879a".to_owned(),
            license: "MIT".to_owned(),
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
            features: if cfg!(feature = "ocr") {
                vec![
                    "tokio-runtime".to_owned(),
                    "pdf".to_owned(),
                    "ocr".to_owned(),
                ]
            } else {
                vec!["tokio-runtime".to_owned(), "pdf".to_owned()]
            },
            supported_profiles: profiles,
            configured_available: true,
            availability_reason: None,
        }
    }

    fn extract(&mut self, request: ExtractionRequest) -> BenchmarkExtractionEnvelopeV1 {
        if request.profile != BenchmarkProfile::Native && !cfg!(feature = "ocr") {
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                self.descriptor(),
                ExtractionOutcome::Unavailable,
                "adapter was built without the isolated ocr feature",
            );
        }
        let descriptor = self.descriptor();
        let started = Instant::now();
        let mut config = ExtractionConfig {
            use_cache: false,
            pages: Some(PageConfig {
                extract_pages: true,
                insert_page_markers: false,
                ..PageConfig::default()
            }),
            result_format: ResultFormat::ElementBased,
            include_document_structure: true,
            ..ExtractionConfig::default()
        };
        match request.profile {
            BenchmarkProfile::Native => config.disable_ocr = true,
            BenchmarkProfile::RecommendedAuto => {
                if let Err(reason) = configure_tesseract_assets() {
                    return BenchmarkExtractionEnvelopeV1::terminal(
                        &request,
                        descriptor,
                        ExtractionOutcome::Unavailable,
                        reason,
                    );
                }
                config.disable_ocr = false;
            }
            BenchmarkProfile::ForcedOcr => {
                if let Err(reason) = configure_tesseract_assets() {
                    return BenchmarkExtractionEnvelopeV1::terminal(
                        &request,
                        descriptor,
                        ExtractionOutcome::Unavailable,
                        reason,
                    );
                }
                config.disable_ocr = false;
                config.force_ocr_pages = Some(request.forced_ocr_pages.clone());
                config.force_ocr = request.forced_ocr_pages.is_empty();
            }
        }
        let output = self.runtime.block_on(extract(
            ExtractInput::from_uri(request.input_path.clone()),
            &config,
        ));
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                let reason = error.to_string();
                let unavailable = reason.contains("dependency")
                    || reason.contains("Tesseract")
                    || reason.contains("OCR")
                    || reason.contains("model");
                return BenchmarkExtractionEnvelopeV1::terminal(
                    &request,
                    descriptor,
                    if unavailable {
                        ExtractionOutcome::Unavailable
                    } else {
                        ExtractionOutcome::Failed
                    },
                    format!("Xberg extraction failed: {reason}"),
                );
            }
        };
        let Some(extracted) = output.results.into_iter().next() else {
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                descriptor,
                ExtractionOutcome::EmptySuccess,
                "Xberg returned no document result",
            );
        };
        let pages = extracted
            .pages
            .as_ref()
            .map(|pages| {
                pages
                    .iter()
                    .map(|page| BenchmarkPage {
                        page_number: page.page_number,
                        label: None,
                        width: None,
                        height: None,
                        rotation_degrees: None,
                        text: page.content.clone(),
                        regions: Vec::new(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let (elements, relationships, mut mapping_losses) = extracted.document.as_ref().map_or_else(
            || {
                (
                    map_flat_elements(extracted.elements.as_deref().unwrap_or_default()),
                    Vec::new(),
                    vec![MappingLoss {
                        capability: "document-tree".to_owned(),
                        reason: "Xberg emitted no DocumentStructure; the adapter retained its explicit flat elements"
                            .to_owned(),
                        source_field: Some("ExtractedDocument.elements".to_owned()),
                    }],
                )
            },
            map_document_structure,
        );
        if elements.iter().any(|element| element.region.is_none()) {
            mapping_losses.push(MappingLoss {
                capability: "bounding-boxes".to_owned(),
                reason: "Xberg node coordinates are source-space values, but the provider output exposes no page dimensions needed for CanonicalDocument normalized coordinates"
                    .to_owned(),
                source_field: Some("DocumentNode.bbox".to_owned()),
            });
        }
        let document = BenchmarkDocumentV1 {
            title: None,
            language: extracted
                .detected_languages
                .as_ref()
                .and_then(|languages| languages.first().cloned()),
            reported_page_count: (extracted.counts.pages > 0)
                .then_some(extracted.counts.pages as u32),
            pages,
            reading_order: elements.iter().map(|element| element.id.clone()).collect(),
            elements,
            relationships,
        };
        let canonical = canonical_from_benchmark(
            &request.artifact_id,
            request.artifact_revision_id,
            &format!("xberg:1.0.14:{}", request.profile.as_str()),
            &document,
            &extracted.content,
        );
        let projection = render_plain_text(&canonical);
        let empty_pages = document
            .pages
            .iter()
            .filter(|page| page.text.trim().is_empty())
            .map(|page| page.page_number)
            .collect::<Vec<_>>();
        let warnings = extracted
            .processing_warnings
            .iter()
            .map(|warning| format!("{warning:?}"))
            .collect::<Vec<_>>();
        let suspicious_pages = if projection.len() < extracted.content.len() / 2 {
            document.pages.iter().map(|page| page.page_number).collect()
        } else {
            Vec::new()
        };
        let ocr_used = extracted
            .extraction_method
            .is_some_and(ExtractionMethod::used_ocr);
        let ocr_recommended_pages = extracted
            .metadata
            .format
            .as_ref()
            .and_then(|metadata| match metadata {
                xberg::FormatMetadata::Pdf(metadata) => Some(metadata),
                _ => None,
            })
            .and_then(|metadata| metadata.scanned_pages.clone())
            .unwrap_or_default();
        let outcome = if projection.is_empty() {
            ExtractionOutcome::EmptySuccess
        } else if !warnings.is_empty() || !suspicious_pages.is_empty() {
            ExtractionOutcome::Suspicious
        } else {
            ExtractionOutcome::Succeeded
        };
        BenchmarkExtractionEnvelopeV1 {
            schema_version: BENCHMARK_SCHEMA_VERSION.to_owned(),
            request_id: request.request_id,
            document_id: request.document_id,
            input_sha256: request.input_sha256,
            candidate: descriptor,
            profile: request.profile.clone(),
            outcome,
            outcome_reason: None,
            searchable_projection: Some(projection),
            document: Some(document.clone()),
            canonical_document: Some(canonical),
            mapping_losses,
            warnings: warnings.clone(),
            observations: ExtractionObservations {
                reported_page_count: (extracted.counts.pages > 0)
                    .then_some(extracted.counts.pages as u32),
                extracted_page_count: Some(document.pages.len() as u32),
                pages_with_text: document
                    .pages
                    .iter()
                    .filter(|page| !page.text.trim().is_empty())
                    .map(|page| page.page_number)
                    .collect(),
                empty_pages,
                suspicious_pages,
                native_text_present: extracted.extraction_method.map(|method| {
                    matches!(method, ExtractionMethod::Native | ExtractionMethod::Mixed)
                }),
                ocr_used: Some(ocr_used),
                ocr_page_indices_reported: Some(request.profile == BenchmarkProfile::Native),
                ocr_pages: Vec::new(),
                ocr_recommended_pages,
                encoding_warnings: warnings
                    .iter()
                    .filter(|warning| warning.to_lowercase().contains("encoding"))
                    .cloned()
                    .collect(),
                layout_warnings: warnings
                    .iter()
                    .filter(|warning| warning.to_lowercase().contains("layout"))
                    .cloned()
                    .collect(),
                characters_by_page: document
                    .pages
                    .iter()
                    .map(|page| page.text.chars().count() as u64)
                    .collect(),
                provider_text_character_count: Some(extracted.content.chars().count() as u64),
                structure_retention_warnings: Vec::new(),
                provider_quality: extracted.quality_score.map(|score| score.to_string()),
                ..ExtractionObservations::default()
            },
            timing: TimingObservation {
                wall_time_ms: started.elapsed().as_millis() as u64,
                ..TimingObservation::default()
            },
            diagnostics: BTreeMap::from([
                (
                    "tableCount".to_owned(),
                    serde_json::json!(extracted.counts.tables),
                ),
                (
                    "imageCount".to_owned(),
                    serde_json::json!(extracted.counts.images),
                ),
                (
                    "providerContentCharacters".to_owned(),
                    serde_json::json!(extracted.content.chars().count()),
                ),
                (
                    "ocrPageProvenance".to_owned(),
                    serde_json::json!(if request.profile == BenchmarkProfile::Native {
                        "not-applicable"
                    } else {
                        "not-reported-by-provider"
                    }),
                ),
            ]),
        }
    }
}

fn map_flat_elements(elements: &[xberg::Element]) -> Vec<BenchmarkElement> {
    elements
        .iter()
        .enumerate()
        .map(|(index, element)| BenchmarkElement {
            id: format!("xberg-element-{index}"),
            kind: map_element_type(element.element_type),
            text: element.text.clone(),
            source_order: index as u32,
            page_number: element.metadata.page_number,
            parent_id: None,
            heading_level: match element.element_type {
                ElementType::Title => Some(1),
                ElementType::Heading => Some(2),
                _ => None,
            },
            region: None,
            table: None,
            formula_latex: None,
            figure_asset_id: None,
            list_ordered: None,
        })
        .collect()
}

fn map_document_structure(
    structure: &DocumentStructure,
) -> (
    Vec<BenchmarkElement>,
    Vec<BenchmarkRelationship>,
    Vec<MappingLoss>,
) {
    let ids = structure
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            if node.id.is_empty() {
                format!("xberg-node-{index}")
            } else {
                node.id.clone()
            }
        })
        .collect::<Vec<_>>();
    let mut elements = structure
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            let mapped = map_node_content(&node.content, index)?;
            Some(BenchmarkElement {
                id: ids[index].clone(),
                kind: mapped.kind,
                text: mapped.text,
                source_order: index as u32,
                page_number: node.page,
                parent_id: node
                    .parent
                    .and_then(|parent| ids.get(parent.0 as usize))
                    .cloned(),
                heading_level: mapped.heading_level,
                region: None,
                table: mapped.table,
                formula_latex: None,
                figure_asset_id: mapped.figure_asset_id,
                list_ordered: mapped.list_ordered,
            })
        })
        .collect::<Vec<_>>();
    let relationships = structure
        .relationships
        .iter()
        .filter_map(|relationship| {
            let source = ids.get(relationship.source.0 as usize)?.clone();
            let target = ids.get(relationship.target.0 as usize)?.clone();
            if relationship.kind == RelationshipKind::Caption {
                if let Some(element) = elements.iter_mut().find(|element| element.id == source) {
                    element.kind = BenchmarkElementKind::Caption;
                }
            }
            Some(BenchmarkRelationship {
                source_element_id: source,
                target_element_id: target,
                relation: map_relationship_kind(relationship.kind).to_owned(),
            })
        })
        .collect();
    let mut losses = Vec::new();
    if structure
        .nodes
        .iter()
        .any(|node| !node.annotations.is_empty())
    {
        losses.push(MappingLoss {
            capability: "inline-formatting".to_owned(),
            reason: "CanonicalDocument v1 has no parser-neutral inline-style model".to_owned(),
            source_field: Some("DocumentNode.annotations".to_owned()),
        });
    }
    if structure.nodes.iter().any(|node| {
        matches!(&node.content, NodeContent::Table { grid } if grid.cells.iter().any(|cell| cell.bbox.is_some()))
    }) {
        losses.push(MappingLoss {
            capability: "table-cell-bounding-boxes".to_owned(),
            reason: "CanonicalDocument v1 retains cell text and spans but has no cell-specific geometry field"
                .to_owned(),
            source_field: Some("GridCell.bbox".to_owned()),
        });
    }
    (elements, relationships, losses)
}

struct MappedNode {
    kind: BenchmarkElementKind,
    text: String,
    heading_level: Option<u8>,
    table: Option<BenchmarkTable>,
    figure_asset_id: Option<String>,
    list_ordered: Option<bool>,
}

fn map_node_content(content: &NodeContent, index: usize) -> Option<MappedNode> {
    let mapped = match content {
        NodeContent::Title { text } => mapped_text(BenchmarkElementKind::Heading, text, Some(1)),
        NodeContent::Heading { level, text } => {
            mapped_text(BenchmarkElementKind::Heading, text, Some(*level))
        }
        NodeContent::Paragraph { text } => mapped_text(BenchmarkElementKind::Paragraph, text, None),
        NodeContent::List { ordered } => MappedNode {
            kind: BenchmarkElementKind::List,
            text: String::new(),
            heading_level: None,
            table: None,
            figure_asset_id: None,
            list_ordered: Some(*ordered),
        },
        NodeContent::ListItem { text } => mapped_text(BenchmarkElementKind::ListItem, text, None),
        NodeContent::Table { grid } => MappedNode {
            kind: BenchmarkElementKind::Table,
            text: grid
                .cells
                .iter()
                .map(|cell| cell.content.as_str())
                .collect::<Vec<_>>()
                .join("\t"),
            heading_level: None,
            table: Some(BenchmarkTable {
                row_count: grid.rows,
                column_count: grid.cols,
                cells: grid
                    .cells
                    .iter()
                    .map(|cell| BenchmarkTableCell {
                        row_index: cell.row,
                        column_index: cell.col,
                        row_span: cell.row_span,
                        column_span: cell.col_span,
                        is_header: cell.is_header,
                        text: cell.content.clone(),
                    })
                    .collect(),
            }),
            figure_asset_id: None,
            list_ordered: None,
        },
        NodeContent::Image { description, .. } => MappedNode {
            kind: BenchmarkElementKind::Figure,
            text: description.clone().unwrap_or_default(),
            heading_level: None,
            table: None,
            figure_asset_id: Some(format!("xberg-image-{index}")),
            list_ordered: None,
        },
        NodeContent::Code { text, .. } => mapped_text(BenchmarkElementKind::Other, text, None),
        NodeContent::Formula { text } => mapped_text(BenchmarkElementKind::Formula, text, None),
        NodeContent::Footnote { text } => mapped_text(BenchmarkElementKind::Other, text, None),
        NodeContent::DefinitionItem { term, definition } => mapped_text(
            BenchmarkElementKind::Other,
            &format!("{term}: {definition}"),
            None,
        ),
        NodeContent::Citation { text, .. } => {
            mapped_text(BenchmarkElementKind::Reference, text, None)
        }
        NodeContent::RawBlock { content, .. } => {
            mapped_text(BenchmarkElementKind::Other, content, None)
        }
        NodeContent::MetadataBlock { entries } => mapped_text(
            BenchmarkElementKind::Other,
            &entries
                .iter()
                .map(|(key, value)| format!("{key}: {value}"))
                .collect::<Vec<_>>()
                .join("\n"),
            None,
        ),
        NodeContent::Group {
            heading_text,
            heading_level,
            label,
        } => {
            let text = heading_text
                .as_ref()
                .or(label.as_ref())
                .cloned()
                .unwrap_or_default();
            mapped_text(BenchmarkElementKind::Heading, &text, *heading_level)
        }
        NodeContent::Slide { title, .. } => mapped_text(
            BenchmarkElementKind::Heading,
            title.as_deref().unwrap_or_default(),
            Some(1),
        ),
        NodeContent::Admonition { title, kind } => mapped_text(
            BenchmarkElementKind::Other,
            title.as_deref().unwrap_or(kind),
            None,
        ),
        NodeContent::Quote | NodeContent::PageBreak | NodeContent::DefinitionList => return None,
    };
    Some(mapped)
}

fn mapped_text(kind: BenchmarkElementKind, text: &str, heading_level: Option<u8>) -> MappedNode {
    MappedNode {
        kind,
        text: text.to_owned(),
        heading_level,
        table: None,
        figure_asset_id: None,
        list_ordered: None,
    }
}

fn map_relationship_kind(kind: RelationshipKind) -> &'static str {
    match kind {
        RelationshipKind::Caption => "caption-of",
        RelationshipKind::FootnoteReference => "footnote-of",
        RelationshipKind::CitationReference
        | RelationshipKind::InternalLink
        | RelationshipKind::Label
        | RelationshipKind::TocEntry
        | RelationshipKind::CrossReference => "reference-target",
    }
}

fn configure_tesseract_assets() -> Result<(), String> {
    let asset_root = env::var("GRAPHITE_PDF_BENCH_ASSET_ROOT").map_err(|_| {
        "GRAPHITE_PDF_BENCH_ASSET_ROOT is required for Xberg OCR profiles".to_owned()
    })?;
    let tessdata = PathBuf::from(asset_root).join("xberg/tessdata");
    if !tessdata.join("eng.traineddata").is_file() {
        return Err(format!(
            "checksum-prepared Xberg OCR data is absent at {}",
            tessdata.display()
        ));
    }
    env::set_var("TESSDATA_PREFIX", tessdata);
    Ok(())
}

fn map_element_type(value: ElementType) -> BenchmarkElementKind {
    match value {
        ElementType::Title | ElementType::Heading => BenchmarkElementKind::Heading,
        ElementType::NarrativeText | ElementType::BlockQuote => BenchmarkElementKind::Paragraph,
        ElementType::ListItem => BenchmarkElementKind::ListItem,
        ElementType::Table => BenchmarkElementKind::Table,
        ElementType::Image => BenchmarkElementKind::Figure,
        ElementType::CodeBlock
        | ElementType::PageBreak
        | ElementType::Footer
        | ElementType::Header => BenchmarkElementKind::Other,
    }
}
