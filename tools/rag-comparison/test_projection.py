"""Check page-preserving text projection without loading model packages.

These synthetic cases cover Unicode offsets, unknown provenance and
multi-page blocks independently from scientific answer correctness.

Classes:
    ProjectionTests: Exercise the pure text and page projection contract.
"""

import unittest

from prepare_docling import project_blocks


class ProjectionTests(unittest.TestCase):
    """Verify source offsets and page bounds on small synthetic blocks."""

    def test_unicode_ranges_and_unknown_pages(self) -> None:
        """Keep page ranges and code-point offsets without guessing.

        Cover a multi-byte character, a page-spanning block and absent
        provenance, with no model installation required.

        Args:
            self: The current test case.

        Returns:
            None: Assert exact text, offsets and unknown page markers.
        """
        text, spans = project_blocks([
            {"text": "\u03bd=2", "pages": [2], "ref": "a"},
            {"text": "cross-page", "pages": [3, 4], "ref": "b"},
            {"text": "unknown", "pages": [], "ref": "c"},
        ])
        self.assertEqual(spans[1]["start"], 5)
        self.assertEqual(text[5:15], "cross-page")
        self.assertEqual(spans[1]["pageEnd"], 4)
        self.assertIsNone(spans[2]["pageStart"])
        with self.assertRaises(ValueError):
            project_blocks([{"text": "bad", "pages": [0], "ref": "d"}])
