import { Buffer } from "node:buffer";
import { createHash } from "node:crypto";
import { mkdir, readFile, rename, stat, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath, URL } from "node:url";
import { parseArgs } from "node:util";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const MAX_PDF_BYTES = 64 * 1024 * 1024;
const PUBLISHERS = new Set([
  "se.copernicus.org",
  "nhess.copernicus.org",
  "gmd.copernicus.org",
  "npg.copernicus.org",
  "gi.copernicus.org",
  "essd.copernicus.org",
  "esurf.copernicus.org",
  "seismica.library.mcgill.ca",
  "authors.library.caltech.edu",
  "link.springer.com",
  "flore.unifi.it",
  "raw.githubusercontent.com",
  "doi.org",
  "www.nature.com",
  "www.frontiersin.org",
]);

export function validateManifest(manifest) {
  if (
    manifest.schemaVersion !== 1 ||
    manifest.id !== "seismology-ml-v2" ||
    manifest.papers?.length !== 60
  ) {
    throw new Error("The version 2 demo must contain exactly 60 papers.");
  }
  const ids = new Set();
  const dois = new Set();
  const hashes = new Set();
  for (const paper of manifest.papers) {
    if (
      !/^[a-z0-9][a-z0-9.-]+$/.test(paper.id) ||
      !/^10\.\d{4,9}\/\S+$/i.test(paper.doi) ||
      !/^[a-f0-9]{64}$/.test(paper.sha256) ||
      !paper.title?.trim() ||
      !paper.journal?.trim() ||
      !paper.authors?.length ||
      !paper.authors.every(
        (author) => typeof author === "string" && author.trim(),
      ) ||
      !Number.isInteger(paper.year) ||
      paper.year < 1900 ||
      !Number.isInteger(paper.pdfBytes) ||
      paper.pdfBytes < 5 ||
      paper.pdfBytes > MAX_PDF_BYTES ||
      !Number.isInteger(paper.pdfPages) ||
      paper.pdfPages < 1 ||
      !["CC-BY-2.0", "CC-BY-3.0", "CC-BY-4.0"].includes(paper.license) ||
      paper.licenseUrl !==
        `https://creativecommons.org/licenses/by/${paper.license.slice(-3)}/` ||
      !paper.attribution?.trim() ||
      !paper.themes?.length ||
      !paper.themes.every((theme) => typeof theme === "string" && theme.trim())
    ) {
      throw new Error(
        "The demo contains an incomplete or invalid paper record.",
      );
    }
    for (const key of ["sourceUrl", "pdfUrl", "rightsEvidenceUrl"]) {
      const url = new URL(paper[key]);
      if (
        url.protocol !== "https:" ||
        !PUBLISHERS.has(url.hostname) ||
        url.username ||
        url.password ||
        url.port
      ) {
        throw new Error(`Unapproved publisher URL for ${paper.id}.`);
      }
    }
    if (
      ids.has(paper.id) ||
      dois.has(paper.doi.toLowerCase()) ||
      hashes.has(paper.sha256)
    ) {
      throw new Error(`Duplicate paper or PDF in the demo: ${paper.id}.`);
    }
    ids.add(paper.id);
    dois.add(paper.doi.toLowerCase());
    hashes.add(paper.sha256);
  }
  return manifest;
}

export function validatePdfBytes(paper, bytes) {
  if (bytes.length !== paper.pdfBytes || bytes.length > MAX_PDF_BYTES) {
    throw new Error(
      `PDF size mismatch for ${paper.id}; reacquire the original.`,
    );
  }
  if (bytes.subarray(0, 5).toString() !== "%PDF-") {
    throw new Error(`Invalid PDF header for ${paper.id}.`);
  }
  const digest = createHash("sha256").update(bytes).digest("hex");
  if (digest !== paper.sha256) {
    throw new Error(`PDF checksum mismatch for ${paper.id}; do not import it.`);
  }
}

function normalized(value) {
  return value
    .normalize("NFKD")
    .toLowerCase()
    .replace(/[^a-z0-9]/g, "");
}

export async function inspectPdf(paper, bytes) {
  validatePdfBytes(paper, bytes);
  const { getDocument } = await import("pdfjs-dist/legacy/build/pdf.mjs");
  const task = getDocument({
    data: new Uint8Array(bytes),
    isEvalSupported: false,
    useSystemFonts: false,
    disableFontFace: true,
    verbosity: 0,
  });
  let document;
  try {
    document = await task.promise;
    if (document.numPages !== paper.pdfPages) {
      throw new Error("Page count differs from the manifest.");
    }
    const pages = [];
    for (let index = 1; index <= document.numPages; index += 1) {
      const page = await document.getPage(index);
      const content = await page.getTextContent();
      pages.push(
        content.items
          .map((item) =>
            "str" in item ? `${item.str}${item.hasEOL ? "\n" : " "}` : "",
          )
          .join(""),
      );
      page.cleanup();
    }
    const opening = normalized(pages.slice(0, 2).join(" "));
    if (
      !opening.includes(normalized(paper.doi)) ||
      !opening.includes(normalized(paper.title))
    ) {
      throw new Error("The title or DOI differs from the manifest.");
    }
    if (pages.join("").length < 1000) {
      throw new Error("The PDF has insufficient extractable text.");
    }
    return pages;
  } catch {
    throw new Error(
      `PDF parsing or metadata validation failed for ${paper.id}.`,
    );
  } finally {
    await task.destroy();
  }
}

function xml(value) {
  return String(value).replace(
    /[<>&"']/g,
    (character) =>
      ({
        "<": "&lt;",
        ">": "&gt;",
        "&": "&amp;",
        '"': "&quot;",
        "'": "&apos;",
      })[character],
  );
}

export function createRdf(manifest) {
  validateManifest(manifest);
  const entries = manifest.papers
    .map((paper) => {
      const authors = paper.authors
        .map((author) => {
          const [family, ...given] = author.split(",");
          return (
            `<rdf:li><foaf:Person><foaf:surname>${xml(family.trim())}</foaf:surname>` +
            `<foaf:givenName>${xml(given.join(",").trim())}</foaf:givenName></foaf:Person></rdf:li>`
          );
        })
        .join("\n");
      const tags = paper.themes
        .map((theme) => `<dc:subject>${xml(theme)}</dc:subject>`)
        .join("\n");
      return `<bib:Article rdf:about="urn:doi:${xml(paper.doi)}">
<z:itemType>journalArticle</z:itemType>
<dc:title>${xml(paper.title)}</dc:title>
<dc:date>${paper.year}</dc:date>
<dc:identifier>DOI ${xml(paper.doi)}</dc:identifier>
<dc:rights>${xml(paper.attribution)}</dc:rights>
<dc:description>Graphite demonstration collection: ${manifest.id}</dc:description>
<bib:authors><rdf:Seq>${authors}</rdf:Seq></bib:authors>
<dcterms:isPartOf><bib:Journal><dc:title>${xml(paper.journal)}</dc:title></bib:Journal></dcterms:isPartOf>
${tags}
<link:link rdf:resource="#pdf-${paper.id}"/>
</bib:Article>
<z:Attachment rdf:about="#pdf-${paper.id}">
<z:itemType>attachment</z:itemType>
<dc:title>${xml(paper.title)} (publisher PDF)</dc:title>
<link:type>application/pdf</link:type>
<z:path rdf:resource="pdfs/${paper.id}.pdf"/>
</z:Attachment>`;
    })
    .join("\n");
  const members = manifest.papers
    .map(
      (paper) => `<dcterms:hasPart rdf:resource="urn:doi:${xml(paper.doi)}"/>`,
    )
    .join("\n");
  return `<?xml version="1.0" encoding="UTF-8"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
 xmlns:bib="http://purl.org/net/biblio#"
 xmlns:dc="http://purl.org/dc/elements/1.1/"
 xmlns:dcterms="http://purl.org/dc/terms/"
 xmlns:foaf="http://xmlns.com/foaf/0.1/"
 xmlns:link="http://purl.org/rss/1.0/modules/link/"
 xmlns:z="http://www.zotero.org/namespaces/export#">
${entries}
<z:Collection rdf:about="#collection-${manifest.id}">
<dc:title>${xml(manifest.title)}</dc:title>
${members}
</z:Collection>
</rdf:RDF>\n`;
}

async function downloadPdf(paper) {
  const response = await globalThis.fetch(paper.pdfUrl, {
    signal: globalThis.AbortSignal.timeout(120_000),
  });
  if (!response.ok || !response.body) {
    throw new Error(`Publisher download failed for ${paper.id}; retry later.`);
  }
  const parts = [];
  let size = 0;
  for await (const part of response.body) {
    size += part.byteLength;
    if (size > MAX_PDF_BYTES) {
      throw new Error(
        `Publisher download exceeds the size limit for ${paper.id}.`,
      );
    }
    parts.push(part);
  }
  return Buffer.concat(parts);
}

export async function prepareDemo(manifest, directory, download = false) {
  validateManifest(manifest);
  const pdfDirectory = join(directory, "pdfs");
  const textDirectory = join(directory, "text");
  await mkdir(pdfDirectory, { recursive: true });
  await mkdir(textDirectory, { recursive: true });
  const evidence = [];
  for (const paper of manifest.papers) {
    const filename = join(pdfDirectory, `${paper.id}.pdf`);
    let bytes;
    try {
      const info = await stat(filename);
      if (!info.isFile() || info.size > MAX_PDF_BYTES) {
        throw new Error(`Invalid local PDF for ${paper.id}.`);
      }
      bytes = await readFile(filename);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      if (!download) {
        throw new Error(`Missing PDF for ${paper.id}; run with --download.`, {
          cause: error,
        });
      }
      bytes = await downloadPdf(paper);
      validatePdfBytes(paper, bytes);
      await writeFile(`${filename}.partial`, bytes, { flag: "wx" });
      await rename(`${filename}.partial`, filename);
    }
    const pages = await inspectPdf(paper, bytes);
    await writeFile(
      join(textDirectory, `${paper.id}.txt`),
      pages.join("\n\f\n"),
    );
    evidence.push({ id: paper.id, sha256: paper.sha256, pages: pages.length });
    process.stdout.write(
      `Verified ${evidence.length}/${manifest.papers.length}: ${paper.id}\n`,
    );
  }
  await writeFile(join(directory, "collection.rdf"), createRdf(manifest));
  await writeFile(
    join(directory, "ATTRIBUTION.txt"),
    manifest.papers
      .map((paper) => `${paper.attribution}\n${paper.licenseUrl}`)
      .join("\n\n") + "\n",
  );
  const receipt = {
    manifestId: manifest.id,
    pdfCount: manifest.papers.length,
    evidence,
  };
  await writeFile(
    join(directory, "verification.json"),
    JSON.stringify(receipt, null, 2) + "\n",
  );
  return receipt;
}

async function main() {
  const { values } = parseArgs({
    options: {
      download: { type: "boolean", default: false },
      directory: { type: "string", default: "artifacts/real-demo" },
      help: { type: "boolean", default: false },
    },
    strict: true,
  });
  if (values.help) {
    process.stdout.write(`Graphite real-paper demo preparation

Verify 60 publisher PDFs against demo/manifest.json and prepare a Zotero RDF
collection with attached PDFs, attribution, extracted text, and a receipt.
Offline verification is the default. --download explicitly contacts the
listed publishers for missing PDFs only; mismatched existing PDFs fail.

Flow:
  1. Validate distinct records and reviewed reuse licenses.
  2. Verify bytes, SHA-256, page count, title, DOI, and extractable text.
  3. Write one deterministic collection for an isolated Zotero profile.

Usage: node scripts/real-demo.mjs [--download] [--directory DIRECTORY]
Example: node scripts/real-demo.mjs --download
No Zotero library is opened or modified by this command.
`);
    return;
  }
  const manifest = JSON.parse(
    await readFile(join(ROOT, "demo/manifest.json"), "utf8"),
  );
  await prepareDemo(manifest, resolve(values.directory), values.download);
  process.stdout.write(
    "Verified 60 distinct PDFs; collection.rdf is ready for isolated import.\n",
  );
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
