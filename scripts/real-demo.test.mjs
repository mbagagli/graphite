import { Buffer } from "node:buffer";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { URL } from "node:url";

import { describe, expect, it } from "vitest";

import {
  createRdf,
  inspectPdf,
  prepareDemo,
  validateManifest,
  validatePdfBytes,
} from "./real-demo.mjs";

const manifest = JSON.parse(
  await readFile(new URL("../demo/manifest.json", import.meta.url), "utf8"),
);

function malformedPaper(bytes) {
  return {
    ...manifest.papers[0],
    pdfBytes: bytes.length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  };
}

const connections = JSON.parse(
  await readFile(new URL("../demo/connections.json", import.meta.url), "utf8"),
);

describe("real seismology demonstration", () => {
  it("contains exactly 60 licensed papers with distinct identifiers and PDFs", () => {
    expect(validateManifest(manifest)).toBe(manifest);
    expect(
      new Set(manifest.papers.map((paper) => paper.year)).size,
    ).toBeGreaterThan(5);
  });

  it("ties every addition to an original paper through reviewed PDF references", () => {
    const papers = new Map(manifest.papers.map((paper) => [paper.id, paper]));
    const originals = new Set(connections.originalPaperIds);
    expect(connections.manifestId).toBe(manifest.id);
    expect(originals.size).toBe(30);
    expect(connections.addedPaperIds).toHaveLength(30);
    expect(new Set([...originals, ...connections.addedPaperIds])).toEqual(
      new Set(papers.keys()),
    );
    const pairs = new Set();
    for (const edge of connections.edges) {
      const source = papers.get(edge.source);
      expect(source).toBeDefined();
      expect(papers.has(edge.target)).toBe(true);
      expect(edge.source).not.toBe(edge.target);
      expect(edge.sourcePdfSha256).toBe(source.sha256);
      expect(edge.pdfPage).toBeGreaterThanOrEqual(1);
      expect(edge.pdfPage).toBeLessThanOrEqual(source.pdfPages);
      expect(edge.referenceTitle.trim()).not.toBe("");
      pairs.add(`${edge.source}:${edge.target}`);
    }
    expect(pairs.size).toBe(connections.edges.length);
    for (const id of connections.addedPaperIds) {
      expect(originals.has(id)).toBe(false);
      expect(
        connections.edges.some(
          (edge) =>
            (edge.source === id && originals.has(edge.target)) ||
            (edge.target === id && originals.has(edge.source)),
        ),
      ).toBe(true);
    }
  });

  it("rejects duplicates, missing records, unsafe paths, and unreviewed rights", () => {
    const incomplete = globalThis.structuredClone(manifest);
    incomplete.papers.pop();
    expect(() => validateManifest(incomplete)).toThrow("exactly 60");
    for (const key of ["id", "doi", "sha256"]) {
      const duplicate = globalThis.structuredClone(manifest);
      duplicate.papers[1][key] = duplicate.papers[0][key];
      expect(() => validateManifest(duplicate)).toThrow("Duplicate");
    }
    for (const [key, value] of [
      ["id", "../../outside"],
      ["license", "unknown"],
      ["pdfBytes", 100_000_000],
      ["pdfUrl", "http://127.0.0.1/private"],
      ["pdfUrl", "https://unreviewed.example/paper.pdf"],
    ]) {
      const invalid = globalThis.structuredClone(manifest);
      invalid.papers[0][key] = value;
      expect(() => validateManifest(invalid)).toThrow();
    }
  });

  it("detects checksum changes and HTML masquerading as a PDF", () => {
    const bytes = Buffer.from("%PDF-1.7\nnot a valid document");
    const paper = malformedPaper(bytes);
    expect(() => validatePdfBytes(paper, bytes)).not.toThrow();
    const corrupted = Buffer.from(bytes);
    corrupted[corrupted.length - 1] ^= 1;
    expect(() => validatePdfBytes(paper, corrupted)).toThrow("checksum");
    const html = Buffer.from("<html>publisher error</html>");
    expect(() => validatePdfBytes(malformedPaper(html), html)).toThrow(
      "header",
    );
  });

  it("rejects malformed PDF content even with a matching checksum", async () => {
    const bytes = Buffer.from("%PDF-1.7\nnot a valid document");
    await expect(inspectPdf(malformedPaper(bytes), bytes)).rejects.toThrow(
      "parsing",
    );
  });

  it("creates deterministic RDF with 60 relative PDF attachments and attribution", () => {
    const rdf = createRdf(manifest);
    expect(createRdf(globalThis.structuredClone(manifest))).toBe(rdf);
    expect(rdf.match(/<bib:Article /g)).toHaveLength(60);
    expect(rdf.match(/<z:Attachment /g)).toHaveLength(60);
    expect(rdf.match(/<z:path rdf:resource="pdfs\//g)).toHaveLength(60);
    expect(rdf).not.toContain("file://");
    expect(rdf).toContain("CC-BY-3.0");
    expect(rdf).toContain("CC-BY-4.0");
  });

  it("reports missing PDFs without contacting publishers in offline mode", async () => {
    const directory = await mkdtemp(join(tmpdir(), "graphite-demo-test-"));
    try {
      await expect(prepareDemo(manifest, directory)).rejects.toThrow(
        "Missing PDF",
      );
      await expect(
        readFile(join(directory, "collection.rdf")),
      ).rejects.toMatchObject({
        code: "ENOENT",
      });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
