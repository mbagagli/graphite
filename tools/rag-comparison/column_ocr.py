"""Constrain local OCR to explicitly confirmed page columns.

This optional adapter changes OCR regions only. Docling still owns text
recognition, layout assembly and page coordinates. Use this adapter for
a single inspected page, never automatic whole-library replacement.

Functions:
    column_bounds: Validate and divide a page into equal-width regions.
    column_pipeline: Build the pinned Docling pipeline with column OCR.
"""

import math
from pathlib import Path


def column_bounds(
    width: float, height: float, columns: int,
) -> list[tuple[float, float, float, float]]:
    """Divide a page into non-overlapping top-left-coordinate regions.

    Confirm equal-width columns from the page image first. This helper
    does not infer gutters or modify recognized text.

    Args:
        width: Positive page width in PDF points.
        height: Positive page height in PDF points.
        columns: Explicit count of two or three columns.

    Returns:
        list[tuple]: Region coordinates in reading order.
    """
    if (not math.isfinite(width) or not math.isfinite(height)
            or width <= 0 or height <= 0
            or not isinstance(columns, int) or columns not in (2, 3)):
        raise ValueError("Require positive finite dimensions and 2-3 columns.")
    return [(width * i / columns, 0, width * (i + 1) / columns, height)
            for i in range(columns)]


def column_pipeline(columns: int) -> type:
    """Build a local pipeline using the existing RapidOCR model assets.

    Optional imports keep geometry tests independent of model packages.
    The private factory hook is pinned to the installed Docling release;
    a real parser smoke run is required when upgrading that dependency.

    Args:
        columns: Explicit count of confirmed equal-width columns.

    Returns:
        type: A Docling pipeline class with bounded OCR input regions.
    """
    from docling.datamodel.base_models import Page
    from docling.models.stages.ocr.rapid_ocr_model import RapidOcrModel
    from docling.pipeline.standard_pdf_pipeline import StandardPdfPipeline
    from docling_core.types.doc import BoundingBox, CoordOrigin

    class ColumnOcrModel(RapidOcrModel):
        """Read confirmed columns in original page coordinates."""

        def get_ocr_rects(self, page: Page) -> list[BoundingBox]:
            """Return explicit column regions for an original PDF page.

            The inherited OCR implementation translates every recognized
            line back to the original physical page coordinate system.

            Args:
                page: Docling page with physical dimensions.

            Returns:
                list: Bounding boxes in left-to-right reading order.
            """
            if page.size is None:
                raise ValueError("Column OCR requires page dimensions.")
            return [BoundingBox(l=l, t=t, r=r, b=b,
                                coord_origin=CoordOrigin.TOPLEFT)
                    for l, t, r, b in column_bounds(
                        page.size.width, page.size.height, columns)]

    class ColumnPdfPipeline(StandardPdfPipeline):
        """Reuse the PDF pipeline with bounded OCR regions."""

        def _make_ocr_model(self, art_path: Path | None) -> ColumnOcrModel:
            """Construct local OCR with bounded regions.

            Model assets and accelerator settings come from the normal
            offline parser configuration.

            Args:
                art_path: Verified local Docling model directory.

            Returns:
                ColumnOcrModel: OCR stage retaining page provenance.
            """
            return ColumnOcrModel(
                enabled=self.pipeline_options.do_ocr,
                artifacts_path=art_path,
                options=self.pipeline_options.ocr_options,
                accelerator_options=self.pipeline_options.accelerator_options,
            )

    return ColumnPdfPipeline
