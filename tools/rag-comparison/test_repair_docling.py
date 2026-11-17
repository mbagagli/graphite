"""Test immutable page replacement using synthetic source blocks.

Exercise Unicode offsets, preserved neighboring text, and rejection of
ambiguous page boundaries without importing model packages.

Classes:
    RepairTests: Verify exact revisions and unsafe-input rejection.
"""

import copy
import unittest

from prepare_docling import project_blocks
from repair_docling import replace_page


class RepairTests(unittest.TestCase):
    """Verify local page repair and immutable input behavior."""

    def test_replacement_and_rejections(self) -> None:
        """Keep unrelated blocks and reject unresolvable provenance.

        Use a Unicode replacement of different length to verify that
        following offsets move while their text and identities remain.

        Args:
            self: The current test case.

        Returns:
            None: Assert repaired content and boundary validation.
        """
        blocks = [
            {"text": "before", "pages": [1], "ref": "a"},
            {"text": "damaged", "pages": [2], "ref": "b"},
            {"text": "after", "pages": [3], "ref": "c"},
        ]
        text, spans = project_blocks(blocks)
        original = {
            "content": text, "spans": spans, "totalPages": 3,
            "sourceVersion": 1,
        }
        saved = copy.deepcopy(original)
        replacement = [{"text": "\u03bd corrected", "pages": [2], "ref": "d"}]
        revised = replace_page(original, replacement, 2)
        self.assertEqual(original, saved)
        self.assertEqual(revised["sourceVersion"], 2)
        self.assertEqual(
            revised["content"], "before\n\n\u03bd corrected\n\nafter",
        )
        tail = revised["spans"][-1]
        self.assertEqual(tail["blockRef"], "c")
        self.assertEqual(revised["content"][tail["start"]:tail["end"]], "after")
        for page in (0, 4, True):
            with self.assertRaises(ValueError):
                replace_page(original, replacement, page)
        for invalid in ([], [{**replacement[0], "pages": [1, 2]}]):
            with self.assertRaises(ValueError):
                replace_page(original, invalid, 2)
        for low, high in ((None, None), (1, 2)):
            ambiguous = copy.deepcopy(original)
            ambiguous["spans"][1].update(pageStart=low, pageEnd=high)
            with self.assertRaises(ValueError):
                replace_page(ambiguous, replacement, 2)
        corrupt = {**original, "content": text + "unmapped"}
        with self.assertRaises(ValueError):
            replace_page(corrupt, replacement, 2)
        boundary = copy.deepcopy(original)
        boundary["spans"][0]["pageEnd"] = 2
        retained = replace_page(boundary, replacement, 2, True)
        self.assertEqual(retained["spans"][0], boundary["spans"][0])
        self.assertTrue(retained["content"].startswith("before\n\n"))
        noncontiguous = copy.deepcopy(original)
        noncontiguous["spans"][0].update(pageStart=2, pageEnd=2)
        noncontiguous["spans"][1].update(pageStart=1, pageEnd=1)
        noncontiguous["spans"][2].update(pageStart=2, pageEnd=2)
        with self.assertRaises(ValueError):
            replace_page(noncontiguous, replacement, 2)
