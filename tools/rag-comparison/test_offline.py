"""Test the offline launcher without importing document models.

These checks use subprocesses so network restrictions cannot affect the
test runner. They validate isolation and CLI behavior, not parser quality.

Classes:
    OfflineTests: Check network restrictions and parser preparation helpers.
"""

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from parse_docling import file_digest


SCRIPTS = Path(__file__).resolve().parent


class OfflineTests(unittest.TestCase):
    """Check network restrictions and parser preparation helpers.

    Run on Linux with libseccomp. No document corpus or model downloads
    are required, and all subprocesses inherit only ordinary pipe handles.
    """

    def test_network_filter_survives_child_execution(self) -> None:
        """Verify the inherited filter with an executable self-check.

        Use the launcher's parent and child probes as the isolation test.

        Args:
            self: The current test instance.

        Returns:
            None: Assert successful filter installation and inheritance.
        """
        result = subprocess.run(
            [sys.executable, str(SCRIPTS / "offline_exec.py"), "--self-test"],
            capture_output=True, text=True, check=False, timeout=20,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PASS", result.stderr)

    def test_hash_and_cli_help_require_no_models(self) -> None:
        """Verify deterministic hashes and model-free CLI help.

        Confirm preparation commands remain usable before installation.

        Args:
            self: The current test instance.

        Returns:
            None: Assert the known digest and documented processing flow.
        """
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.txt"
            path.write_bytes(b"abc")
            self.assertEqual(
                file_digest(path),
                "ba7816bf8f01cfea414140de5dae2223"
                "b00361a396177a9cb410ff61f20015ad",
            )
        for name in ("offline_exec.py", "parse_docling.py"):
            result = subprocess.run(
                [sys.executable, str(SCRIPTS / name), "--help"],
                capture_output=True, text=True, check=False, timeout=20,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Flow:", result.stdout)

    def test_run_name_cannot_escape_output_root(self) -> None:
        """Reject traversal before optional model imports occur.

        Run the parser entry point with an invalid output identifier.

        Args:
            self: The current test instance.

        Returns:
            None: Assert that argument validation rejects the identifier.
        """
        result = subprocess.run(
            [sys.executable, str(SCRIPTS / "parse_docling.py"),
             "--document", "paper20", "--run", "../escape"],
            capture_output=True, text=True, check=False, timeout=20,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("Run name", result.stderr)

    def test_wrong_python_environment_is_rejected(self) -> None:
        """Reject parser execution from an unrelated Python environment.

        Avoid importing optional packages from an existing environment.

        Args:
            self: The current test instance.

        Returns:
            None: Assert rejection outside the dedicated runtime prefix.
        """
        expected = SCRIPTS.parents[1] / ".cache" / "rag-comparison"
        expected = expected / "envs" / "graphite-rag-comparison"
        if Path(sys.prefix).resolve() == expected.resolve():
            self.skipTest("Test requires a different Python environment.")
        result = subprocess.run(
            [sys.executable, str(SCRIPTS / "parse_docling.py"),
             "--document", "paper20", "--run", "wrong-environment"],
            capture_output=True, text=True, check=False, timeout=20,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("dedicated graphite-rag-comparison Python", result.stderr)


if __name__ == "__main__":
    unittest.main()
