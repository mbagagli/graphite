use std::collections::BTreeMap;
use std::time::Instant;

use docling::{ConversionStatus, DoclingDocument, DocumentConverter, Node, SourceDocument, Table};
use graphite_pdf_benchmark_common::{
    canonical::canonical_from_benchmark, render_plain_text, run_adapter, Adapter,
    BenchmarkDocumentV1, BenchmarkElement, BenchmarkElementKind, BenchmarkExtractionEnvelopeV1,
    BenchmarkPage, BenchmarkProfile, BenchmarkRegion, BenchmarkRelationship, BenchmarkTable,
    BenchmarkTableCell, CandidateDescriptor, ExtractionObservations, ExtractionOutcome,
    ExtractionRequest, MappingLoss, OcrPageProvenanceObservation, TimingObservation,
    BENCHMARK_SCHEMA_VERSION,
};

fn main() {
    if let Err(error) = run_adapter(&mut DoclingAdapter) {
        eprintln!("docling benchmark adapter: {error}");
        std::process::exit(1);
    }
}

struct DoclingAdapter;

impl Adapter for DoclingAdapter {
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
        let availability_reason = if cfg!(feature = "ocr") {
            missing_runtime_asset()
        } else {
            None
        };
        CandidateDescriptor {
            candidate_id: "docling".to_owned(),
            version: "1.16.0".to_owned(),
            revision: "7f8de88e860e289c42873922bd888cd50815ef26".to_owned(),
            license: "MIT".to_owned(),
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
            features: if cfg!(feature = "ocr") {
                vec!["pdf-text".to_owned(), "pdf".to_owned()]
            } else {
                vec!["pdf-text".to_owned()]
            },
            supported_profiles: profiles,
            configured_available: availability_reason.is_none(),
            availability_reason,
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
        if !descriptor.configured_available {
            let reason = descriptor
                .availability_reason
                .clone()
                .unwrap_or_else(|| "Docling benchmark assets are unavailable".to_owned());
            return BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                descriptor,
                ExtractionOutcome::Unavailable,
                reason,
            );
        }
        configure_assets();
        let started = Instant::now();
        let source = match SourceDocument::from_file(&request.input_path) {
            Ok(source) => source,
            Err(error) => {
                return BenchmarkExtractionEnvelopeV1::terminal(
                    &request,
                    descriptor,
                    ExtractionOutcome::Failed,
                    format!("Docling could not load the PDF: {error}"),
                )
            }
        };
        let converter = converter_for(&request.profile);
        let result = match converter.convert(source) {
            Ok(result) => result,
            Err(error) => {
                let reason = error.to_string();
                let unavailable = reason.contains("model")
                    || reason.contains("PDFium")
                    || reason.contains("runtime")
                    || reason.contains("not compiled");
                return BenchmarkExtractionEnvelopeV1::terminal(
                    &request,
                    descriptor,
                    if unavailable {
                        ExtractionOutcome::Unavailable
                    } else {
                        ExtractionOutcome::Failed
                    },
                    format!("Docling extraction failed: {reason}"),
                );
            }
        };
        let provider_projection = result.document.export_to_markdown();
        let (document, mut mapping_losses) = map_docling_document(&result.document);
        let canonical = canonical_from_benchmark(
            &request.artifact_id,
            request.artifact_revision_id,
            &format!("docling:1.16.0:{}", request.profile.as_str()),
            &document,
            &provider_projection,
        );
        let projection = render_plain_text(&canonical);
        let outcome = match result.status {
            ConversionStatus::Success if projection.is_empty() => ExtractionOutcome::EmptySuccess,
            ConversionStatus::Success => ExtractionOutcome::Succeeded,
            ConversionStatus::PartialSuccess if projection.is_empty() => {
                ExtractionOutcome::EmptySuccess
            }
            ConversionStatus::PartialSuccess => ExtractionOutcome::Partial,
            ConversionStatus::Failure => ExtractionOutcome::Failed,
        };
        if document.pages.is_empty() {
            mapping_losses.push(MappingLoss {
                capability: "page-provenance".to_owned(),
                reason: "Docling emitted no explicit PageInfo markers for this output; page identity remains absent"
                    .to_owned(),
                source_field: Some("DoclingDocument.nodes".to_owned()),
            });
        }
        let replacement_characters = projection
            .chars()
            .filter(|character| *character == '\u{fffd}')
            .count();
        let encoding_warnings = if replacement_characters > 0 {
            vec![format!(
                "searchable projection contains {replacement_characters} replacement characters"
            )]
        } else {
            Vec::new()
        };
        let characters_by_page = document
            .pages
            .iter()
            .map(|page| page.text.chars().count() as u64)
            .collect::<Vec<_>>();
        let empty_pages = document
            .pages
            .iter()
            .filter(|page| page.text.trim().is_empty())
            .map(|page| page.page_number)
            .collect::<Vec<_>>();
        let ocr_page_provenance = result
            .document
            .confidence
            .as_ref()
            .map(|confidence| {
                confidence
                    .pages
                    .iter()
                    .filter_map(|(page, scores)| {
                        scores.ocr_score.map(|score| OcrPageProvenanceObservation {
                            page_number: *page as u32,
                            source: "ocr".to_owned(),
                            model: Some("docling-rs-ocr".to_owned()),
                            model_revision: Some("1.16.0".to_owned()),
                            render_dpi: None,
                            confidence: Some(score),
                            warnings: Vec::new(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let ocr_pages = ocr_page_provenance
            .iter()
            .map(|provenance| provenance.page_number)
            .collect::<Vec<_>>();
        let ocr_indices_reported =
            request.profile == BenchmarkProfile::Native || result.document.confidence.is_some();
        BenchmarkExtractionEnvelopeV1 {
            schema_version: BENCHMARK_SCHEMA_VERSION.to_owned(),
            request_id: request.request_id,
            document_id: request.document_id,
            input_sha256: request.input_sha256,
            candidate: descriptor,
            profile: request.profile.clone(),
            outcome,
            outcome_reason: None,
            searchable_projection: Some(projection.clone()),
            document: Some(document.clone()),
            canonical_document: Some(canonical),
            mapping_losses,
            warnings: if matches!(result.status, ConversionStatus::PartialSuccess) {
                vec!["Docling reported partial success".to_owned()]
            } else {
                Vec::new()
            },
            observations: ExtractionObservations {
                reported_page_count: document.reported_page_count,
                extracted_page_count: Some(document.pages.len() as u32),
                pages_with_text: document
                    .pages
                    .iter()
                    .filter(|page| !page.text.trim().is_empty())
                    .map(|page| page.page_number)
                    .collect(),
                empty_pages,
                native_text_present: (request.profile == BenchmarkProfile::Native).then_some(true),
                ocr_used: ocr_indices_reported.then_some(!ocr_pages.is_empty()),
                ocr_page_indices_reported: Some(ocr_indices_reported),
                ocr_pages,
                encoding_warnings,
                characters_by_page,
                provider_text_character_count: Some(provider_projection.chars().count() as u64),
                ocr_page_provenance,
                provider_quality: Some(format!("{:#?}", result.status)),
                ..ExtractionObservations::default()
            },
            timing: TimingObservation {
                wall_time_ms: started.elapsed().as_millis() as u64,
                ..TimingObservation::default()
            },
            diagnostics: BTreeMap::from([
                (
                    "conversionStatus".to_owned(),
                    serde_json::json!(format!("{:?}", result.status)),
                ),
                (
                    "markdownCharacters".to_owned(),
                    serde_json::json!(provider_projection.chars().count()),
                ),
                (
                    "mappedElements".to_owned(),
                    serde_json::json!(document.elements.len()),
                ),
                (
                    "mappedPages".to_owned(),
                    serde_json::json!(document.pages.len()),
                ),
            ]),
        }
    }
}

#[derive(Default)]
struct DoclingMappingState {
    current_page: Option<u32>,
    pages: BTreeMap<u32, BenchmarkPage>,
    elements: Vec<BenchmarkElement>,
    relationships: Vec<BenchmarkRelationship>,
    cell_geometry_dropped: bool,
}

fn map_docling_document(source: &DoclingDocument) -> (BenchmarkDocumentV1, Vec<MappingLoss>) {
    let mut state = DoclingMappingState::default();
    map_docling_nodes(&source.nodes, None, None, &mut state);
    populate_page_text_and_regions(&mut state);
    let pages = state.pages.into_values().collect::<Vec<_>>();
    let mut losses = vec![MappingLoss {
        capability: "inline-formatting".to_owned(),
        reason: "CanonicalDocument v1 preserves extracted text but has no parser-neutral inline-style model"
            .to_owned(),
        source_field: Some("Node::InlineGroup.runs".to_owned()),
    }];
    if state.cell_geometry_dropped {
        losses.push(MappingLoss {
            capability: "table-cell-bounding-boxes".to_owned(),
            reason: "CanonicalDocument v1 represents table cell text and spans but not cell-specific geometry"
                .to_owned(),
            source_field: Some("TableCell.bbox".to_owned()),
        });
    }
    let reported_page_count = pages.iter().map(|page| page.page_number).max();
    let reading_order = state
        .elements
        .iter()
        .map(|element| element.id.clone())
        .collect();
    (
        BenchmarkDocumentV1 {
            title: None,
            language: None,
            reported_page_count,
            pages,
            elements: state.elements,
            reading_order,
            relationships: state.relationships,
        },
        losses,
    )
}

fn map_docling_nodes(
    nodes: &[Node],
    parent_id: Option<&str>,
    inherited_location: Option<[u16; 4]>,
    state: &mut DoclingMappingState,
) {
    for node in nodes {
        map_docling_node(node, parent_id, inherited_location, state);
    }
}

fn map_docling_node(
    node: &Node,
    parent_id: Option<&str>,
    inherited_location: Option<[u16; 4]>,
    state: &mut DoclingMappingState,
) {
    match node {
        Node::PageInfo {
            page_no,
            width,
            height,
        } => {
            let Ok(page_number) = u32::try_from(*page_no) else {
                state.current_page = None;
                return;
            };
            if page_number == 0 {
                state.current_page = None;
                return;
            }
            state.current_page = Some(page_number);
            state.pages.entry(page_number).or_insert(BenchmarkPage {
                page_number,
                label: None,
                width: Some(f64::from(*width)),
                height: Some(f64::from(*height)),
                rotation_degrees: None,
                text: String::new(),
                regions: Vec::new(),
            });
        }
        Node::Located { location, inner } => {
            map_docling_node(inner, parent_id, Some(*location), state);
        }
        Node::Group { children, .. } => {
            map_docling_nodes(children, parent_id, inherited_location, state);
        }
        Node::Furniture { .. }
        | Node::PageFurniture { .. }
        | Node::PageBreak
        | Node::DoclangOnly(_) => {}
        Node::Heading { level, text } => {
            push_docling_element(
                BenchmarkElementKind::Heading,
                text.clone(),
                parent_id,
                Some(*level),
                inherited_location,
                None,
                None,
                None,
                None,
                state,
            );
        }
        Node::Paragraph { text } => {
            push_docling_paragraph(text, parent_id, inherited_location, state);
        }
        Node::CheckboxItem { text, .. } => {
            push_docling_element(
                BenchmarkElementKind::ListItem,
                text.clone(),
                parent_id,
                None,
                inherited_location,
                None,
                None,
                None,
                None,
                state,
            );
        }
        Node::ListItem {
            text,
            ordered,
            location,
            ..
        } => {
            let list_id = push_docling_element(
                BenchmarkElementKind::List,
                String::new(),
                parent_id,
                None,
                inherited_location,
                None,
                None,
                None,
                Some(*ordered),
                state,
            );
            push_docling_element(
                BenchmarkElementKind::ListItem,
                text.clone(),
                Some(&list_id),
                None,
                location.or(inherited_location),
                None,
                None,
                None,
                None,
                state,
            );
        }
        Node::Code { text, .. } => {
            push_docling_element(
                BenchmarkElementKind::Other,
                text.clone(),
                parent_id,
                None,
                inherited_location,
                None,
                None,
                None,
                None,
                state,
            );
        }
        Node::Table(table) => {
            push_docling_table(table, parent_id, inherited_location, state);
        }
        Node::Picture { caption, .. } => {
            push_docling_figure(caption.as_deref(), parent_id, inherited_location, state);
        }
        Node::Formula {
            latex,
            orig,
            location,
        } => {
            push_docling_element(
                BenchmarkElementKind::Formula,
                orig.clone(),
                parent_id,
                None,
                location.or(inherited_location),
                None,
                Some(latex.clone()),
                None,
                None,
                state,
            );
        }
        Node::Chart {
            table,
            caption,
            location,
            ..
        } => {
            let figure_id = push_docling_figure(
                caption.as_deref(),
                parent_id,
                location.or(inherited_location),
                state,
            );
            push_docling_table(
                table,
                Some(&figure_id),
                location.or(inherited_location),
                state,
            );
        }
        Node::FieldRegion { items } => {
            for item in items {
                let text = [
                    item.marker.as_deref(),
                    item.key.as_deref(),
                    item.value.as_deref(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
                push_docling_paragraph(&text, parent_id, inherited_location, state);
            }
        }
        Node::InlineGroup { md_text, .. } => {
            push_docling_paragraph(md_text, parent_id, inherited_location, state);
        }
        Node::TextDump(text) => {
            push_docling_paragraph(text, parent_id, inherited_location, state);
        }
    }
}

fn push_docling_paragraph(
    text: &str,
    parent_id: Option<&str>,
    location: Option<[u16; 4]>,
    state: &mut DoclingMappingState,
) -> String {
    push_docling_element(
        BenchmarkElementKind::Paragraph,
        text.to_owned(),
        parent_id,
        None,
        location,
        None,
        None,
        None,
        None,
        state,
    )
}

fn push_docling_table(
    table: &Table,
    parent_id: Option<&str>,
    inherited_location: Option<[u16; 4]>,
    state: &mut DoclingMappingState,
) -> String {
    let table_model = benchmark_table(table, state);
    let text = table
        .rows
        .iter()
        .map(|row| row.join("\t"))
        .collect::<Vec<_>>()
        .join("\n");
    let table_id = push_docling_element(
        BenchmarkElementKind::Table,
        text,
        parent_id,
        None,
        table.location.or(inherited_location),
        Some(table_model),
        None,
        None,
        None,
        state,
    );
    if let Some(caption) = table.caption.as_deref() {
        let caption_id = push_docling_element(
            BenchmarkElementKind::Caption,
            caption.to_owned(),
            parent_id,
            None,
            table.location.or(inherited_location),
            None,
            None,
            None,
            None,
            state,
        );
        state.relationships.push(BenchmarkRelationship {
            source_element_id: caption_id,
            target_element_id: table_id.clone(),
            relation: "caption-of".to_owned(),
        });
    }
    table_id
}

fn benchmark_table(table: &Table, state: &mut DoclingMappingState) -> BenchmarkTable {
    if let Some(cells) = table.cells.as_ref() {
        state.cell_geometry_dropped |= cells.iter().any(|cell| cell.bbox.is_some());
        let row_count = cells
            .iter()
            .map(|cell| cell.start_row + cell.row_span)
            .max()
            .unwrap_or(table.rows.len()) as u32;
        let column_count = cells
            .iter()
            .map(|cell| cell.start_col + cell.col_span)
            .max()
            .unwrap_or_else(|| table.rows.iter().map(Vec::len).max().unwrap_or_default())
            as u32;
        return BenchmarkTable {
            row_count,
            column_count,
            cells: cells
                .iter()
                .map(|cell| BenchmarkTableCell {
                    row_index: cell.start_row as u32,
                    column_index: cell.start_col as u32,
                    row_span: cell.row_span as u32,
                    column_span: cell.col_span as u32,
                    is_header: cell.column_header || cell.row_header || cell.row_section,
                    text: cell.text.clone(),
                })
                .collect(),
        };
    }
    BenchmarkTable {
        row_count: table.rows.len() as u32,
        column_count: table.rows.iter().map(Vec::len).max().unwrap_or_default() as u32,
        cells: table
            .rows
            .iter()
            .enumerate()
            .flat_map(|(row, cells)| {
                cells
                    .iter()
                    .enumerate()
                    .map(move |(column, text)| BenchmarkTableCell {
                        row_index: row as u32,
                        column_index: column as u32,
                        row_span: 1,
                        column_span: 1,
                        is_header: row == 0,
                        text: text.clone(),
                    })
            })
            .collect(),
    }
}

fn push_docling_figure(
    caption: Option<&str>,
    parent_id: Option<&str>,
    location: Option<[u16; 4]>,
    state: &mut DoclingMappingState,
) -> String {
    let asset_id = format!("docling-asset-{}", state.elements.len());
    let figure_id = push_docling_element(
        BenchmarkElementKind::Figure,
        String::new(),
        parent_id,
        None,
        location,
        None,
        None,
        Some(asset_id),
        None,
        state,
    );
    if let Some(caption) = caption {
        let caption_id = push_docling_element(
            BenchmarkElementKind::Caption,
            caption.to_owned(),
            parent_id,
            None,
            location,
            None,
            None,
            None,
            None,
            state,
        );
        state.relationships.push(BenchmarkRelationship {
            source_element_id: caption_id,
            target_element_id: figure_id.clone(),
            relation: "caption-of".to_owned(),
        });
    }
    figure_id
}

#[allow(clippy::too_many_arguments)]
fn push_docling_element(
    kind: BenchmarkElementKind,
    text: String,
    parent_id: Option<&str>,
    heading_level: Option<u8>,
    location: Option<[u16; 4]>,
    table: Option<BenchmarkTable>,
    formula_latex: Option<String>,
    figure_asset_id: Option<String>,
    list_ordered: Option<bool>,
    state: &mut DoclingMappingState,
) -> String {
    let id = format!("docling-element-{}", state.elements.len());
    let region = location.and_then(|location| normalized_docling_region(&id, location));
    state.elements.push(BenchmarkElement {
        id: id.clone(),
        kind,
        text,
        source_order: state.elements.len() as u32,
        page_number: state.current_page,
        parent_id: parent_id.map(str::to_owned),
        heading_level,
        region,
        table,
        formula_latex,
        figure_asset_id,
        list_ordered,
    });
    id
}

fn normalized_docling_region(element_id: &str, location: [u16; 4]) -> Option<BenchmarkRegion> {
    let [x0, y0, x1, y1] = location;
    if x1 < x0 || y1 < y0 || x1 > 512 || y1 > 512 {
        return None;
    }
    Some(BenchmarkRegion {
        element_id: Some(element_id.to_owned()),
        x: f64::from(x0) / 512.0,
        y: f64::from(y0) / 512.0,
        width: f64::from(x1 - x0) / 512.0,
        height: f64::from(y1 - y0) / 512.0,
    })
}

fn populate_page_text_and_regions(state: &mut DoclingMappingState) {
    for element in &state.elements {
        let Some(page_number) = element.page_number else {
            continue;
        };
        let Some(page) = state.pages.get_mut(&page_number) else {
            continue;
        };
        let text = element.table.as_ref().map_or_else(
            || element.text.clone(),
            |table| {
                table
                    .cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\t")
            },
        );
        if !text.is_empty() {
            if !page.text.is_empty() {
                page.text.push_str("\n\n");
            }
            page.text.push_str(&text);
        }
        if let Some(region) = element.region.clone() {
            page.regions.push(region);
        }
    }
}

fn configure_assets() {
    let Ok(root) = std::env::var("GRAPHITE_PDF_BENCH_ASSET_ROOT") else {
        return;
    };
    let root = std::path::Path::new(&root).join("docling");
    for (variable, relative) in [
        ("PDFIUM_DYNAMIC_LIB_PATH", ".pdfium/lib"),
        ("DOCLING_LAYOUT_ONNX", ".models/layout_heron.onnx"),
        ("DOCLING_OCR_REC_ONNX", ".models/ocr_rec.onnx"),
        ("DOCLING_OCR_DICT", ".models/ppocr_keys_v1.txt"),
        (
            "DOCLING_TABLEFORMER_ENCODER",
            ".models/tableformer/encoder.onnx",
        ),
        (
            "DOCLING_TABLEFORMER_DECODER",
            ".models/tableformer/decoder.onnx",
        ),
        ("DOCLING_TABLEFORMER_BBOX", ".models/tableformer/bbox.onnx"),
    ] {
        std::env::set_var(variable, root.join(relative));
    }
    let runtime = root
        .join(".runtime")
        .join(platform_slug())
        .join(onnxruntime_library_relative_path());
    if runtime.is_file() {
        std::env::set_var("ORT_DYLIB_PATH", runtime);
    }
}

fn missing_runtime_asset() -> Option<String> {
    let root = match std::env::var("GRAPHITE_PDF_BENCH_ASSET_ROOT") {
        Ok(root) => root,
        Err(_) => {
            return Some(
                "GRAPHITE_PDF_BENCH_ASSET_ROOT is required for Docling PDF profiles".to_owned(),
            )
        }
    };
    let root = std::path::Path::new(&root).join("docling");
    let required = [
        ".models/layout_heron.onnx",
        ".models/ocr_rec.onnx",
        ".models/ppocr_keys_v1.txt",
        ".models/tableformer/encoder.onnx",
        ".models/tableformer/decoder.onnx",
        ".models/tableformer/decoder.onnx.data",
        ".models/tableformer/bbox.onnx",
        ".models/tableformer/bbox.onnx.data",
    ];
    if let Some(relative) = required
        .iter()
        .find(|relative| !root.join(relative).is_file())
    {
        return Some(format!(
            "checksum-prepared Docling asset {relative} is absent"
        ));
    }
    if !root
        .join(".pdfium/lib")
        .join(pdfium_library_filename())
        .is_file()
    {
        return Some("checksum-prepared Docling PDFium library is absent".to_owned());
    }
    let runtime = root
        .join(".runtime")
        .join(platform_slug())
        .join(onnxruntime_library_relative_path());
    if !runtime.is_file() {
        return Some("checksum-prepared Docling ONNX Runtime is absent".to_owned());
    }
    None
}

fn pdfium_library_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "pdfium.dll"
    } else if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    }
}

fn platform_slug() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "aarch64") => "mac-arm64",
        _ => "unsupported-platform",
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn onnxruntime_library_relative_path() -> &'static str {
    "onnxruntime-linux-x64-1.28.0/lib/libonnxruntime.so"
}

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
fn onnxruntime_library_relative_path() -> &'static str {
    "onnxruntime-linux-aarch64-1.28.0/lib/libonnxruntime.so"
}

#[cfg(target_os = "macos")]
fn onnxruntime_library_relative_path() -> &'static str {
    "onnxruntime-osx-arm64-1.28.0/lib/libonnxruntime.dylib"
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn onnxruntime_library_relative_path() -> &'static str {
    "unsupported/libonnxruntime"
}

fn converter_for(profile: &BenchmarkProfile) -> DocumentConverter {
    match profile {
        BenchmarkProfile::Native => DocumentConverter::new().no_ocr(true),
        BenchmarkProfile::RecommendedAuto => DocumentConverter::new(),
        BenchmarkProfile::ForcedOcr => DocumentConverter::new().force_full_page_ocr(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphite_pdf_benchmark_common::canonical::validate_canonical_document;

    fn document(nodes: Vec<Node>) -> DoclingDocument {
        DoclingDocument {
            name: "fixture".to_owned(),
            nodes,
            strict_markdown: false,
            compact_tables: false,
            links: Vec::new(),
            confidence: None,
        }
    }

    #[test]
    fn explicit_page_and_geometry_survive_mapping() {
        let source = document(vec![
            Node::PageInfo {
                page_no: 7,
                width: 612.0,
                height: 792.0,
            },
            Node::Located {
                location: [64, 128, 320, 384],
                inner: Box::new(Node::Paragraph {
                    text: "explicit provenance".to_owned(),
                }),
            },
        ]);
        let (mapped, _) = map_docling_document(&source);
        assert_eq!(mapped.elements[0].page_number, Some(7));
        assert_eq!(mapped.pages[0].page_number, 7);
        assert_eq!(mapped.elements[0].region.as_ref().unwrap().x, 0.125);
        assert_eq!(mapped.elements[0].region.as_ref().unwrap().height, 0.5);
        let canonical = canonical_from_benchmark("artifact", 1, "docling:test", &mapped, "");
        validate_canonical_document(&canonical).unwrap();
        assert_eq!(canonical.blocks[0].page_range.as_ref().unwrap().start, 7);
        assert_eq!(canonical.blocks[0].regions[0].bounding_box.width, 0.5);
    }

    #[test]
    fn absent_page_provenance_is_not_fabricated() {
        let source = document(vec![Node::Paragraph {
            text: "no page marker".to_owned(),
        }]);
        let (mapped, _) = map_docling_document(&source);
        assert!(mapped.pages.is_empty());
        assert_eq!(mapped.elements[0].page_number, None);
        assert!(mapped.elements[0].region.is_none());
    }
}
