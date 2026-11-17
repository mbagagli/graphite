import { randomUUID } from "node:crypto";
import { mkdir, open, readFile, unlink, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL, URL } from "node:url";
import { parseArgs } from "node:util";

import { createRdf, prepareDemo, validateManifest } from "./real-demo.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export function isolatedZoteroOrigin(value) {
  const url = new URL(value);
  if (
    url.protocol !== "http:" ||
    url.hostname !== "127.0.0.1" ||
    !url.port ||
    Number(url.port) < 1024 ||
    url.port === "23119" ||
    url.pathname !== "/" ||
    url.search ||
    url.hash ||
    url.username ||
    url.password
  ) {
    throw new Error(
      "Use an explicit http://127.0.0.1:PORT for an empty isolated Zotero profile; the normal port is forbidden.",
    );
  }
  return url.origin;
}

async function requestJson(origin, path, fetcher, options = {}) {
  let response;
  try {
    response = await fetcher(`${origin}${path}`, {
      ...options,
      redirect: "error",
      signal: globalThis.AbortSignal.timeout(120_000),
      headers: {
        "Zotero-API-Version": "3",
        ...options.headers,
      },
    });
  } catch (error) {
    throw new Error(
      "Could not reach isolated Zotero. Start the temporary profile and enable its local API on the chosen port. Redirects are refused.",
      { cause: error },
    );
  }
  if (!response.ok) {
    throw new Error(
      `Isolated Zotero request failed (${response.status}); check the temporary profile and local API settings.`,
    );
  }
  const records = await response.json();
  if (!Array.isArray(records)) {
    throw new Error("Isolated Zotero returned an unexpected record format.");
  }
  return { records, total: response.headers.get("Total-Results") };
}

export async function assertEmptyLibrary(origin, fetcher = globalThis.fetch) {
  const { records, total } = await requestJson(
    origin,
    "/api/users/0/items?limit=1&includeTrashed=1",
    fetcher,
  );
  if (records.length !== 0 || total !== "0") {
    throw new Error(
      "Import refused: the isolated Zotero library is not confirmed empty. Use a new empty profile; do not repeat an import.",
    );
  }
}

export async function importDemo(
  manifest,
  directory,
  zoteroUrl,
  { fetcher = globalThis.fetch, prepare = prepareDemo } = {},
) {
  validateManifest(manifest);
  const origin = isolatedZoteroOrigin(zoteroUrl);
  await assertEmptyLibrary(origin, fetcher);
  await mkdir(directory, { recursive: true });
  const lockPath = join(directory, "import.lock");
  const lock = await open(lockPath, "wx");
  try {
    await prepare(manifest, directory);
    const rdf = createRdf(manifest).replace(
      /rdf:resource="pdfs\/([^"/]+)"/g,
      (_match, filename) =>
        `rdf:resource="${pathToFileURL(join(resolve(directory), "pdfs", filename)).href.replaceAll("&", "&amp;")}"`,
    );
    // The preparation can take time; recheck immediately before mutation.
    await assertEmptyLibrary(origin, fetcher);
    const imported = await requestJson(
      origin,
      `/connector/import?session=${randomUUID()}`,
      fetcher,
      {
        method: "POST",
        headers: {
          "Content-Type": "application/xml",
          "X-Zotero-Connector-API-Version": "3",
        },
        body: rdf,
      },
    );
    if (imported.records.length !== manifest.papers.length) {
      throw new Error(
        "Import returned an incomplete collection. Inspect the disposable profile; automatic retry is disabled.",
      );
    }
    const expectedRecords = manifest.papers.length * 2;
    const records = [];
    for (let start = 0; start < expectedRecords; start += 100) {
      const page = await requestJson(
        origin,
        `/api/users/0/items?limit=100&start=${start}&includeTrashed=1`,
        fetcher,
      );
      if (
        page.total !== String(expectedRecords) ||
        page.records.length !== Math.min(100, expectedRecords - start)
      ) {
        throw new Error(
          "Import verification failed: incomplete or changed article/PDF pages. Inspect the disposable profile; automatic retry is disabled.",
        );
      }
      records.push(...page.records);
    }
    const articles = records.filter(
      (item) => item.data?.itemType === "journalArticle",
    );
    const attachments = records.filter(
      (item) => item.data?.itemType === "attachment",
    );
    const expected = new Set(manifest.papers.map((paper) => paper.doi));
    const keys = new Set(articles.map((item) => item.key));
    const parentKeys = new Set(attachments.map((item) => item.data.parentItem));
    if (
      records.length !== expectedRecords ||
      articles.length !== manifest.papers.length ||
      attachments.length !== manifest.papers.length ||
      new Set(articles.map((item) => item.data.DOI)).size !==
        manifest.papers.length ||
      !articles.every((item) => expected.has(item.data.DOI)) ||
      keys.size !== manifest.papers.length ||
      parentKeys.size !== manifest.papers.length ||
      !attachments.every(
        (item) =>
          keys.has(item.data.parentItem) &&
          item.data.contentType === "application/pdf" &&
          item.data.linkMode === "imported_file",
      )
    ) {
      throw new Error(
        "Import verification failed. Inspect the disposable profile; automatic retry is disabled.",
      );
    }
    const receipt = {
      manifestId: manifest.id,
      papers: articles.length,
      pdfAttachments: attachments.length,
      emptyLibraryConfirmed: true,
      records: articles.map((item) => ({ key: item.key, doi: item.data.DOI })),
    };
    await writeFile(
      join(directory, "import-verification.json"),
      JSON.stringify(receipt, null, 2) + "\n",
    );
    return receipt;
  } finally {
    await lock.close();
    await unlink(lockPath);
  }
}

async function main() {
  const { values } = parseArgs({
    options: {
      "zotero-url": { type: "string" },
      directory: { type: "string", default: "artifacts/real-demo" },
      help: { type: "boolean", default: false },
    },
    strict: true,
  });
  if (values.help) {
    process.stdout.write(`Graphite isolated real-paper demo import

Import the verified 60-paper collection into an already running, empty,
isolated Zotero profile through its official connector API. Input is the
prepared PDF directory and an explicit nondefault loopback server URL.
Output is 60 articles, 60 copied PDFs, and import-verification.json.

Flow:
  1. Refuse normal ports, redirects, and nonempty libraries (including trash).
  2. Recheck all original PDF bytes, metadata, and extractable text offline.
  3. Reconfirm the library is empty, import once, and verify all 60 records.

Usage: node scripts/import-real-demo.mjs --zotero-url URL [--directory DIR]
Example: npm run demo:import -- --zotero-url http://127.0.0.1:23129

Create a disposable profile with a separate data directory, account sync
disabled, and local API enabled on the specified nondefault port. Close
other clients of that profile during import. Never point this command at
a personal library. Failed or partial imports are not automatically retried;
use a new empty disposable profile before trying again.
`);
    return;
  }
  if (!values["zotero-url"]) {
    throw new Error("An explicit --zotero-url is required; see --help.");
  }
  const manifest = JSON.parse(
    await readFile(join(ROOT, "demo/manifest.json"), "utf8"),
  );
  await importDemo(manifest, resolve(values.directory), values["zotero-url"]);
  process.stdout.write("Imported and verified 60 papers with 60 PDFs.\n");
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
