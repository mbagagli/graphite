"""Convert one local PDF into page-aware Graphite blocks offline.

Run only in the existing isolated environment. Validate pinned assets and
block networking before importing Docling. Never read evaluation answers.

Functions:
    digest: Hash exact file bytes.
    main: Convert a bounded PDF and emit a structured local result.
"""

import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import signal
import sys

from offline_exec import deny_network


def digest(path: Path) -> str:
    """Hash a local file without buffering it in memory.

    Use the same exact-byte identity for input and model validation.

    Args:
        path: File to read.

    Returns:
        str: Hexadecimal SHA-256 digest.
    """
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for data in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(data)
    return value.hexdigest()


def main() -> None:
    """Validate local resources and emit native text with page ranges.

    Automatic OCR and native Markdown tables share the existing pinned
    CPU configuration. Figure interpretation is not enabled.

    Args:
        none (None): Read command-line inputs.

    Returns:
        None: Write a JSON result to standard output.
    """
    parser = argparse.ArgumentParser(
        description=(
            "Graphite local Docling worker.\n"
            "Inputs: validated PDF, expected hash, existing runtime.\n"
            "Output: JSON text blocks and physical page ranges.\n"
            "Flow:\n1. Deny networking and verify assets.\n"
            "2. Convert locally on CPU with automatic OCR.\n"
            "3. Emit text and provenance; do not answer questions.\n"
            "Limits: 200 pages, 512 MiB, 15 minutes. No downloads."
        ), formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--lock", type=Path, required=True)
    args = parser.parse_args()
    # Exit with the app, including when it closes during a conversion.
    parent_pid = os.getppid()
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(1, signal.SIGTERM, 0, 0, 0) != 0:
        raise RuntimeError("Could not bind worker lifetime to Graphite.")
    if os.getppid() != parent_pid or parent_pid == 1:
        raise RuntimeError("Graphite exited before the worker started.")
    deny_network()
    runtime = args.runtime.resolve()
    if Path(sys.prefix).resolve() != (
        runtime / "envs/graphite-rag-comparison"
    ).resolve():
        raise ValueError("The dedicated Docling environment is required.")
    if digest(args.input) != args.sha256:
        raise ValueError("The PDF changed before conversion.")
    for key, folder in {
        "HF_HOME": "huggingface", "XDG_CACHE_HOME": "cache",
        "TORCH_HOME": "torch", "TMPDIR": "tmp",
    }.items():
        os.environ[key] = str(runtime / folder)
    for key in ("HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE",
                "HF_HUB_DISABLE_TELEMETRY", "DO_NOT_TRACK"):
        os.environ[key] = "1"
    os.environ["OMP_NUM_THREADS"] = "2"
    os.environ["OPENBLAS_NUM_THREADS"] = "2"
    from tqdm import tqdm
    for asset in tqdm(json.loads(args.lock.read_text())["files"],
                      desc="Verify local models", ncols=50):
        path = runtime / "models" / asset["path"]
        if digest(path) != asset["sha256"]:
            raise ValueError("A local model differs from its pinned hash.")
    from docling.datamodel.base_models import InputFormat
    from docling.datamodel.pipeline_options import (
        AcceleratorDevice, AcceleratorOptions, PdfPipelineOptions,
        RapidOcrOptions,
    )
    from docling.document_converter import DocumentConverter, PdfFormatOption
    from docling_core.types.doc import TableItem, TextItem
    from visible_docling import VisibleDoclingBackend
    from importlib.metadata import version
    if version("docling") != "2.142.1":
        raise ValueError("This adapter requires Docling 2.142.1.")
    options = PdfPipelineOptions(
        artifacts_path=runtime / "models", enable_remote_services=False,
        allow_external_plugins=False, do_ocr=True, do_table_structure=True,
        do_formula_enrichment=False, do_picture_description=False,
        document_timeout=900,
        accelerator_options=AcceleratorOptions(
            device=AcceleratorDevice.CPU, num_threads=2,
        ),
        ocr_options=RapidOcrOptions(backend="onnxruntime", lang=["iso:en"]),
    )
    converter = DocumentConverter(format_options={
        InputFormat.PDF: PdfFormatOption(
            pipeline_options=options, backend=VisibleDoclingBackend,
        ),
    })
    result = converter.convert(
        args.input, max_num_pages=200, max_file_size=512 * 1024 * 1024,
    )
    if result.status.value != "success":
        raise ValueError("Conversion did not complete; no text was activated.")
    blocks = []
    for item, _ in result.document.iterate_items(traverse_pictures=True):
        if isinstance(item, TableItem):
            text = item.export_to_markdown(doc=result.document)
            label = "table-markdown"
        elif isinstance(item, TextItem):
            text = item.text
            label = item.label.value
        else:
            continue
        if not text.strip():
            continue
        blocks.append({"id": item.self_ref, "text": text, "label": label,
                       "pages": sorted({p.page_no for p in item.prov})})
    if digest(args.input) != args.sha256:
        raise ValueError("The PDF changed during conversion.")
    print(json.dumps({"schemaVersion": 1, "inputSha256": args.sha256,
                      "pageCount": len(result.document.pages),
                      "blocks": blocks}, ensure_ascii=True))


if __name__ == "__main__":
    main()
