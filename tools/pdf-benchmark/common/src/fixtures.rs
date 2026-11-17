use std::fs;
use std::path::{Path, PathBuf};

use crate::model::{
    Anchor, AnchorKind, AnchorMatcher, AnchorSet, AnchorVerificationStatus, CorpusDocument,
    CorpusManifest, TextLayerKind, ZoteroTextSidecar, ANCHOR_SCHEMA_VERSION,
};
use crate::validation::{content_hash, write_json_atomic};

pub fn generate_synthetic_corpus(root: &Path) -> Result<CorpusManifest, String> {
    fs::create_dir_all(root.join("local"))
        .map_err(|error| format!("cannot create synthetic corpus: {error}"))?;
    fs::create_dir_all(root.join("anchors"))
        .map_err(|error| format!("cannot create synthetic anchors: {error}"))?;
    let fixtures = vec![
        SyntheticFixture::native(
            "native-two-column",
            vec![vec![
                TextPlacement::new(54, 750, "Methods"),
                TextPlacement::new(54, 720, "The native extraction anchor is alpha 42."),
                TextPlacement::new(320, 750, "Results"),
                TextPlacement::new(320, 720, "The second column follows the first."),
            ]],
            vec!["native", "two-column"],
            "The native extraction anchor is alpha 42.",
        ),
        SyntheticFixture::native(
            "complex-reading-order",
            vec![vec![
                TextPlacement::new(54, 750, "ORDER FIRST"),
                TextPlacement::new(320, 750, "ORDER THIRD"),
                TextPlacement::new(54, 700, "ORDER SECOND"),
            ]],
            vec!["native", "complex-layout"],
            "ORDER SECOND",
        ),
        SyntheticFixture::image(
            "scanned-page",
            "SCAN ANCHOR",
            vec!["scanned", "ocr"],
            "SCAN ANCHOR",
        ),
        SyntheticFixture::mixed(
            "mixed-native-scanned",
            "Native page anchor.",
            "MIXED OCR",
            vec!["mixed", "ocr"],
            "MIXED OCR",
        ),
        SyntheticFixture::native(
            "equation-heavy",
            vec![vec![
                TextPlacement::new(54, 750, "Wave equation"),
                TextPlacement::new(54, 710, "omega^2 = c^2 k^2"),
                TextPlacement::new(54, 680, "epsilon = 3.2e-4"),
            ]],
            vec!["native", "equation-heavy"],
            "epsilon = 3.2e-4",
        ),
        SyntheticFixture::native(
            "table-heavy",
            vec![vec![
                TextPlacement::new(54, 750, "Velocity table"),
                TextPlacement::new(54, 710, "Layer Velocity"),
                TextPlacement::new(54, 680, "A 327 m/s"),
                TextPlacement::new(54, 650, "B 512 m/s"),
            ]],
            vec!["native", "table-heavy"],
            "327 m/s",
        ),
        SyntheticFixture::native(
            "figure-caption",
            vec![vec![
                TextPlacement::new(54, 750, "Figure 1"),
                TextPlacement::new(54, 710, "Surface-wave dispersion curve at station A."),
            ]],
            vec!["native", "figure-caption-heavy"],
            "Surface-wave dispersion curve at station A.",
        ),
        SyntheticFixture::native(
            "font-ligature-encoding",
            vec![vec![
                TextPlacement::new(54, 750, "office affine coefficient"),
                TextPlacement::unicode_encoding_control(54, 710),
                TextPlacement::new(54, 670, "Encoding control anchor remains readable."),
            ]],
            vec!["native", "encoding", "ligature"],
            "Encoding control anchor remains readable.",
        ),
        SyntheticFixture::rotated(
            "rotated-content",
            "ROTATED ANCHOR 90 DEGREES",
            vec!["native", "rotated"],
        ),
        SyntheticFixture::native(
            "long-document",
            (1..=24)
                .map(|page| {
                    vec![
                        TextPlacement::new(54, 750, &format!("Long paper page {page}")),
                        TextPlacement::new(
                            54,
                            710,
                            if page == 24 {
                                "TAIL ANCHOR PRESERVED"
                            } else {
                                "Repeated scientific discussion with unique page context."
                            },
                        ),
                    ]
                })
                .collect(),
            vec!["native", "long"],
            "TAIL ANCHOR PRESERVED",
        ),
    ];
    let mut documents = Vec::new();
    for fixture in fixtures {
        let path = root.join("local").join(format!("{}.pdf", fixture.id));
        let bytes = fixture.pdf_bytes();
        fs::write(&path, &bytes)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        let sidecar_path = root
            .join("local")
            .join(format!("{}.zotero.json", fixture.id));
        let source_text = fixture.source_text();
        write_json_atomic(
            &sidecar_path,
            &ZoteroTextSidecar {
                schema_version: 2,
                document_id: Some(fixture.id.to_owned()),
                provider: "synthetic-zotero-baseline".to_owned(),
                source_version: 1,
                content: source_text.clone(),
                indexed_pages: Some(fixture.pages.len() as u32),
                total_pages: Some(fixture.pages.len() as u32),
                indexed_characters: Some(source_text.chars().count() as u64),
                total_characters: Some(source_text.chars().count() as u64),
                content_sha256: Some(content_hash(source_text.as_bytes())),
            },
        )?;
        let anchors_path = root.join("anchors").join(format!("{}.json", fixture.id));
        write_json_atomic(&anchors_path, &fixture.anchor_set())?;
        let anchors_sha256 = file_hash(&anchors_path)?;
        let zotero_baseline_sha256 = file_hash(&sidecar_path)?;
        documents.push(CorpusDocument {
            id: fixture.id.to_owned(),
            file: format!("local/{}.pdf", fixture.id),
            sha256: content_hash(&bytes),
            source_url: None,
            license: "CC0-1.0 generated fixture".to_owned(),
            redistributable: true,
            categories: fixture.categories.iter().map(ToString::to_string).collect(),
            language: "en".to_owned(),
            expected_pages: Some(fixture.pages.len() as u32),
            text_layer: fixture.text_layer,
            expected_ocr_pages: fixture.expected_ocr_pages,
            anchors: format!("anchors/{}.json", fixture.id),
            anchors_sha256,
            zotero_baseline: Some(format!("local/{}.zotero.json", fixture.id)),
            zotero_baseline_sha256: Some(zotero_baseline_sha256),
        });
    }
    let malformed = root.join("local/malformed.pdf");
    fs::write(&malformed, b"%PDF-1.7\nmalformed")
        .map_err(|error| format!("cannot write malformed fixture: {error}"))?;
    let truncated_source = build_pdf(&[PageSpec::text(
        vec![TextPlacement::new(54, 750, "TRUNCATED TAIL ANCHOR")],
        None,
    )]);
    fs::write(
        root.join("local/truncated.pdf"),
        &truncated_source[..truncated_source.len().saturating_sub(36)],
    )
    .map_err(|error| format!("cannot write truncated fixture: {error}"))?;
    fs::write(
        root.join("local/empty.pdf"),
        build_pdf(&[PageSpec::text(Vec::new(), None)]),
    )
    .map_err(|error| format!("cannot write empty fixture: {error}"))?;
    fs::write(root.join("local/encrypted.pdf"), encrypted_pdf())
        .map_err(|error| format!("cannot write encrypted fixture: {error}"))?;
    let manifest = CorpusManifest {
        schema_version: 1,
        corpus_id: "graphite-synthetic-pdf-smoke-v1".to_owned(),
        documents,
    };
    write_json_atomic(&root.join("synthetic-manifest.json"), &manifest)?;
    write_failure_manifest(root)?;
    write_protocol_manifest(root)?;
    Ok(manifest)
}

fn write_failure_manifest(root: &Path) -> Result<(), String> {
    let cases = [
        ("malformed", "malformed.pdf"),
        ("truncated", "truncated.pdf"),
        ("empty", "empty.pdf"),
        ("encrypted", "encrypted.pdf"),
    ];
    let mut documents = Vec::new();
    for (id, file) in cases {
        let bytes = fs::read(root.join("local").join(file))
            .map_err(|error| format!("cannot read failure fixture {file}: {error}"))?;
        let anchors = AnchorSet {
            schema_version: ANCHOR_SCHEMA_VERSION,
            document_id: id.to_owned(),
            anchors: Vec::new(),
        };
        let anchors_path = root.join("anchors").join(format!("{id}.json"));
        write_json_atomic(&anchors_path, &anchors)?;
        documents.push(CorpusDocument {
            id: id.to_owned(),
            file: format!("local/{file}"),
            sha256: content_hash(&bytes),
            source_url: None,
            license: "CC0-1.0 generated fixture".to_owned(),
            redistributable: true,
            categories: vec!["failure-mode".to_owned(), id.to_owned()],
            language: "en".to_owned(),
            expected_pages: None,
            text_layer: TextLayerKind::Broken,
            expected_ocr_pages: Vec::new(),
            anchors: format!("anchors/{id}.json"),
            anchors_sha256: file_hash(&anchors_path)?,
            zotero_baseline: None,
            zotero_baseline_sha256: None,
        });
    }
    write_json_atomic(
        &root.join("failure-manifest.json"),
        &CorpusManifest {
            schema_version: 1,
            corpus_id: "graphite-synthetic-pdf-failures-v1".to_owned(),
            documents,
        },
    )
}

fn write_protocol_manifest(root: &Path) -> Result<(), String> {
    let bytes = build_pdf(&[PageSpec::text(
        vec![TextPlacement::new(54, 750, "PROTOCOL CONTROL")],
        None,
    )]);
    fs::write(root.join("local/protocol-control.pdf"), &bytes)
        .map_err(|error| format!("cannot write protocol fixture: {error}"))?;
    let mut documents = Vec::new();
    for id in [
        "success",
        "partial",
        "empty-success",
        "unsupported",
        "unavailable",
        "failed",
        "timeout",
        "crash",
    ] {
        let anchors_path = root.join("anchors").join(format!("protocol-{id}.json"));
        write_json_atomic(
            &anchors_path,
            &AnchorSet {
                schema_version: ANCHOR_SCHEMA_VERSION,
                document_id: id.to_owned(),
                anchors: Vec::new(),
            },
        )?;
        documents.push(CorpusDocument {
            id: id.to_owned(),
            file: "local/protocol-control.pdf".to_owned(),
            sha256: content_hash(&bytes),
            source_url: None,
            license: "CC0-1.0 generated fixture".to_owned(),
            redistributable: true,
            categories: vec!["protocol-control".to_owned(), id.to_owned()],
            language: "en".to_owned(),
            expected_pages: Some(1),
            text_layer: TextLayerKind::Native,
            expected_ocr_pages: Vec::new(),
            anchors: format!("anchors/protocol-{id}.json"),
            anchors_sha256: file_hash(&anchors_path)?,
            zotero_baseline: None,
            zotero_baseline_sha256: None,
        });
    }
    write_json_atomic(
        &root.join("protocol-manifest.json"),
        &CorpusManifest {
            schema_version: 1,
            corpus_id: "graphite-pdf-benchmark-protocol-v1".to_owned(),
            documents,
        },
    )
}

fn file_hash(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot checksum {}: {error}", path.display()))?;
    Ok(content_hash(&bytes))
}

#[derive(Clone)]
struct SyntheticFixture {
    id: &'static str,
    pages: Vec<PageSpec>,
    categories: Vec<&'static str>,
    required_anchor: String,
    text_layer: TextLayerKind,
    expected_ocr_pages: Vec<u32>,
}

impl SyntheticFixture {
    fn native(
        id: &'static str,
        pages: Vec<Vec<TextPlacement>>,
        categories: Vec<&'static str>,
        anchor: &str,
    ) -> Self {
        Self {
            id,
            pages: pages
                .into_iter()
                .map(|placements| PageSpec::text(placements, None))
                .collect(),
            categories,
            required_anchor: anchor.to_owned(),
            text_layer: TextLayerKind::Native,
            expected_ocr_pages: Vec::new(),
        }
    }

    fn image(id: &'static str, text: &str, categories: Vec<&'static str>, anchor: &str) -> Self {
        Self {
            id,
            pages: vec![PageSpec::image(text)],
            categories,
            required_anchor: anchor.to_owned(),
            text_layer: TextLayerKind::Scanned,
            expected_ocr_pages: vec![1],
        }
    }

    fn mixed(
        id: &'static str,
        native: &str,
        image: &str,
        categories: Vec<&'static str>,
        anchor: &str,
    ) -> Self {
        Self {
            id,
            pages: vec![
                PageSpec::text(vec![TextPlacement::new(54, 750, native)], None),
                PageSpec::image(image),
            ],
            categories,
            required_anchor: anchor.to_owned(),
            text_layer: TextLayerKind::Mixed,
            expected_ocr_pages: vec![2],
        }
    }

    fn rotated(id: &'static str, text: &str, categories: Vec<&'static str>) -> Self {
        Self {
            id,
            pages: vec![PageSpec::text(
                vec![TextPlacement::new(54, 750, text)],
                Some(90),
            )],
            categories,
            required_anchor: text.to_owned(),
            text_layer: TextLayerKind::Native,
            expected_ocr_pages: Vec::new(),
        }
    }

    fn pdf_bytes(&self) -> Vec<u8> {
        build_pdf(&self.pages)
    }

    fn source_text(&self) -> String {
        self.pages
            .iter()
            .map(PageSpec::ground_truth_text)
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn anchor_set(&self) -> AnchorSet {
        if self.id == "complex-reading-order" {
            return AnchorSet {
                schema_version: ANCHOR_SCHEMA_VERSION,
                document_id: self.id.to_owned(),
                anchors: vec![
                    ordered_anchor("order-first", "ORDER FIRST", None, Some("order-second")),
                    ordered_anchor(
                        "order-second",
                        "ORDER SECOND",
                        Some("order-first"),
                        Some("order-third"),
                    ),
                    ordered_anchor("order-third", "ORDER THIRD", Some("order-second"), None),
                ],
            };
        }
        AnchorSet {
            schema_version: ANCHOR_SCHEMA_VERSION,
            document_id: self.id.to_owned(),
            anchors: vec![Anchor {
                id: format!("{}-required", self.id),
                kind: if self.categories.contains(&"table-heavy") {
                    AnchorKind::TableValue
                } else if self.categories.contains(&"equation-heavy") {
                    AnchorKind::Formula
                } else if self.categories.contains(&"figure-caption-heavy") {
                    AnchorKind::FigureCaption
                } else {
                    AnchorKind::TextFact
                },
                required: true,
                verification_status: AnchorVerificationStatus::HumanVerified,
                verification_note: "deterministic generated fixture ground truth".to_owned(),
                expected_pages: if self.expected_ocr_pages.is_empty() {
                    Vec::new()
                } else {
                    self.expected_ocr_pages.clone()
                },
                matcher: AnchorMatcher::Normalized {
                    value: self.required_anchor.clone(),
                },
                expected_after: None,
                expected_before: None,
            }],
        }
    }
}

fn ordered_anchor(
    id: &str,
    value: &str,
    expected_after: Option<&str>,
    expected_before: Option<&str>,
) -> Anchor {
    Anchor {
        id: id.to_owned(),
        kind: AnchorKind::ReadingOrderPair,
        required: true,
        verification_status: AnchorVerificationStatus::HumanVerified,
        verification_note: "deterministic generated fixture ground truth".to_owned(),
        expected_pages: Vec::new(),
        matcher: AnchorMatcher::Exact {
            value: value.to_owned(),
        },
        expected_after: expected_after.map(ToOwned::to_owned),
        expected_before: expected_before.map(ToOwned::to_owned),
    }
}

#[derive(Clone)]
struct TextPlacement {
    x: i32,
    y: i32,
    text: String,
}

impl TextPlacement {
    fn new(x: i32, y: i32, text: &str) -> Self {
        Self {
            x,
            y,
            text: text.to_owned(),
        }
    }

    fn unicode_encoding_control(x: i32, y: i32) -> Self {
        let ligature = char::from_u32(0xfb03).expect("valid ligature scalar");
        let omega = char::from_u32(0x03a9).expect("valid Greek scalar");
        let acute = char::from_u32(0x00e9).expect("valid accented scalar");
        Self {
            x,
            y,
            text: format!("Ligature: coe{ligature}cient; Unicode: caf{acute}; {omega}."),
        }
    }
}

#[derive(Clone)]
enum PageSpec {
    Text {
        placements: Vec<TextPlacement>,
        rotation: Option<i16>,
    },
    Image {
        label: String,
    },
}

impl PageSpec {
    fn text(placements: Vec<TextPlacement>, rotation: Option<i16>) -> Self {
        Self::Text {
            placements,
            rotation,
        }
    }

    fn image(label: &str) -> Self {
        Self::Image {
            label: label.to_owned(),
        }
    }

    fn ground_truth_text(&self) -> String {
        match self {
            Self::Text { placements, .. } => placements
                .iter()
                .map(|placement| placement.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            Self::Image { label } => label.clone(),
        }
    }
}

fn build_pdf(pages: &[PageSpec]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = Vec::new();
    objects.push(Vec::new());
    objects.push(Vec::new());
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    let font_id = 3usize;
    let mut page_ids = Vec::new();
    for page in pages {
        let image_id = if let PageSpec::Image { label } = page {
            let (width, height, pixels) = raster_text(label);
            let id = objects.len() + 1;
            objects.push(image_object(width, height, &pixels));
            Some(id)
        } else {
            None
        };
        let content_id = objects.len() + 1;
        let content = match page {
            PageSpec::Text { placements, .. } => text_stream(placements),
            PageSpec::Image { .. } => b"q 500 0 0 125 56 330 cm /Im1 Do Q\n".to_vec(),
        };
        objects.push(stream_object(&content));
        let page_id = objects.len() + 1;
        let rotation = match page {
            PageSpec::Text {
                rotation: Some(value),
                ..
            } => format!(" /Rotate {value}"),
            _ => String::new(),
        };
        let resources = image_id.map_or_else(
            || format!("/Font << /F1 {font_id} 0 R >>"),
            |id| format!("/XObject << /Im1 {id} 0 R >>"),
        );
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792]{rotation} \
                 /Resources << {resources} >> /Contents {content_id} 0 R >>"
            )
            .into_bytes(),
        );
        page_ids.push(page_id);
    }
    objects[1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        page_ids
            .iter()
            .map(|id| format!("{id} 0 R"))
            .collect::<Vec<_>>()
            .join(" "),
        page_ids.len()
    )
    .into_bytes();
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    serialize_pdf(&objects)
}

fn text_stream(placements: &[TextPlacement]) -> Vec<u8> {
    let mut output = Vec::new();
    for placement in placements {
        let escaped = placement
            .text
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        output.extend_from_slice(
            format!(
                "BT /F1 12 Tf 1 0 0 1 {} {} Tm ({escaped}) Tj ET\n",
                placement.x, placement.y
            )
            .as_bytes(),
        );
    }
    output
}

fn raster_text(label: &str) -> (usize, usize, Vec<u8>) {
    const SCALE: usize = 8;
    const GLYPH_WIDTH: usize = 5;
    const GLYPH_HEIGHT: usize = 7;
    const SPACING: usize = 2;
    const MARGIN: usize = 8;
    let glyphs = label.chars().collect::<Vec<_>>();
    let width = MARGIN * 2 + glyphs.len() * (GLYPH_WIDTH + SPACING) * SCALE - SPACING * SCALE;
    let height = MARGIN * 2 + GLYPH_HEIGHT * SCALE;
    let mut pixels = vec![255; width * height];
    for (glyph_index, character) in glyphs.into_iter().enumerate() {
        let pattern = glyph_pattern(character);
        for (row, bits) in pattern.iter().enumerate() {
            for column in 0..GLYPH_WIDTH {
                if bits & (1 << (GLYPH_WIDTH - column - 1)) == 0 {
                    continue;
                }
                let left = MARGIN + glyph_index * (GLYPH_WIDTH + SPACING) * SCALE + column * SCALE;
                let top = MARGIN + row * SCALE;
                for y in top..top + SCALE {
                    for x in left..left + SCALE {
                        pixels[y * width + x] = 0;
                    }
                }
            }
        }
    }
    (width, height, pixels)
}

fn glyph_pattern(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        _ => [0; 7],
    }
}

fn image_object(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    let mut object = format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} \
         /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {} >>\nstream\n",
        pixels.len()
    )
    .into_bytes();
    object.extend_from_slice(pixels);
    object.extend_from_slice(b"\nendstream");
    object
}

fn stream_object(content: &[u8]) -> Vec<u8> {
    let mut object = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    object.extend_from_slice(content);
    object.extend_from_slice(b"endstream");
    object
}

fn serialize_pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    serialize_pdf_with_trailer(objects, "")
}

fn serialize_pdf_with_trailer(objects: &[Vec<u8>], trailer_fields: &str) -> Vec<u8> {
    let mut output = b"%PDF-1.7\n%Graphite synthetic benchmark fixture\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        output.extend_from_slice(object);
        output.extend_from_slice(b"\nendobj\n");
    }
    let xref = output.len();
    output.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    output.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {trailer_fields} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1,
        )
        .as_bytes(),
    );
    output
}

fn encrypted_pdf() -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [5 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        stream_object(b"BT /F1 12 Tf 54 750 Td (ENCRYPTED CONTROL) Tj ET\n"),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
          /Resources << /Font << /F1 3 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
    ];
    objects.push(
        b"<< /Filter /Standard /V 1 /R 2 /Length 40 /P -4 \
          /O <0000000000000000000000000000000000000000000000000000000000000000> \
          /U <1111111111111111111111111111111111111111111111111111111111111111> >>"
            .to_vec(),
    );
    serialize_pdf_with_trailer(
        &objects,
        "/Encrypt 6 0 R /ID [<0123456789abcdef0123456789abcdef> \
         <0123456789abcdef0123456789abcdef>]",
    )
}

pub fn synthetic_auxiliary_paths(root: &Path) -> Vec<PathBuf> {
    [
        "malformed.pdf",
        "truncated.pdf",
        "empty.pdf",
        "encrypted.pdf",
    ]
    .into_iter()
    .map(|name| root.join("local").join(name))
    .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::validation::validate_manifest;

    #[test]
    fn generated_manifest_has_ten_checksum_valid_documents() {
        let directory = tempdir().expect("temp directory");
        let manifest = generate_synthetic_corpus(directory.path()).expect("generate corpus");
        assert_eq!(manifest.documents.len(), 10);
        validate_manifest(&manifest, directory.path()).expect("valid manifest");
        let scanned = std::fs::read(directory.path().join("local/scanned-page.pdf"))
            .expect("scanned fixture");
        assert!(!scanned
            .windows(b"SCAN ANCHOR".len())
            .any(|value| value == b"SCAN ANCHOR"));
        assert!(scanned
            .windows(b"/Subtype /Image".len())
            .any(|value| { value == b"/Subtype /Image" }));
        let failures: CorpusManifest =
            crate::validation::read_json(&directory.path().join("failure-manifest.json"))
                .expect("failure manifest");
        assert_eq!(failures.documents.len(), 4);
        validate_manifest(&failures, directory.path()).expect("valid failure manifest");
        let protocol: CorpusManifest =
            crate::validation::read_json(&directory.path().join("protocol-manifest.json"))
                .expect("protocol manifest");
        assert_eq!(protocol.documents.len(), 8);
        validate_manifest(&protocol, directory.path()).expect("valid protocol manifest");
    }
}
