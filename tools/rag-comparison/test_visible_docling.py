"""Check conservative visibility filtering with synthetic pixel evidence.

Use small in-memory images to exercise faint text, invalid coordinates,
and blank regions without relying on the research corpus.

Classes:
    VisibilityTests: Verify only entirely white in-bounds cells are removed.
"""

from types import SimpleNamespace
import unittest

from PIL import Image
from visible_docling import is_blank_region


class VisibilityTests(unittest.TestCase):
    """Verify conservative filtering independently of document content."""

    def test_white_region_is_blank(self):
        """Recognize an entirely white cell rectangle.

        Use a normal in-bounds rectangle on a two-times rendering.

        Args:
            self: The current test case.

        Returns:
            None: Assert the white cell is excluded.
        """
        image = Image.new("RGB", (100, 100), "white")
        box = SimpleNamespace(l=10, t=10, r=20, b=20)
        self.assertTrue(is_blank_region(image, box, 50, 50))

    def test_faint_text_and_colored_background_are_retained(self):
        """Retain any non-white evidence in the cell rectangle.

        A single near-white pixel is sufficient; no contrast heuristic
        may erase faint notation or text over a colored background.

        Args:
            self: The current test case.

        Returns:
            None: Assert every non-white pixel prevents exclusion.
        """
        box = SimpleNamespace(l=10, t=10, r=20, b=20)
        for color in ((254, 255, 255), (0, 0, 0), (250, 250, 250)):
            image = Image.new("RGB", (100, 100), "white")
            image.putpixel((21, 21), color)
            self.assertFalse(is_blank_region(image, box, 50, 50))

    def test_uncertain_rectangles_are_retained(self):
        """Retain invalid and partially out-of-bounds rectangles.

        Do not infer invisibility from a clipped or empty pixel crop.

        Args:
            self: The current test case.

        Returns:
            None: Assert uncertain rectangles are preserved.
        """
        image = Image.new("RGB", (100, 100), "white")
        for left, top, right, bottom in (
            (-1, 10, 20, 20), (10, 10, 51, 20),
            (10, 10, 10, 20), (10, 10, 20, float("nan")),
        ):
            box = SimpleNamespace(l=left, t=top, r=right, b=bottom)
            self.assertFalse(is_blank_region(image, box, 50, 50))


if __name__ == "__main__":
    unittest.main()
