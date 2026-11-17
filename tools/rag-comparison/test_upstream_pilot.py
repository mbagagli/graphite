"""Check upstream bridge framing without loading models.

Exercise the test-runner prefix and failed-process boundary using in-memory
streams. These checks do not measure scientific answer quality.

Classes:
    BridgeTests: Verify pipe protocol framing and premature termination.
"""

import asyncio
import hashlib
import io
import json
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from upstream_pilot import GraphiteBridge, prepared_blocks


class BridgeTests(unittest.TestCase):
    """Verify bridge responses interleaved with Rust test headings."""

    def test_heading_before_handshake(self) -> None:
        """Accept the test heading on the same line as the handshake.

        Rust's harness does not end the test name with a newline.

        Args:
            self: The current test case.

        Returns:
            None: Assert the decoded handshake.
        """
        bridge = object.__new__(GraphiteBridge)
        bridge.process = SimpleNamespace(stdout=io.StringIO(
            'running 1 test\ntest bridge ... GRAPHITE_BRIDGE {"ready": true}\n'
        ))
        self.assertEqual(bridge.receive(), {"ready": True})

    def test_source_pages_and_tables(self) -> None:
        """Keep exact source text and refuse to guess multi-page origins.

        A synthetic prepared document exercises page conversion and native
        table adaptation without consuming reference answers.

        Args:
            self: The current test case.

        Returns:
            None: Assert source fidelity and hash validation.
        """
        with TemporaryDirectory() as folder:
            repo = Path(folder)
            prepared = repo / ".cache/rag-comparison/prepared-docling-v1"
            prepared.mkdir(parents=True)
            source = {
                "documentSha256": "synthetic", "content": "abcTABLExyz",
                "spans": [
                    {"start": 0, "end": 3, "blockRef": "x:#/texts/0",
                     "pageStart": 2, "pageEnd": 2},
                    {"start": 3, "end": 8, "blockRef": "x:#/tables/0",
                     "pageStart": 3, "pageEnd": 4},
                ],
            }
            raw = json.dumps(source).encode()
            (prepared / "source.json").write_bytes(raw)
            manifest = json.dumps({"documents": [{
                "id": "paper", "zoteroBaseline": "source.json",
                "zoteroBaselineSha256": hashlib.sha256(raw).hexdigest(),
            }]}).encode()
            (prepared / "manifest.json").write_bytes(manifest)
            corpus = repo / "evaluation/upstream-rag-v1/corpus.json"
            corpus.parent.mkdir(parents=True)
            corpus.write_text(json.dumps({"documents": [
                {"id": "paper", "pdfSha256": "synthetic"},
            ]}))
            with patch("upstream_pilot.MANIFEST_SHA256",
                       hashlib.sha256(manifest).hexdigest()):
                blocks, _ = prepared_blocks(repo, "paper")
                self.assertEqual(blocks[0]["text"], "abc")
                self.assertEqual(blocks[0]["page_idx"], 1)
                self.assertEqual(blocks[1]["table_body"], "TABLE")
                self.assertNotIn("page_idx", blocks[1])
                (prepared / "source.json").write_text("changed")
                with self.assertRaises(AssertionError):
                    prepared_blocks(repo, "paper")

    def test_incomplete_generation_rejected(self) -> None:
        """Reject capped or timed-out extraction before graph insertion.

        Retain this boundary even if upstream accepts partial records.

        Args:
            self: The current test case.

        Returns:
            None: Assert both truncation and cancellation are failures.
        """
        bridge = object.__new__(GraphiteBridge)
        for tokens, timed_out in [(1024, False), (10, True)]:
            with patch.object(bridge, "request", return_value={
                "text": "partial", "generatedTokens": tokens,
                "timedOut": timed_out,
            }):
                with self.assertRaisesRegex(RuntimeError, "incomplete"):
                    asyncio.run(bridge.model("Extract entities"))

    def test_premature_exit(self) -> None:
        """Reject an ended process instead of returning an empty answer.

        Missing protocol output must remain a runtime failure.

        Args:
            self: The current test case.

        Returns:
            None: Assert the explicit failure.
        """
        bridge = object.__new__(GraphiteBridge)
        bridge.process = SimpleNamespace(stdout=io.StringIO('test failed\n'))
        with self.assertRaisesRegex(RuntimeError, 'before responding'):
            bridge.receive()


if __name__ == '__main__':
    unittest.main()
