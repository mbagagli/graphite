"""Project locked Docling output into text with physical-page provenance.

Reuse Docling's reading order and native table serializer. This offline
comparison adapter does not read questions, answer keys or anchor files.
It creates new experiment inputs and never modifies production storage.

Functions:
    project_blocks: Preserve Unicode character offsets and page ranges.
    main: Verify successful parser runs and write isolated adapter inputs.
"""

import argparse
import json
import logging
from pathlib import Path
import sys

from offline_exec import deny_network
from parse_docling import ROOT, RUNTIME, file_digest


def project_blocks(blocks: list[dict]) -> tuple[str, list[dict]]:
    """Join source blocks without changing their text or page identities.

    Offsets count Unicode code points, matching Rust's chars().count().
    Multiple source pages remain a range; missing provenance stays null.

    Args:
        blocks: Ordered text blocks with original page lists and identities.

    Returns:
        tuple[str, list[dict]]: Joined text and half-open character spans.
    """
    parts = []
    spans = []
    offset = 0
    for block in blocks:
        text = block["text"]
        if not text.strip():
            continue
        if parts:
            parts.append("\n\n")
            offset += 2
        pages = block["pages"]
        if any(not isinstance(page, int) or page < 1 for page in pages):
            raise ValueError("Physical pages must be positive integers.")
        spans.append({
            "start": offset, "end": offset + len(text),
            "pageStart": min(pages) if pages else None,
            "pageEnd": max(pages) if pages else None,
            "blockRef": block["ref"],
        })
        parts.append(text)
        offset += len(text)
    return "".join(parts), spans


def main() -> None:
    """Verify successful parser runs and write isolated adapter inputs.

    Reject partial runs, changed bytes, duplicate pages and missing pages.
    The output sidecars use the existing fixture ingestion contract while
    retaining separate parser provenance for chunk-page mapping.

    Args:
        none (None): Parse command-line options only.

    Returns:
        None: Write a new local input directory and manifest.
    """
    parser = argparse.ArgumentParser(
        description=(
            "Prepare Graphite's offline page-aware Docling comparison.\n"
            "Inputs: locked successful parser JSON, corpus identities.\n"
            "Outputs: text sidecars, page spans and input hashes.\n"
            "Flow:\n1. Block networking and validate parser identities.\n"
            "2. Reuse Docling reading order and table rendering.\n"
            "3. Save fresh local inputs without reading the answer key.\n"
            "Requires the dedicated comparison Python environment."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.parse_args()
    deny_network()
    if Path(sys.prefix).resolve() != (
        RUNTIME / "envs" / "graphite-rag-comparison"
    ).resolve():
        raise RuntimeError("Use the dedicated comparison Python environment.")
    from docling_core.types.doc import DoclingDocument, TableItem, TextItem
    from tqdm import tqdm

    logging.basicConfig(level=logging.INFO, format="%(message)s")
    bundle = ROOT / "evaluation" / "upstream-rag-v1"
    inputs = json.loads((bundle / "docling-inputs.json").read_text())
    corpus = json.loads((bundle / "corpus.json").read_text())
    docs = {doc["id"]: doc for doc in corpus["documents"]}
    output = RUNTIME / "prepared-docling-v1"
    output.mkdir(exist_ok=False)
    manifest = {"documents": [], "profile": inputs["profile"]}
    for entry in tqdm(inputs["documents"], desc="Project pages", ncols=50):
        identity = docs[entry["id"]]
        blocks = []
        covered = set()
        for run in entry["runs"]:
            folder = RUNTIME / "runs" / run["runId"]
            if folder.resolve().parent != (RUNTIME / "runs").resolve():
                raise ValueError("Parser run must be inside the run directory.")
            for name, key in [("summary.json", "summarySha256"),
                              ("document.json", "outputSha256")]:
                if file_digest(folder / name) != run[key]:
                    raise ValueError(f"Changed parser input: {run['runId']}")
            summary = json.loads((folder / "summary.json").read_text())
            if (summary["status"] != "success"
                    or summary["documentSha256"] != identity["pdfSha256"]):
                raise ValueError("Only complete, matching parser runs qualify.")
            first, last = summary["requestedPdfPages"]
            pages = set(range(first, last + 1))
            if pages & covered:
                raise ValueError("Duplicate parser page coverage.")
            covered.update(pages)
            document = DoclingDocument.model_validate_json(
                (folder / "document.json").read_text()
            )
            for item, _ in document.iterate_items(traverse_pictures=True):
                if isinstance(item, TableItem):
                    text = item.export_to_markdown(doc=document)
                elif isinstance(item, TextItem):
                    text = item.text
                else:
                    continue
                item_pages = [value.page_no for value in item.prov]
                if not set(item_pages) <= pages:
                    raise ValueError("Block provenance exceeds its parser run.")
                blocks.append({
                    "text": text, "pages": item_pages,
                    "ref": f"{run['outputSha256']}:{item.self_ref}",
                })
        if covered != set(range(1, identity["expectedPages"] + 1)):
            raise ValueError("Parser runs do not cover the complete document.")
        text, spans = project_blocks(blocks)
        payload = {
            "sourceVersion": 1, "content": text, "spans": spans,
            "indexedPages": len(covered), "totalPages": len(covered),
            "indexedCharacters": len(text), "totalCharacters": len(text),
            "documentSha256": identity["pdfSha256"],
            "profile": inputs["profile"], "parserRuns": entry["runs"],
        }
        target = output / f"{entry['id']}.json"
        target.write_text(json.dumps(payload, ensure_ascii=True) + "\n")
        manifest["documents"].append({
            "id": entry["id"], "zoteroBaseline": target.name,
            "zoteroBaselineSha256": file_digest(target),
        })
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    logging.info("Prepared %d page-aware documents.", len(docs))


if __name__ == "__main__":
    main()
