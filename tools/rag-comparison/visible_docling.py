"""Exclude embedded text positioned on completely white rendered areas.

Use the installed PDFium renderer as an independent check on Docling's
text coordinates. This conservative check does not detect all clipping,
white text over pictures, or text painted over by non-white content.

Functions:
    is_blank_region: Check an in-bounds text rectangle for white pixels.

Classes:
    VisibleDoclingBackend: Filter blank-position text before layout.
"""

import logging
import math

from docling.backend.docling_parse_backend import (
    ThreadedDoclingParseDocumentBackend,
)
import pypdfium2 as pdfium
from tqdm import tqdm


LOGGER = logging.getLogger(__name__)
RENDER_SCALE = 2


def is_blank_region(image, box, width: float, height: float) -> bool:
    """Check whether a text rectangle contains only white pixels.

    Expand to pixel boundaries and retain uncertain or partial rectangles.
    Require exact white in every channel, avoiding contrast thresholds
    that could discard faint but visible scientific notation.

    Args:
        image: RGB rendering of the physical page.
        box: Text bounding box in top-left page coordinates.
        width: Physical page width in points.
        height: Physical page height in points.

    Returns:
        bool: True only for a valid, entirely white rectangle.
    """
    left, top, right, bottom = box.l, box.t, box.r, box.b
    if not (0 <= left < right <= width and
            0 <= top < bottom <= height):
        return False
    sx, sy = image.width / width, image.height / height
    crop = image.crop((math.floor(left * sx), math.floor(top * sy),
                       math.ceil(right * sx), math.ceil(bottom * sy)))
    return all(extrema == (255, 255) for extrema in crop.getextrema())


class VisibleDoclingBackend(ThreadedDoclingParseDocumentBackend):
    """Filter blank-position text using the existing PDFium renderer.

    Preserve native cells on non-white or uncertain regions. Filter all
    cell views together so layout, OCR merging, and table text use the
    same evidence. Original PDF bytes and physical pages are unchanged.
    """

    def iter_pages(self):
        """Yield page backends after excluding provably blank text areas.

        Render one physical page at a time to bound memory usage. The
        installed native backend supplies the remaining parsing behavior.

        Args:
            self: The current document backend.

        Returns:
            Iterator: Filtered page backends in their original order.
        """
        with pdfium.PdfDocument(self.path_or_stream) as document:
            for page in tqdm(super().iter_pages(), desc="Check text",
                             ncols=50):
                segmented = page.get_segmented_page()
                if segmented is not None:
                    rendered_page = document[page.page_no - 1]
                    try:
                        bitmap = rendered_page.render(scale=RENDER_SCALE)
                        try:
                            image = bitmap.to_pil().convert("RGB")
                        finally:
                            bitmap.close()
                    finally:
                        rendered_page.close()
                    size = page.get_size()
                    removed = 0
                    for name in ("textline_cells", "word_cells",
                                 "char_cells"):
                        cells = getattr(segmented, name)
                        retained = [cell for cell in cells if not
                                    is_blank_region(
                                        image, cell.rect.to_bounding_box(),
                                        size.width, size.height)]
                        removed += len(cells) - len(retained)
                        setattr(segmented, name, retained)
                    image.close()
                    if removed:
                        LOGGER.warning(
                            "Page %s: excluded %s blank-position cells",
                            page.page_no, removed,
                        )
                yield page
