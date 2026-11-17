import { Buffer } from "node:buffer";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  rm,
  symlink,
  utimes,
  writeFile,
} from "node:fs/promises";
import process from "node:process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { URL } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import {
  archiveMetadata,
  createDemoArchive,
  validateRightsReview,
} from "./demo-archive.mjs";

const manifest = JSON.parse(
  await readFile(new URL("../demo/manifest.json", import.meta.url), "utf8"),
);
const rights = JSON.parse(
  await readFile(
    new URL("../demo/rights-review.json", import.meta.url),
    "utf8",
  ),
);
const temporaryDirectories = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map((directory) => rm(directory, { recursive: true, force: true })),
  );
});

async function fixture() {
  const directory = await mkdtemp(
    join(tmpdir(), "graphite-demo-archive-test-"),
  );
  temporaryDirectories.push(directory);
  const input = join(directory, "input");
  await mkdir(join(input, "pdfs"), { recursive: true });
  const sample = globalThis.structuredClone(manifest);
  const review = globalThis.structuredClone(rights);
  for (const paper of sample.papers) {
    const bytes = Buffer.from(`%PDF-1.7\n${paper.id}\n%%EOF\n`);
    paper.pdfBytes = bytes.length;
    paper.sha256 = createHash("sha256").update(bytes).digest("hex");
    review.papers.find((item) => item.id === paper.id).pdfSha256 = paper.sha256;
    await writeFile(join(input, "pdfs", `${paper.id}.pdf`), bytes);
  }
  return { directory, input, sample, review };
}

describe("reviewed offline demo archive", () => {
  it("requires the current license review for every exact PDF", () => {
    expect(validateRightsReview(manifest, rights)).toBe(rights);
    const changed = globalThis.structuredClone(rights);
    changed.papers[0].pdfSha256 = "0".repeat(64);
    expect(() => validateRightsReview(manifest, changed)).toThrow(
      "stale rights",
    );
    changed.papers = rights.papers.slice(1);
    expect(() => validateRightsReview(manifest, changed)).toThrow(
      "complete rights",
    );
    expect(archiveMetadata(manifest, rights)["ATTRIBUTION.txt"]).toContain(
      "Federal Office of Topography swisstopo",
    );
  });

  it("produces identical bytes, fixed metadata, and only reviewed contents", async () => {
    const { directory, input, sample, review } = await fixture();
    const first = await createDemoArchive(
      sample,
      review,
      input,
      join(directory, "one"),
    );
    for (const paper of sample.papers) {
      await utimes(join(input, "pdfs", `${paper.id}.pdf`), 12345, 12345);
    }
    await writeFile(join(input, "extra-local-note.txt"), "not archive input");
    const second = await createDemoArchive(
      sample,
      review,
      input,
      join(directory, "two"),
    );
    expect(second).toEqual(first);
    const repeated = await createDemoArchive(
      sample,
      review,
      input,
      join(directory, "one"),
    );
    expect(repeated).toEqual(first);
    const contents = JSON.parse(
      execFileSync(
        "python3",
        [
          "-c",
          `
import json, sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as archive:
    print(json.dumps([{"name": i.filename, "time": i.date_time,
        "mode": i.external_attr >> 16} for i in archive.infolist()]))
`,
          join(directory, "one", first.archive),
        ],
        { encoding: "utf8" },
      ),
    );
    expect(contents).toHaveLength(manifest.papers.length + 5);
    expect(contents.filter((item) => item.name.endsWith(".pdf"))).toHaveLength(
      60,
    );
    expect(contents.map((item) => item.name)).toEqual(
      contents.map((item) => item.name).sort(),
    );
    for (const item of contents) {
      expect(item.name).toMatch(/^graphite-demo-seismology-ml-v2\//);
      expect(item.name).not.toContain("..");
      expect(item.time).toEqual([1980, 1, 1, 0, 0, 0]);
      expect(item.mode).toBe(0o100644);
    }
    expect(JSON.stringify(first)).not.toContain(directory);
  });

  it("creates only three Zenodo uploads and preserves the external receipt", async () => {
    const { directory, input, sample, review } = await fixture();
    const output = join(directory, "zenodo");
    const receipt = await createDemoArchive(sample, review, input, output, {
      publicationBundle: true,
    });
    expect((await readdir(output)).sort()).toEqual(
      ["README.txt", receipt.archive, `${receipt.archive}.sha256`].sort(),
    );
    expect(
      JSON.parse(await readFile(`${output}.verification.json`, "utf8")),
    ).toEqual(receipt);
    const metadata = archiveMetadata(sample, review);
    expect(await readFile(join(output, "README.txt"), "utf8")).toBe(
      metadata["README.txt"],
    );
    expect(metadata["README.txt"]).toContain("Prepare library for AI");
    expect(metadata["README.txt"]).toContain("Already downloaded?");
    expect(metadata["README.txt"]).toContain("Do not unpack it first");
    expect(metadata["README.txt"]).toContain(
      "No model is downloaded automatically",
    );
    expect(
      await createDemoArchive(sample, review, input, output, {
        publicationBundle: true,
      }),
    ).toEqual(receipt);
    await writeFile(join(output, "README.txt"), "existing owner text");
    await expect(
      createDemoArchive(sample, review, input, output, {
        publicationBundle: true,
      }),
    ).rejects.toThrow("different content");
    expect(await readFile(join(output, "README.txt"), "utf8")).toBe(
      "existing owner text",
    );
  });

  it("rejects missing, corrupt, and symbolic-link PDFs before producing an archive", async () => {
    const { directory, input, sample, review } = await fixture();
    const filename = join(input, "pdfs", `${sample.papers[0].id}.pdf`);
    const bytes = await readFile(filename);
    await rm(filename);
    const output = join(directory, "out");
    await expect(
      createDemoArchive(sample, review, input, output),
    ).rejects.toThrow("Missing PDF");
    bytes[bytes.length - 2] ^= 1;
    await writeFile(filename, bytes);
    await expect(
      createDemoArchive(sample, review, input, output),
    ).rejects.toThrow("checksum mismatch");
    await rm(filename);
    await symlink(join(input, "pdfs", `${sample.papers[1].id}.pdf`), filename);
    await expect(
      createDemoArchive(sample, review, input, output),
    ).rejects.toThrow("Invalid local PDF");
  });

  it("refuses to overwrite a different existing output", async () => {
    const { directory, input, sample, review } = await fixture();
    const output = join(directory, "out");
    await mkdir(output);
    const filename = join(output, `graphite-demo-${sample.id}-r1.zip`);
    await writeFile(filename, "existing user file");
    await expect(
      createDemoArchive(sample, review, input, output),
    ).rejects.toThrow("different bytes");
    expect(await readFile(filename, "utf8")).toBe("existing user file");
  });

  it("explains offline behavior and required inputs in CLI help", () => {
    const help = execFileSync(
      process.execPath,
      ["scripts/demo-archive.mjs", "--help"],
      { encoding: "utf8" },
    );
    expect(help).toContain("command is offline");
    expect(help).toContain("Python 3");
    expect(help).toContain("--directory DIRECTORY");
    expect(help).toContain("--zenodo");
    expect(help).toContain("no upload or publication");
  });
});
