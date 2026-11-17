"""Test isolated upstream workspaces with synthetic local evidence.

Exercise the installed RAG-Anything and LightRAG code using deterministic
fake models. No research document or scientific answer is evaluated. The
event loop's private notification pair is created before the inherited
network filter; upstream imports and processing happen after restriction.

Classes:
    CharacterTokenizer: Supply reversible tokens without downloading assets.

Functions:
    fake_embedding: Supply deterministic synthetic vectors.
    fake_model: Return synthetic graph records and observable answers.
    index_hashes: Fingerprint persistent source and graph indexes.
    exercise: Index or reopen synthetic papers and check selection switches.
    main: Run the offline smoke-test command.
"""

import argparse
import asyncio
import atexit
import hashlib
import json
import logging
from pathlib import Path
import re

from offline_exec import deny_network, verify_denial


PAPERS = {"alpha": 9, "bravo": 19, "charlie": 29}
MODEL_CALLS = []


class CharacterTokenizer:
    """Provide reversible synthetic tokens for dependency-free smoke tests.

    Each Unicode code point is one token. This tokenizer is deliberately
    unsuitable for model-quality or performance comparisons.
    """

    def encode(self, text: str) -> list[int]:
        """Encode text as Unicode code points.

        Preserve every character so page spans can be checked exactly.

        Args:
            text: Synthetic source text.

        Returns:
            list[int]: Reversible character tokens.
        """
        return [ord(character) for character in text]

    def decode(self, tokens: list[int]) -> str:
        """Decode synthetic tokens without normalizing text.

        Reverse the character encoding used by this smoke test.

        Args:
            tokens: Unicode code points.

        Returns:
            str: Original synthetic text.
        """
        return "".join(chr(token) for token in tokens)


async def fake_embedding(texts: list[str], **kwargs):
    """Embed synthetic text as identical normalized vectors.

    Equal similarities make selection isolation independent of relevance.

    Args:
        texts: Synthetic passages or queries.
        **kwargs: Upstream embedding options, unused by this fake.

    Returns:
        ndarray: Eight-dimensional deterministic vectors.
    """
    import numpy as np

    return np.ones((len(texts), 8), dtype=np.float32) / (8**0.5)


async def fake_model(prompt: str, system_prompt=None, **kwargs) -> str:
    """Return graph fixtures and observable source markers.

    Use the real upstream prompts and persistence with a deterministic
    callback. This tests plumbing rather than language-model behavior.

    Args:
        prompt: Upstream user prompt.
        system_prompt: Optional upstream system prompt.
        **kwargs: Upstream generation options.

    Returns:
        str: Synthetic extraction, keywords or source-marker answer.
    """
    combined = (system_prompt or "") + "\n" + prompt
    markers = sorted(set(re.findall(r"(?:ALPHA|BRAVO|CHARLIE)_ONLY", combined)))
    MODEL_CALLS.append(markers)
    if kwargs.get("keyword_extraction"):
        return json.dumps({
            "high_level_keywords": ["Calibration"],
            "low_level_keywords": ["Shared Sensor"],
        })
    if "missed or incorrectly formatted" in prompt:
        return "<|COMPLETE|>"
    if "Knowledge Graph Specialist" in combined:
        assert len(markers) == 1, markers
        marker = markers[0]
        return (
            f"entity<|#|>Shared Sensor<|#|>Artifact<|#|>{marker} sensor.\n"
            f"entity<|#|>Calibration<|#|>Concept<|#|>{marker} calibration.\n"
            "relation<|#|>Shared Sensor<|#|>Calibration<|#|>reports<|#|>"
            f"{marker} reports calibration.\n<|COMPLETE|>"
        )
    assert len(markers) == 1, markers
    return f"{markers[0]} [1]"


def index_hashes(root: Path) -> dict[str, str]:
    """Fingerprint persistent indexes while excluding answer caches.

    Hash graph, vector, document and chunk stores. Query-result caches may
    change, but are separately checked through repeated scoped answers.

    Args:
        root: Synthetic experiment directory.

    Returns:
        dict[str, str]: Relative source-store paths and SHA-256 hashes.
    """
    names = ("vdb_", "graph_", "kv_store_text_chunks", "kv_store_full_docs")
    return {
        str(path.relative_to(root)): hashlib.sha256(
            path.read_bytes()
        ).hexdigest()
        for path in root.rglob("*")
        if path.is_file() and path.name.startswith(names)
    }


async def exercise(root: Path, phase: str) -> dict:
    """Exercise upstream indexing, page storage and selection isolation.

    Build three independent paper indexes or reopen them in another process.
    Query A+B, A+C and A+B without indexing at selection time.

    Args:
        root: Dedicated synthetic storage directory.
        phase: Either index or replay.

    Returns:
        dict: Observed isolation, page and index-identity checks.
    """
    from lightrag import LightRAG, QueryParam
    from lightrag.utils import EmbeddingFunc, Tokenizer
    from raganything import RAGAnything, RAGAnythingConfig
    from tqdm import tqdm

    instances = {}
    for paper in tqdm(PAPERS, ncols=50, ascii=True, desc=phase):
        directory = root / paper
        if phase == "index" and directory.exists():
            raise ValueError("Use a fresh directory; never overwrite a run.")
        if phase == "replay" and not directory.exists():
            raise ValueError("Index the synthetic corpus before replay.")
        directory.mkdir(parents=True, exist_ok=True)
        backend = LightRAG(
            working_dir=str(directory), workspace=paper,
            llm_model_func=fake_model, llm_model_name="synthetic-only",
            embedding_func=EmbeddingFunc(
                embedding_dim=8, max_token_size=1000,
                func=fake_embedding, model_name="synthetic-only",
            ),
            tokenizer=Tokenizer("synthetic-character", CharacterTokenizer()),
            chunk_token_size=500, chunk_overlap_token_size=0,
            entity_extract_max_gleaning=0,
            vector_db_storage_cls_kwargs={"cosine_better_than_threshold": 0},
        )
        await backend.initialize_storages()
        rag = RAGAnything(
            lightrag=backend,
            config=RAGAnythingConfig(
                working_dir=str(directory), parser="mineru",
                enable_image_processing=False, enable_table_processing=False,
                enable_equation_processing=False,
            ),
        )
        if phase == "index":
            await rag.insert_content_list(
                [{"type": "text", "page_idx": PAPERS[paper],
                  "text": (f"{paper.upper()}_ONLY "
                           "Shared Sensor reports calibration.")}],
                file_path=paper, doc_id=paper, display_stats=False,
            )
        instances[paper] = rag

    before = index_hashes(root)
    assert before, "No persistent indexes found"
    observations = []
    selections = (("alpha", "bravo"), ("alpha", "charlie"), ("alpha", "bravo"))
    for selection in selections:
        allowed = {paper.upper() + "_ONLY" for paper in selection}
        start = len(MODEL_CALLS)
        query_rows = []
        for paper in selection:
            rag = instances[paper]
            options = {
                "hl_keywords": ["Calibration"],
                "ll_keywords": ["Shared Sensor"],
                "enable_rerank": False, "top_k": 3, "chunk_top_k": 3,
                "max_total_tokens": 6000,
            }
            result = await rag.lightrag.aquery_data(
                "What does Shared Sensor report?",
                QueryParam(mode="mix", **options)
            )
            assert result["status"] == "success", result
            payload = json.dumps(result["data"])
            markers = set(re.findall(r"(?:ALPHA|BRAVO|CHARLIE)_ONLY", payload))
            assert markers == {paper.upper() + "_ONLY"}, markers
            chunks = result["data"]["chunks"]
            assert chunks
            for chunk in chunks:
                stored = await rag.lightrag.text_chunks.get_by_id(
                    chunk["chunk_id"]
                )
                assert stored["full_doc_id"] == paper, stored
                assert stored["page_idx"] == PAPERS[paper], stored
                assert stored["content"] == chunk["content"], chunk
            answer = await rag.aquery(
                "What does Shared Sensor report?", mode="mix", **options
            )
            answer_markers = re.findall(
                r"(?:ALPHA|BRAVO|CHARLIE)_ONLY", answer
            )
            assert set(answer_markers) == markers
            query_rows.append({
                "paper": paper, "answer": answer,
                "chunkCount": len(chunks),
                "storedPhysicalPage": PAPERS[paper] + 1,
                "queryReturnsPageMetadata": all(
                    "page_idx" in c for c in chunks
                ),
            })
        assert all(set(markers) <= allowed for markers in MODEL_CALLS[start:])
        observations.append({"selection": selection, "papers": query_rows})
    for rag in instances.values():
        await rag.finalize_storages()
        await rag.lightrag.llm_model_func.shutdown()
        await rag.lightrag.embedding_func.func.shutdown()
        # Manual finalization avoids a second event loop during atexit.
        atexit.unregister(rag.close)
    after = index_hashes(root)
    assert before == after, "A query changed persistent source indexes"
    return {
        "kind": "synthetic-upstream-smoke", "phase": phase,
        "scientificAnswerQualityScored": False,
        "networkPolicy": "linux-seccomp-network-denied",
        "selectionReindexCount": 0, "indexHashes": after,
        "scopeLeakCount": 0, "observations": observations,
    }


def main() -> None:
    """Run the isolated synthetic test with inherited network denial.

    Initialize the event loop's private notification pair before denying
    sockets. No upstream module or document is loaded before restriction.

    Args:
        none (None): Read command-line arguments.

    Returns:
        None: Write a local JSON observation report or raise on failure.
    """
    parser = argparse.ArgumentParser(description=(
        "Graphite upstream isolation smoke test. Inputs: a fresh local run "
        "directory and phase. Output: synthetic observations and indexes.\n"
        "Flow: 1. Deny networking. 2. Index or reopen synthetic papers. "
        "3. Check selection switches, page storage and unchanged indexes.\n"
        "Uses fake models; never scores scientific answers. Run index, then "
        "replay in a separate process with the same directory."
    ))
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--phase", choices=("index", "replay"), required=True)
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO)
    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    deny_network()
    verify_denial()
    try:
        report = loop.run_until_complete(exercise(args.root, args.phase))
        (args.root / f"{args.phase}-report.json").write_text(
            json.dumps(report, indent=2) + "\n"
        )
    finally:
        pending = asyncio.all_tasks(loop)
        for task in pending:
            task.cancel()
        if pending:
            loop.run_until_complete(asyncio.gather(
                *pending, return_exceptions=True
            ))
        loop.close()


if __name__ == "__main__":
    main()
