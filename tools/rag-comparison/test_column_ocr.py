"""Check explicit column geometry and offline parser CLI boundaries.

Use synthetic page dimensions without loading model packages.
The opt-in parser smoke run verifies the pinned upstream adapter itself.

Classes:
    ColumnOcrTests: Exercise bounds and reject unsafe mode combinations.
"""

from pathlib import Path
import subprocess
import sys
import unittest

from column_ocr import column_bounds


class ColumnOcrTests(unittest.TestCase):
    """Check page partitions and single-page opt-in behavior."""

    def test_columns_cover_page_without_overlap(self) -> None:
        """Retain the original dimensions and left-to-right ordering.

        Include a fractional width so rounding cannot create page gaps.

        Args:
            self: The current test case.

        Returns:
            None: Assert exact region continuity and full coverage.
        """
        for count in (2, 3):
            bounds = column_bounds(601.5, 792.0, count)
            self.assertEqual(bounds[0][0], 0)
            self.assertEqual(bounds[-1][2], 601.5)
            for previous, following in zip(bounds, bounds[1:]):
                self.assertEqual(previous[2], following[0])
            self.assertTrue(all(b[1] == 0 and b[3] == 792 for b in bounds))
        for width, height, count in [
            (0, 792, 2), (600, -1, 2), (float('nan'), 792, 2),
            (600, float('inf'), 2), (600, 792, 4), (600, 792, 2.0),
        ]:
            with self.assertRaises(ValueError):
                column_bounds(width, height, count)

    def test_cli_requires_explicit_single_page_and_exclusive_modes(
        self,
    ) -> None:
        """Reject ambiguous modes before loading optional models.

        Run help and invalid requests with the test interpreter, proving
        these boundaries do not need Docling or network access.

        Args:
            self: The current test case.

        Returns:
            None: Assert help and early argument-validation behavior.
        """
        script = Path(__file__).with_name('parse_docling.py')
        command = [sys.executable, str(script)]
        help_result = subprocess.run(command + ['--help'], capture_output=True)
        self.assertEqual(help_result.returncode, 0)
        self.assertIn(b'--ocr-columns', help_result.stdout)
        base = command + ['--document', 'synthetic', '--run', 'cli-test']
        for options in [
            ['--ocr-columns', '2'],
            ['--ocr-columns', '2', '--first-page', '1', '--last-page', '2'],
            ['--ocr-columns', '2', '--force-full-page-ocr'],
            ['--ocr-detection-side', '255'],
            ['--ocr-detection-side', '2049'],
        ]:
            result = subprocess.run(base + options, capture_output=True)
            self.assertEqual(result.returncode, 2)
            self.assertNotIn(b'Traceback', result.stderr)
