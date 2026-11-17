import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  link,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import process from "node:process";
import { spawnSync } from "node:child_process";
import { fileURLToPath, URL } from "node:url";
import { parseArgs } from "node:util";
import { validateManifest, validatePdfBytes } from "./real-demo.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ARCHIVE_TIMESTAMP = "1980-01-01T00:00:00";
const ZIP_SCRIPT = `"""Package a reviewed staging directory with fixed ZIP metadata."""
import hashlib
from pathlib import Path
import sys
import zipfile

source, destination = map(Path, sys.argv[1:])
files = sorted(path for path in source.rglob("*") if path.is_file())
with zipfile.ZipFile(destination, "x", compression=zipfile.ZIP_STORED) as archive:
    for path in files:
        name = source.name + "/" + path.relative_to(source).as_posix()
        info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
        info.create_system = 3
        info.external_attr = 0o100644 << 16
        archive.writestr(info, path.read_bytes())
with zipfile.ZipFile(destination) as archive:
    if archive.testzip() is not None:
        raise ValueError("Archive CRC verification failed")
    checksums = (source / "SHA256SUMS").read_text().splitlines()
    for line in checksums:
        expected, name = line.split("  ", 1)
        actual = hashlib.sha256(archive.read(source.name + "/" + name))
        if actual.hexdigest() != expected:
            raise ValueError("Archive checksum verification failed")
`;

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function json(value) {
  return JSON.stringify(value, null, 2) + "\n";
}

async function fileDigest(filename) {
  const hash = createHash("sha256");
  for await (const bytes of createReadStream(filename)) hash.update(bytes);
  return hash.digest("hex");
}

export function validateRightsReview(manifest, rights) {
  validateManifest(manifest);
  if (
    rights.schemaVersion !== 1 ||
    rights.manifestId !== manifest.id ||
    !/^\d{4}-\d{2}-\d{2}$/.test(rights.reviewedAt) ||
    !rights.method?.trim() ||
    rights.papers?.length !== manifest.papers.length
  ) {
    throw new Error("A complete rights review is required for this archive.");
  }
  const reviewed = new Map(rights.papers.map((paper) => [paper.id, paper]));
  for (const paper of manifest.papers) {
    const record = reviewed.get(paper.id);
    if (
      !record ||
      record.pdfSha256 !== paper.sha256 ||
      record.license !== paper.license ||
      record.licenseUrl !== paper.licenseUrl ||
      record.evidenceUrl !== paper.rightsEvidenceUrl ||
      !["publisher-pdf", "publisher-article-page"].includes(
        record.evidenceKind,
      ) ||
      !record.reviewNote?.trim() ||
      !Array.isArray(record.licensePages) ||
      (record.evidenceKind === "publisher-pdf" &&
        !record.licensePages.length) ||
      !record.licensePages.every(
        (page) => Number.isInteger(page) && page >= 1 && page <= paper.pdfPages,
      ) ||
      !Array.isArray(record.additionalAttribution) ||
      !record.additionalAttribution.every(
        (value) => typeof value === "string",
      ) ||
      !Array.isArray(record.additionalEvidenceUrls) ||
      !record.additionalEvidenceUrls.every((value) => {
        try {
          const url = new URL(value);
          return url.protocol === "https:" && !url.username && !url.password;
        } catch {
          return false;
        }
      })
    ) {
      throw new Error(`Missing or stale rights review for ${paper.id}.`);
    }
  }
  return rights;
}

export function archiveMetadata(manifest, rights) {
  validateRightsReview(manifest, rights);
  const attribution =
    manifest.papers
      .map((paper) => {
        const review = rights.papers.find((item) => item.id === paper.id);
        return [
          paper.attribution,
          paper.licenseUrl,
          `Publisher and license evidence: ${paper.rightsEvidenceUrl}`,
          ...review.additionalAttribution,
          ...review.additionalEvidenceUrls,
        ].join("\n");
      })
      .join("\n\n") + "\n";
  return {
    "manifest.json": json(manifest),
    "RIGHTS.json": json(rights),
    "ATTRIBUTION.txt": attribution,
    "README.txt":
      `Graphite real-paper offline demo: ${manifest.id}\n\n` +
      `This archive contains ${manifest.papers.length} unchanged open-access PDFs and their attribution.\n` +
      "Each paper retains its own license; Graphite's application license does\n" +
      "not replace the article licenses. See ATTRIBUTION.txt and RIGHTS.json.\n\n" +
      "Open Graphite Desktop and choose Data Source > Use demo data.\n" +
      "Choose Prepare library for AI. For a ZIP already on your computer,\n" +
      "choose Already downloaded? in that flow and select this ZIP directly.\n" +
      "Do not unpack it first. The online option downloads the published\n" +
      "Zenodo archive when its version-specific address is configured.\n" +
      `Graphite verifies all ${manifest.papers.length} PDFs against its bundled manifest.\n` +
      "It prepares document text, citation connections and semantic indexing;\n" +
      "the progress bar shows the current stage. Verified completed work is\n" +
      "kept if you cancel. Use Resume preparation to continue later.\n\n" +
      "Collections share the normal library workflow: all papers in a root\n" +
      "collection, with child collections for their curated subject themes.\n\n" +
      "Models are separate downloads. Install the embedding/search model in\n" +
      "Data Source when prompted, then resume preparation. Install an answer\n" +
      "model in TYRION > Setup before asking questions, or configure an\n" +
      "existing supported local AI app there. Library prepared means its\n" +
      "available documents are prepared; Ready to ask TYRION also requires\n" +
      "an available answer provider. No model is downloaded automatically.\n" +
      "Open a prepared paper, ask a focused question and check the cited\n" +
      "passages in the original PDF. Metadata alone is not document evidence.\n\n" +
      "Offline ZIP import requires no publisher or Zotero access. After models\n" +
      "are installed, preparation and questions run locally. Missing or scanned\n" +
      "documents are reported rather than invented. Graphite stores the demo\n" +
      "separately from your Zotero library and never uploads research text.\n\n" +
      "SHA256SUMS covers every other file in this folder. Keep the attribution\n" +
      "and rights files with any redistributed copy. No PDFs were modified.\n",
  };
}

async function publishArchive(temporary, filename, expected) {
  try {
    await link(temporary, filename);
  } catch (error) {
    if (error.code !== "EEXIST") throw error;
    const existing = await lstat(filename);
    if (!existing.isFile() || (await fileDigest(filename)) !== expected) {
      throw new Error("Output archive already exists with different bytes.", {
        cause: error,
      });
    }
  }
}

async function writeReceipt(filename, content) {
  try {
    await writeFile(filename, content, { flag: "wx" });
  } catch (error) {
    if (error.code !== "EEXIST") throw error;
    if (
      !(await lstat(filename)).isFile() ||
      (await readFile(filename, "utf8")) !== content
    ) {
      throw new Error("Output receipt exists with different content.", {
        cause: error,
      });
    }
  }
}

export async function createDemoArchive(
  manifest,
  rights,
  input,
  output,
  { publicationBundle = false } = {},
) {
  const metadata = archiveMetadata(manifest, rights);
  await mkdir(output, { recursive: true });
  const temporary = await mkdtemp(join(output, ".demo-archive-"));
  const archiveRoot = `graphite-demo-${manifest.id}`;
  const filename = `${archiveRoot}-r1.zip`;
  const staging = join(temporary, archiveRoot);
  try {
    await mkdir(join(staging, "pdfs"), { recursive: true });
    const checksums = [];
    for (const paper of manifest.papers) {
      const relative = `pdfs/${paper.id}.pdf`;
      const source = join(input, relative);
      let info;
      try {
        info = await lstat(source);
      } catch (error) {
        if (error.code !== "ENOENT") throw error;
        throw new Error(
          `Missing PDF for ${paper.id}; prepare the demo first.`,
          { cause: error },
        );
      }
      if (!info.isFile() || info.size !== paper.pdfBytes) {
        throw new Error(`Invalid local PDF for ${paper.id}.`);
      }
      const bytes = await readFile(source);
      validatePdfBytes(paper, bytes);
      await writeFile(join(staging, relative), bytes, { flag: "wx" });
      checksums.push(`${paper.sha256}  ${relative}`);
    }
    for (const [name, content] of Object.entries(metadata)) {
      await writeFile(join(staging, name), content, { flag: "wx" });
      checksums.push(`${digest(content)}  ${name}`);
    }
    await writeFile(
      join(staging, "SHA256SUMS"),
      checksums.sort().join("\n") + "\n",
    );
    const zipPath = join(temporary, filename);
    const result = spawnSync("python3", ["-c", ZIP_SCRIPT, staging, zipPath], {
      encoding: "utf8",
      timeout: 120_000,
      maxBuffer: 1024 * 1024,
    });
    if (result.error || result.status !== 0) {
      throw new Error(
        "Archive creation failed; Python 3 with zipfile is required.",
      );
    }
    const sha256 = await fileDigest(zipPath);
    const receipt = {
      schemaVersion: 1,
      manifestId: manifest.id,
      manifestSha256: digest(metadata["manifest.json"]),
      rightsSha256: digest(metadata["RIGHTS.json"]),
      metadataHashScope:
        "Exact UTF-8 bytes of manifest.json and RIGHTS.json inside the ZIP; " +
        "JSON.stringify(value, null, 2) followed by LF, not repository file bytes.",
      archive: filename,
      sha256,
      bytes: (await lstat(zipPath)).size,
      pdfCount: manifest.papers.length,
      pdfBytes: manifest.papers.reduce((sum, paper) => sum + paper.pdfBytes, 0),
      fileCount: checksums.length + 1,
      format: "ZIP_STORED",
      timestamp: ARCHIVE_TIMESTAMP,
      verified: "All archive entries passed CRC and SHA-256 checks.",
    };
    await publishArchive(zipPath, join(output, filename), sha256);
    await writeReceipt(
      join(output, `${filename}.sha256`),
      `${sha256}  ${filename}\n`,
    );
    if (publicationBundle) {
      await writeReceipt(join(output, "README.txt"), metadata["README.txt"]);
    }
    const receiptPath = publicationBundle
      ? `${output}.verification.json`
      : join(output, "verification.json");
    await writeReceipt(receiptPath, json(receipt));
    return receipt;
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

async function main() {
  const { values } = parseArgs({
    options: {
      directory: { type: "string", default: "artifacts/real-demo" },
      output: { type: "string" },
      zenodo: { type: "boolean", default: false },
      help: { type: "boolean", default: false },
    },
    strict: true,
  });
  if (values.help) {
    process.stdout.write(
      `Graphite real-paper offline archive\n\n` +
        "Package the 60 reviewed publisher PDFs from DIRECTORY/pdfs. This\n" +
        "command is offline; Python 3 is required for deterministic ZIP output.\n" +
        "Only manifest-listed PDFs and generated attribution/rights metadata\n" +
        "enter the archive. No model caches or application data are included.\n\n" +
        "Flow:\n  1. Validate the manifest and matching rights review.\n" +
        "  2. Verify every PDF size, header, and SHA-256.\n" +
        "  3. Create and recheck a ZIP with fixed times, order, and permissions.\n\n" +
        "Usage: npm run demo:archive -- [--directory DIRECTORY] [--output OUTPUT]\n" +
        "       [--zenodo]\n" +
        "Prepare missing PDFs explicitly: npm run demo:prepare -- --download\n" +
        "Defaults: artifacts/real-demo and artifacts/demo-archive.\n" +
        "Use --zenodo for the three-file publication bundle: ZIP, archive checksum,\n" +
        "README.txt. Manifest, attribution and rights remain inside the ZIP. It defaults\n" +
        "to artifacts/demo-zenodo; its verification receipt stays alongside the\n" +
        "folder as OUTPUT.verification.json, outside the upload contents.\n" +
        "No Zenodo account is created and no upload or publication is performed.\n" +
        "Normal outputs: ZIP, archive checksum, and verification.json. Different\n" +
        "outputs are never overwritten; choose a new output folder after changes.\n",
    );
    return;
  }
  const manifest = JSON.parse(
    await readFile(join(ROOT, "demo/manifest.json"), "utf8"),
  );
  const rights = JSON.parse(
    await readFile(join(ROOT, "demo/rights-review.json"), "utf8"),
  );
  const receipt = await createDemoArchive(
    manifest,
    rights,
    resolve(values.directory),
    resolve(
      values.output ??
        (values.zenodo ? "artifacts/demo-zenodo" : "artifacts/demo-archive"),
    ),
    { publicationBundle: values.zenodo },
  );
  process.stdout.write(
    `${receipt.archive}: ${receipt.pdfCount} verified PDFs, ` +
      `${receipt.bytes} bytes\nSHA-256: ${receipt.sha256}\n`,
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
