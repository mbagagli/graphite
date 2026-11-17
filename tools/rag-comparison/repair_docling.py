"""Create an immutable comparison extraction with one repaired page.

Read the original prepared sidecar and a successful Docling page run.
Replace only unambiguous single-page blocks, retaining all other text.
No questions or expected answers are inputs to this adapter.

Functions:
    replace_page: Replace contiguous blocks and recompute exact offsets.
    main: Validate parser identity and save a fresh extraction revision.
"""

import argparse
import json
from pathlib import Path
import sys

from offline_exec import deny_network
from parse_docling import RUNTIME, file_digest
from prepare_docling import project_blocks


def replace_page(
    payload: dict, blocks: list[dict], page: int,
    retain_cross_page: bool = False,
) -> dict:
    """Replace one page without guessing at ambiguous source boundaries.

    Preserve the original payload and regenerate offsets for the entire
    document because later text moves when the page length changes.

    Args:
        payload: Original text and exact block spans.
        blocks: Repaired blocks in native reading order.
        page: Positive physical page number to replace.
        retain_cross_page: Keep original crossing blocks after manual audit.

    Returns:
        dict: New payload with incremented version and recomputed spans.
    """
    if type(page) is not int or not 1 <= page <= payload["totalPages"]:
        raise ValueError("Repair page must be within the source document.")
    if not blocks or any(block["pages"] != [page] for block in blocks):
        raise ValueError(
            "Repair blocks must have exact single-page provenance."
        )
    original = []
    replaced = []
    for index, span in enumerate(payload["spans"]):
        low, high = span["pageStart"], span["pageEnd"]
        if (low is None or high is None
                or low <= page <= high and low != high
                and not retain_cross_page):
            raise ValueError("Cannot repair ambiguous or cross-page blocks.")
        original.append({
            "text": payload["content"][span["start"]:span["end"]],
            "pages": list(range(low, high + 1)), "ref": span["blockRef"],
        })
        if low == high == page:
            replaced.append(index)
    if not replaced or replaced != list(range(replaced[0], replaced[-1] + 1)):
        raise ValueError("Repair requires contiguous original page blocks.")
    text, spans = project_blocks(original)
    if text != payload["content"] or spans != payload["spans"]:
        raise ValueError("Original spans do not reproduce exact source text.")
    text, spans = project_blocks(
        original[:replaced[0]] + blocks + original[replaced[-1] + 1:]
    )
    return {
        **payload, "sourceVersion": payload["sourceVersion"] + 1,
        "content": text, "spans": spans,
        "indexedCharacters": len(text), "totalCharacters": len(text),
    }


def main() -> None:
    """Validate a local repair and create fresh sidecars and provenance.

    Copy unaffected sidecars byte-for-byte and require matching PDF hashes.
    Refuse existing output directories so failed attempts remain visible.

    Args:
        none (None): Read command-line arguments.

    Returns:
        None: Write a new prepared input directory.
    """
    parser = argparse.ArgumentParser(
        description=(
            "Create a page-repaired Graphite comparison extraction.\n"
            "Inputs: original prepared directory, document ID, parser run.\n"
            "Outputs: fresh sidecars, manifest and repair provenance.\n"
            "Flow:\n1. Deny networking and verify source hashes.\n"
            "2. Replace one unambiguous page and recompute offsets.\n"
            "3. Copy other documents unchanged into a new directory.\n"
            "Requires the existing isolated Docling environment."
        ), formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--document", required=True)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument(
        "--retain-cross-page", action="store_true",
        help="Keep original boundary blocks after auditing their overlap.",
    )
    parser.add_argument(
        "--exclude-block", action="append", default=[],
        help="Native repaired block ref already covered by a kept boundary.",
    )
    args = parser.parse_args()
    deny_network()
    if Path(sys.prefix).resolve() != (
        RUNTIME / "envs" / "graphite-rag-comparison"
    ).resolve():
        raise RuntimeError("Use the dedicated comparison Python environment.")
    from docling_core.types.doc import DoclingDocument, TableItem, TextItem
    from tqdm import tqdm

    manifest_path = args.input / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    summary = json.loads((args.run / "summary.json").read_text())
    first, last = summary["requestedPdfPages"]
    if (summary["status"] != "success" or first != last
            or summary["parsedPageCount"] != 1
            or summary["documentId"] != args.document):
        raise ValueError("Repair requires a successful matching single page.")
    if sum(d["id"] == args.document for d in manifest["documents"]) != 1:
        raise ValueError("Repair document must occur exactly once.")
    document_path = args.run / "document.json"
    document = DoclingDocument.model_validate_json(document_path.read_text())
    digest = file_digest(document_path)
    blocks = []
    excluded = set()
    for item, _ in document.iterate_items(traverse_pictures=True):
        if isinstance(item, TableItem):
            text = item.export_to_markdown(doc=document)
        elif isinstance(item, TextItem):
            text = item.text
        else:
            continue
        if item.self_ref in args.exclude_block:
            excluded.add(item.self_ref)
            continue
        blocks.append({
            "text": text, "pages": sorted({p.page_no for p in item.prov}),
            "ref": f"{digest}:{item.self_ref}",
        })
    if excluded != set(args.exclude_block):
        raise ValueError("Excluded block is absent from the parser output.")
    if excluded and not args.retain_cross_page:
        raise ValueError("Exclusions require audited boundary preservation.")
    args.output.mkdir(exist_ok=False)
    for entry in tqdm(manifest["documents"], desc="Revise page", ncols=50):
        source = args.input / entry["zoteroBaseline"]
        if source.resolve().parent != args.input.resolve():
            raise ValueError("Sidecar must be directly inside input directory.")
        if file_digest(source) != entry["zoteroBaselineSha256"]:
            raise ValueError("Original sidecar checksum mismatch.")
        data = source.read_bytes()
        if entry["id"] == args.document:
            payload = json.loads(data)
            if payload["documentSha256"] != summary["documentSha256"]:
                raise ValueError("Repair PDF identity differs from original.")
            revised = replace_page(
                payload, blocks, first, args.retain_cross_page,
            )
            revised["repair"] = {
                "parentSidecarSha256": file_digest(source),
                "parentManifestSha256": file_digest(manifest_path),
                "page": first, "runId": args.run.name,
                "retainedCrossPageBlocks": args.retain_cross_page,
                "excludedRepairBlocks": sorted(excluded),
                "outputSha256": digest,
                "summarySha256": file_digest(args.run / "summary.json"),
            }
            data = (json.dumps(revised, ensure_ascii=True) + "\n").encode()
        target = args.output / source.name
        target.write_bytes(data)
        entry["zoteroBaselineSha256"] = file_digest(target)
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
