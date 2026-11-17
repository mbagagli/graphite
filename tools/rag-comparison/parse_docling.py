"""Extract a bounded benchmark PDF with network-disabled Python Docling.

This preparation utility reads existing corpus manifests and model locks.
It exports local parser evidence only; it does not retrieve or answer
questions and never reads the evaluation answer key.

Functions:
    file_digest: Hash an input without loading it all into memory.
    main: Validate inputs and run the isolated parser smoke check.
"""

import argparse
import hashlib
import json
import logging
import os
from pathlib import Path
import resource
import sys
import time

from offline_exec import deny_network


ROOT = Path(__file__).resolve().parents[2]
RUNTIME = ROOT / ".cache" / "rag-comparison"


def file_digest(path: Path) -> str:
    """Hash an input without loading it all into memory.

    Compute the exact-byte SHA-256 identity used by corpus and model locks.

    Args:
        path: Existing file whose content must be checked.

    Returns:
        str: Lowercase hexadecimal SHA-256 digest.
    """
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    """Validate inputs and run the isolated parser smoke check.

    Restrict networking before importing Docling or opening document bytes.
    Keep all generated evidence in a new managed runtime subdirectory.

    Args:
        none (None): This function does not accept arguments.

    Returns:
        None: Write local JSON extraction and a summary.
    """
    parser = argparse.ArgumentParser(
        description=(
            "Graphite offline Docling smoke check. Read one existing "
            "benchmark PDF and export parser JSON plus operational counts.\n"
            "Flow:\n1. Deny networking and verify document/model hashes.\n"
            "2. Parse a bounded PDF page range locally on CPU.\n"
            "3. Save local output and timing without scoring answers.\n"
            "Defaults: full document, English automatic OCR, two CPU "
            "threads, 120-second document timeout. Models must be prepared.\n"
            "Example: python parse_docling.py --document paper20 "
            "--run smoke-paper20 --last-page 2"
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--document", required=True)
    parser.add_argument("--run", required=True)
    parser.add_argument("--first-page", type=int, default=1)
    parser.add_argument("--last-page", type=int)
    ocr_modes = parser.add_mutually_exclusive_group()
    ocr_modes.add_argument(
        "--force-full-page-ocr", action="store_true",
        help=("Replace embedded text with local OCR for every requested "
              "page. Use for damaged text layers; keep automatic OCR "
              "as the default. No new models are downloaded."),
    )
    ocr_modes.add_argument(
        "--ocr-layout-regions", action="store_true",
        help=("Run local OCR within detected layout regions. Use to "
              "avoid recognition across separate text columns."),
    )
    ocr_modes.add_argument(
        "--ocr-columns", type=int, choices=(2, 3),
        help=("OCR an inspected single page as equal-width columns. "
              "Requires identical --first-page and --last-page. Do not "
              "use on full-width tables or mixed-column layouts."),
    )
    parser.add_argument(
        "--ocr-detection-side", type=int,
        help=("Override RapidOCR's minimum detection side in pixels. "
              "Larger values can preserve narrow column gutters but cost "
              "more memory. Default: use the installed OCR default."),
    )
    args = parser.parse_args()
    if args.ocr_detection_side is not None and not (
        256 <= args.ocr_detection_side <= 2048
    ):
        parser.error("OCR detection side must be between 256 and 2048.")
    if not args.run or any(char not in
                           "abcdefghijklmnopqrstuvwxyz0123456789-"
                           for char in args.run):
        parser.error("Run name must contain lowercase letters/digits/hyphens.")
    if args.ocr_columns and args.first_page != args.last_page:
        parser.error("Column OCR requires one explicitly bounded PDF page.")
    deny_network()
    expected_prefix = RUNTIME / "envs" / "graphite-rag-comparison"
    if Path(sys.prefix).resolve() != expected_prefix.resolve():
        raise RuntimeError("Use the dedicated graphite-rag-comparison Python.")
    cache_paths = {
        "HF_HOME": RUNTIME / "huggingface",
        "XDG_CACHE_HOME": RUNTIME / "cache",
        "TORCH_HOME": RUNTIME / "torch",
        "TMPDIR": RUNTIME / "tmp",
    }
    for name, path in cache_paths.items():
        path.mkdir(parents=True, exist_ok=True)
        os.environ[name] = str(path)
    for name in ("HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE",
                 "HF_HUB_DISABLE_TELEMETRY", "DO_NOT_TRACK"):
        os.environ[name] = "1"
    os.environ["OMP_NUM_THREADS"] = "2"
    os.environ["OPENBLAS_NUM_THREADS"] = "2"
    logging.basicConfig(level=logging.INFO, format="%(message)s")

    # Optional runtime imports occur only after the network restriction.
    from tqdm import tqdm
    from docling.datamodel.base_models import InputFormat
    from docling.datamodel.pipeline_options import (
        AcceleratorDevice, AcceleratorOptions, PdfPipelineOptions,
        OcrMode, RapidOcrOptions,
    )
    from docling.document_converter import DocumentConverter, PdfFormatOption

    bundle_path = ROOT / "evaluation" / "upstream-rag-v1" / "corpus.json"
    corpus = json.loads(bundle_path.read_text())
    matches = [doc for doc in corpus["documents"]
               if doc["id"] == args.document]
    if len(matches) != 1:
        parser.error("Document must be one of the six comparison papers.")
    doc = matches[0]
    last_page = args.last_page or doc["expectedPages"]
    if not 1 <= args.first_page <= last_page <= doc["expectedPages"]:
        parser.error("Page range must lie within the selected PDF.")
    manifest_path = ROOT / doc["manifest"]
    if file_digest(manifest_path) != doc["manifestSha256"]:
        raise ValueError("Source manifest differs from the comparison lock.")
    manifest = json.loads(manifest_path.read_text())
    source = next(row for row in manifest["documents"]
                  if row["id"] == args.document)
    pdf_path = manifest_path.parent / source["file"]
    if file_digest(pdf_path) != doc["pdfSha256"]:
        raise ValueError("PDF differs from the comparison lock.")
    model_lock = Path(__file__).parent / "models-docling.lock.json"
    models = json.loads(model_lock.read_text())
    model_root = RUNTIME / "models"
    for item in tqdm(models["files"], desc="Verify assets", ncols=50):
        if file_digest(model_root / item["path"]) != item["sha256"]:
            raise ValueError("Model asset differs from its pinned checksum.")
    output = RUNTIME / "runs" / args.run
    output.mkdir(parents=True, exist_ok=False)
    options = PdfPipelineOptions(
        artifacts_path=model_root,
        enable_remote_services=False,
        allow_external_plugins=False,
        do_ocr=True,
        do_table_structure=True,
        do_formula_enrichment=False,
        do_picture_description=False,
        document_timeout=120,
        accelerator_options=AcceleratorOptions(
            device=AcceleratorDevice.CPU, num_threads=2
        ),
        ocr_options=RapidOcrOptions(
            backend="onnxruntime", lang=["iso:en"],
            mode=(OcrMode.FULL_PAGE
                  if args.force_full_page_ocr or args.ocr_columns
                  else OcrMode.LAYOUT_REGIONS if args.ocr_layout_regions
                  else OcrMode.DEFAULT),
            rapidocr_params=(
                {"Det.limit_side_len": args.ocr_detection_side}
                if args.ocr_detection_side is not None else {}
            ),
        ),
    )
    started = time.perf_counter()
    format_options = {"pipeline_options": options}
    if args.ocr_columns:
        from column_ocr import column_pipeline

        format_options["pipeline_cls"] = column_pipeline(args.ocr_columns)
    converter = DocumentConverter(format_options={
        InputFormat.PDF: PdfFormatOption(**format_options)
    })
    result = converter.convert(
        pdf_path, page_range=(args.first_page, last_page)
    )
    elapsed = time.perf_counter() - started
    payload = result.document.export_to_dict()
    (output / "document.json").write_text(json.dumps(payload) + "\n")
    (output / "document.md").write_text(result.document.export_to_markdown())
    summary = {
        "documentId": args.document,
        "documentSha256": doc["pdfSha256"],
        "profile": models["profile"],
        "modelsLockSha256": file_digest(model_lock),
        "status": result.status.value,
        "requestedPdfPages": [args.first_page, last_page],
        "parsedPageCount": len(result.document.pages),
        "textItemCount": len(result.document.texts),
        "tableCount": len(result.document.tables),
        "pictureCount": len(result.document.pictures),
        "conversionSeconds": elapsed,
        "peakProcessRssKiB": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
        "networkPolicy": "linux-seccomp-network-denied",
        "remoteServices": False,
        "forceFullPageOcr": options.ocr_options.mode == OcrMode.FULL_PAGE,
        "ocrDetectionSide": args.ocr_detection_side,
        "ocrLayoutRegions": args.ocr_layout_regions,
        "ocrColumns": args.ocr_columns,
        "resolvedOcrMode": options.ocr_options.mode.value,
        "scored": False,
    }
    (output / "summary.json").write_text(
        json.dumps(summary, indent=2) + "\n"
    )
    logging.info("Local extraction summary: %s", json.dumps(summary))
    if result.status.value != "success":
        raise RuntimeError("Parser did not report a complete success.")


if __name__ == "__main__":
    main()
