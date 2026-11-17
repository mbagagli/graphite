"""Measure an offline upstream pilot using Graphite's installed models.

Run the existing Rust inference providers through local pipes. Import only
prepared source text and corpus identities, never approved reference answers.
This pilot must complete before the larger selected-paper answer comparison.

Classes:
    GraphiteBridge: Own the local model subprocess and its callback trace.

Functions:
    prepared_blocks: Verify and adapt existing Docling source blocks.
    pilot: Check the runtime or index one paper through RAG-Anything.
    main: Apply network denial and run the bounded pilot command.
"""

import argparse
import asyncio
import atexit
import hashlib
import json
import logging
import os
from pathlib import Path
import subprocess
import time

from offline_exec import deny_network, verify_denial


MANIFEST_SHA256 = (
    "0f68a7d5f7524549686fa46381d3d3a8b1827ad63958fa663246df1ec79c82c7"
)
MODEL_SHA256 = (
    "d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785"
)


class GraphiteBridge:
    """Own a serial, pipe-only connection to Graphite's local providers.

    The child inherits network denial. Its model registry is read-only,
    and it validates the installed GGUF hash before accepting requests.
    Trace files retain prompts for subsequent evidence auditing.
    """

    def __init__(self, executable: Path, app: Path, embeddings: Path,
                 output: Path) -> None:
        """Start the pinned local model bridge.

        Keep callbacks serial so neither model requests nor pipe responses
        can be interleaved across papers.

        Args:
            executable: Compiled opt-in Rust test executable.
            app: Existing application model-registry directory.
            embeddings: Existing isolated Graphite embedding assets.
            output: Fresh pilot output directory.

        Returns:
            None: Initialize the subprocess and verify its handshake.
        """
        self.trace = output / "model-calls.jsonl"
        self.error_log = (output / "runtime.log").open("w")
        environment = dict(os.environ)
        environment["GRAPHITE_COMPARISON_APP_ROOT"] = str(app)
        environment["GRAPHITE_COMPARISON_EMBEDDINGS"] = str(embeddings)
        self.process = subprocess.Popen(
            [str(executable), "upstream_local_model_bridge", "--ignored",
             "--nocapture", "--test-threads=1"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=self.error_log, text=True, env=environment,
        )
        logging.info("Waiting for local model identity verification")
        self.identity = self.receive()
        assert self.identity["ready"]
        assert self.identity["modelSha256"] == MODEL_SHA256
        assert self.identity["embedding"]["dimension"] == 384
        assert self.identity["embedding"]["modelRevision"] == (
            "52398278842ec682c6f32300af41344b1c0b0bb2"
        )

    def receive(self) -> dict:
        """Read one bridge response while ignoring test-runner headings.

        Fail on premature process termination rather than returning an
        empty result that could look like a valid model refusal.

        Args:
            self: The current bridge.

        Returns:
            dict: One decoded protocol response.
        """
        while line := self.process.stdout.readline():
            if "GRAPHITE_BRIDGE " in line:
                return json.loads(line.partition("GRAPHITE_BRIDGE ")[2])
        raise RuntimeError("Local model bridge exited before responding.")

    def request(self, command: dict) -> dict:
        """Run and record one bounded inference operation.

        Retain actual generation prompts and results. Embedding vectors are
        summarized by shape and digest to avoid duplicating large arrays.

        Args:
            command: JSON protocol request for the production provider.

        Returns:
            dict: Successful inference result.
        """
        started = time.monotonic()
        self.process.stdin.write(json.dumps(command) + "\n")
        self.process.stdin.flush()
        response = self.receive()
        recorded = dict(response)
        if command["op"] == "embed" and response["ok"]:
            vectors = response["result"]["vectors"]
            recorded = {"ok": True, "vectorCount": len(vectors),
                        "sha256": hashlib.sha256(
                            json.dumps(vectors).encode()).hexdigest()}
        with self.trace.open("a") as output:
            output.write(json.dumps({
                "request": command, "response": recorded,
                "elapsedSeconds": time.monotonic() - started,
            }) + "\n")
        if not response["ok"]:
            raise RuntimeError(response["error"])
        return response["result"]

    async def embed(self, texts: list[str], **kwargs):
        """Use Graphite's existing BGE document or query embedding path.

        Preserve the same prefixes, normalization and ONNX model assets.

        Args:
            texts: Upstream document passages or queries.
            **kwargs: Upstream options, including embedding context.

        Returns:
            ndarray: Production BGE vectors.
        """
        import numpy as np

        result = self.request({"op": "embed", "texts": texts,
                               "context": kwargs.get("context", "document")})
        return np.asarray(result["vectors"], dtype=np.float32)

    async def model(self, prompt: str, system_prompt: str | None = None,
                    history_messages: list | None = None, **kwargs) -> str:
        """Generate upstream indexing text with the installed Qwen model.

        Use a separate, explicit indexing output cap. Scientific answers
        remain subject to the unchanged 420-token budget in the later run.
        Reject capped or timed-out responses before graph insertion.

        Args:
            prompt: Upstream extraction or summary prompt.
            system_prompt: Upstream system instructions.
            history_messages: Internal history; unsupported in this pilot.
            **kwargs: Upstream options, including an optional token cap.

        Returns:
            str: Actual local model output.
        """
        if history_messages:
            raise ValueError("Pilot does not accept conversation carryover.")
        token_limit = min(int(kwargs.get("max_tokens") or 1024), 1024)
        result = self.request({
            "op": "generate", "prompt": prompt,
            "system": system_prompt or "",
            "maxTokens": token_limit,
        })
        if result["timedOut"] or result["generatedTokens"] >= token_limit:
            raise RuntimeError(
                "Index generation reached its time or output limit; "
                "refusing to accept a potentially incomplete extraction."
            )
        return result["text"]

    def close(self) -> None:
        """Stop and reap the owned bridge process.

        A short graceful exit is followed by termination if necessary.
        No unrelated process is signaled.

        Args:
            self: The current bridge.

        Returns:
            None: Close owned process handles.
        """
        if self.process.poll() is None:
            self.process.stdin.write('{"op":"quit"}\n')
            self.process.stdin.flush()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.process.stdin.close()
        self.process.stdout.close()
        self.error_log.close()


def prepared_blocks(repo: Path, paper: str) -> tuple[list[dict], dict]:
    """Verify and adapt the same prepared Docling inputs as the baseline.

    Preserve text and native table bodies. Only known single-page blocks
    receive a page index; multi-page or unknown blocks are not guessed.

    Args:
        repo: Graphite repository root.
        paper: Stable corpus document identifier.

    Returns:
        tuple[list[dict], dict]: Upstream blocks and verified input identity.
    """
    folder = repo / ".cache/rag-comparison/prepared-docling-v1"
    raw = (folder / "manifest.json").read_bytes()
    assert hashlib.sha256(raw).hexdigest() == MANIFEST_SHA256
    entry = next(d for d in json.loads(raw)["documents"] if d["id"] == paper)
    raw = (folder / entry["zoteroBaseline"]).read_bytes()
    assert hashlib.sha256(raw).hexdigest() == entry["zoteroBaselineSha256"]
    source = json.loads(raw)
    corpus_path = repo / "evaluation/upstream-rag-v1/corpus.json"
    corpus = json.loads(corpus_path.read_text())
    document = next(d for d in corpus["documents"] if d["id"] == paper)
    assert source["documentSha256"] == document["pdfSha256"]
    blocks = []
    for span in source["spans"]:
        text = source["content"][span["start"]:span["end"]]
        block = {"type": "text", "text": text}
        if ":#/tables/" in span["blockRef"]:
            block = {"type": "table", "table_body": text,
                     "table_caption": [], "table_footnote": []}
        if (span["pageStart"] is not None
                and span["pageStart"] == span["pageEnd"]):
            block["page_idx"] = span["pageStart"] - 1
        blocks.append(block)
    return blocks, {"paper": paper, "preparedManifestSha256": MANIFEST_SHA256,
                    "preparedTextSha256": entry["zoteroBaselineSha256"],
                    "pdfSha256": source["documentSha256"],
                    "blockCount": len(blocks)}


async def pilot(args: argparse.Namespace) -> dict:
    """Verify production model callbacks or index one existing paper.

    Require a fresh output directory and real offline providers. The outer
    supervisor enforces the approved 20-minute pilot wall-clock cap.

    Args:
        args: Parsed command-line options.

    Returns:
        dict: Runtime verification or measured indexing observations.
    """
    logging.info("Loading installed upstream libraries")
    from lightrag import LightRAG
    from lightrag.utils import EmbeddingFunc
    from raganything import RAGAnything, RAGAnythingConfig

    logging.info("Starting the local model bridge")
    bridge = GraphiteBridge(
        args.executable, args.app, args.embeddings, args.output
    )
    rag = None
    try:
        if args.check_runtime:
            vectors = await bridge.embed(["Synthetic sensor calibration."])
            repeated = await bridge.embed(["Synthetic sensor calibration."])
            assert vectors.shape == (1, 384)
            assert (vectors == repeated).all()
            assert abs(float((vectors[0] ** 2).sum()) - 1) < 0.00001
            answer = bridge.request({
                "op": "generate", "system": "Answer briefly.",
                "prompt": "Reply with the single word READY.",
                "maxTokens": 8,
            })
            assert answer["text"].strip()
            return {"status": "passed", "identity": bridge.identity,
                    "deterministicEmbedding": True, "generation": answer}
        blocks, identity = prepared_blocks(args.repo, args.paper)
        async def model_callback(*args, **kwargs):
            """Forward generation without copying the owned pipe handles.

            LightRAG copies its configuration, so use a plain callback.

            Args:
                *args: Upstream generation arguments.
                **kwargs: Upstream generation options.

            Returns:
                str: Local model response.
            """
            return await bridge.model(*args, **kwargs)

        async def embedding_callback(*args, **kwargs):
            """Forward embeddings without copying the owned pipe handles.

            Keep the subprocess owner outside upstream configuration.

            Args:
                *args: Upstream embedding arguments.
                **kwargs: Upstream embedding options.

            Returns:
                ndarray: Local embedding vectors.
            """
            return await bridge.embed(*args, **kwargs)

        backend = LightRAG(
            working_dir=str(args.output / "index"), workspace=args.paper,
            llm_model_func=model_callback, llm_model_name="graphite-qwen3-8b",
            llm_model_max_async=1, embedding_func_max_async=1,
            embedding_batch_num=16, max_parallel_insert=1,
            embedding_func=EmbeddingFunc(
                embedding_dim=384, max_token_size=512, func=embedding_callback,
                model_name="graphite-bge-small-en-v1.5-onnx-q",
            ),
            chunk_token_size=360, chunk_overlap_token_size=36,
            entity_extract_max_gleaning=0,
        )
        await backend.initialize_storages()
        rag = RAGAnything(lightrag=backend, config=RAGAnythingConfig(
            working_dir=str(args.output / "index"), parser="mineru",
            enable_image_processing=False, enable_table_processing=True,
            enable_equation_processing=False,
        ))
        (args.output / "input-identity.json").write_text(
            json.dumps(identity, indent=2)
        )
        started = time.monotonic()
        await rag.insert_content_list(blocks, file_path=args.paper,
                                      doc_id=args.paper, display_stats=False)
        status = await backend.doc_status.get_by_id(args.paper)
        if not status or status.get("status") != "processed":
            raise RuntimeError("Upstream document indexing did not complete.")
        return {"status": "indexed", "identity": bridge.identity,
                "input": identity, "indexSeconds": time.monotonic() - started,
                "chunkTokens": 360, "overlapTokens": 36,
                "indexGenerationTokenLimit": 1024,
                "scientificAnswerQualityScored": False}
    finally:
        if rag is not None:
            await rag.finalize_storages()
            await rag.lightrag.llm_model_func.shutdown()
            await rag.lightrag.embedding_func.func.shutdown()
            atexit.unregister(rag.close)
        bridge.close()


def main() -> None:
    """Run the offline pilot and retain failures as explicit observations.

    Create only the event loop's private notification pair before denying
    networking. Import upstream code and read source documents afterward.

    Args:
        none (None): Read command-line options.

    Returns:
        None: Write a local report, or record and re-raise a failure.
    """
    parser = argparse.ArgumentParser(description=(
        "Graphite upstream local-model pilot. Inputs: pinned Rust bridge, "
        "existing model roots and one prepared paper. Output: fresh index, "
        "model-call trace and report.\nFlow: 1. Deny networking. "
        "2. Verify installed model identities. 3. Check callbacks or index "
        "one paper. Requires an external 20-minute wall-clock supervisor; "
        "does not run or score the approved answer questions."
    ))
    for name in ("repo", "executable", "app", "embeddings", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--paper", default="paper20")
    parser.add_argument("--check-runtime", action="store_true")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    logging.basicConfig(level=logging.INFO)
    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    deny_network()
    verify_denial()
    try:
        report = loop.run_until_complete(pilot(args))
        (args.output / "report.json").write_text(
            json.dumps(report, indent=2) + "\n"
        )
    except Exception as error:
        (args.output / "report.json").write_text(json.dumps({
            "status": "failed", "reason": str(error),
            "scientificAnswerQualityScored": False,
        }, indent=2) + "\n")
        raise
    finally:
        pending = asyncio.all_tasks(loop)
        for task in pending:
            task.cancel()
        if pending:
            loop.run_until_complete(
                asyncio.gather(*pending, return_exceptions=True)
            )
        loop.close()


if __name__ == "__main__":
    main()
