import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { URL } from "node:url";

import { describe, expect, it, vi } from "vitest";

import {
  assertEmptyLibrary,
  importDemo,
  isolatedZoteroOrigin,
} from "./import-real-demo.mjs";

const manifest = JSON.parse(
  await readFile(new URL("../demo/manifest.json", import.meta.url), "utf8"),
);
const ORIGIN = "http://127.0.0.1:23129";

function response(records, total = String(records.length)) {
  return {
    ok: true,
    status: 200,
    headers: { get: () => total },
    json: async () => records,
  };
}

function importedRecords() {
  return manifest.papers.flatMap((paper, index) => [
    {
      key: `PAPER${index}`,
      data: { itemType: "journalArticle", DOI: paper.doi },
    },
    {
      key: `PDF${index}`,
      data: {
        itemType: "attachment",
        parentItem: `PAPER${index}`,
        contentType: "application/pdf",
        linkMode: "imported_file",
      },
    },
  ]);
}

describe("isolated real-demo import", () => {
  it("requires an explicit nondefault loopback origin", () => {
    expect(isolatedZoteroOrigin(ORIGIN)).toBe(ORIGIN);
    for (const value of [
      "http://127.0.0.1:23119",
      "http://localhost:23129",
      "http://192.0.2.1:23129",
      "http://127.0.0.1",
      "https://127.0.0.1:23129",
      "http://user:secret@127.0.0.1:23129",
      `${ORIGIN}/api`,
      `${ORIGIN}/?target=elsewhere`,
      `${ORIGIN}/#fragment`,
    ]) {
      expect(() => isolatedZoteroOrigin(value)).toThrow("explicit");
    }
  });

  it("requires both an empty record list and an explicit zero total", async () => {
    for (const result of [
      response([{}]),
      response([], "1"),
      response([], null),
    ]) {
      await expect(
        assertEmptyLibrary(ORIGIN, async () => result),
      ).rejects.toThrow("not confirmed empty");
    }
    const fetcher = vi.fn(async () => response([]));
    await assertEmptyLibrary(ORIGIN, fetcher);
    expect(fetcher.mock.calls[0][0]).toContain("includeTrashed=1");
    expect(fetcher.mock.calls[0][1].redirect).toBe("error");
  });

  it("refuses a repeated import before preparation or mutation", async () => {
    const prepare = vi.fn();
    const fetcher = vi.fn(async () =>
      response(importedRecords().slice(0, 1), "60"),
    );
    await expect(
      importDemo(manifest, ".", ORIGIN, { fetcher, prepare }),
    ).rejects.toThrow("Import refused");
    expect(prepare).not.toHaveBeenCalled();
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(fetcher.mock.calls[0][1].method).toBeUndefined();
  });

  it("rechecks emptiness after verification and removes its lock on refusal", async () => {
    const directory = await mkdtemp(join(tmpdir(), "graphite-import-test-"));
    const prepare = vi.fn(async () => undefined);
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(response([]))
      .mockResolvedValueOnce(response([{}]));
    try {
      await expect(
        importDemo(manifest, directory, ORIGIN, { fetcher, prepare }),
      ).rejects.toThrow("Import refused");
      expect(prepare).toHaveBeenCalledOnce();
      expect(fetcher).toHaveBeenCalledTimes(2);
      await expect(
        readFile(join(directory, "import.lock")),
      ).rejects.toMatchObject({ code: "ENOENT" });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("imports once through the official API and verifies all article/PDF pairs", async () => {
    const directory = await mkdtemp(join(tmpdir(), "graphite-import-test-"));
    const records = importedRecords();
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(response([]))
      .mockResolvedValueOnce(response([]))
      .mockResolvedValueOnce(
        response(
          records.filter((item) => item.data.itemType === "journalArticle"),
        ),
      )
      .mockResolvedValueOnce(
        response(records.slice(0, 100), String(records.length)),
      )
      .mockResolvedValueOnce(
        response(records.slice(100), String(records.length)),
      );
    try {
      const receipt = await importDemo(manifest, directory, ORIGIN, {
        fetcher,
        prepare: async () => undefined,
      });
      expect(receipt.papers).toBe(60);
      expect(receipt.pdfAttachments).toBe(60);
      expect(receipt.emptyLibraryConfirmed).toBe(true);
      expect(fetcher.mock.calls[3][0]).toContain("start=0");
      expect(fetcher.mock.calls[4][0]).toContain("start=100");
      const [url, request] = fetcher.mock.calls[2];
      expect(url).toContain(`${ORIGIN}/connector/import?session=`);
      expect(request.method).toBe("POST");
      expect(request.body.match(/rdf:resource="file:/g)).toHaveLength(60);
      expect(
        fetcher.mock.calls.every(([, options]) => options.redirect === "error"),
      ).toBe(true);
      expect(
        JSON.parse(
          await readFile(join(directory, "import-verification.json"), "utf8"),
        ),
      ).toEqual(receipt);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("refuses an incomplete second verification page without retrying import", async () => {
    const directory = await mkdtemp(join(tmpdir(), "graphite-import-test-"));
    const records = importedRecords();
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(response([]))
      .mockResolvedValueOnce(response([]))
      .mockResolvedValueOnce(
        response(
          records.filter((item) => item.data.itemType === "journalArticle"),
        ),
      )
      .mockResolvedValueOnce(
        response(records.slice(0, 100), String(records.length)),
      )
      .mockResolvedValueOnce(response([], String(records.length)));
    try {
      await expect(
        importDemo(manifest, directory, ORIGIN, {
          fetcher,
          prepare: async () => undefined,
        }),
      ).rejects.toThrow("incomplete or changed");
      expect(
        fetcher.mock.calls.filter(([, options]) => options.method === "POST"),
      ).toHaveLength(1);
      await expect(
        readFile(join(directory, "import-verification.json")),
      ).rejects.toMatchObject({ code: "ENOENT" });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  it("does not retry a partial import", async () => {
    const directory = await mkdtemp(join(tmpdir(), "graphite-import-test-"));
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(response([]))
      .mockResolvedValueOnce(response([]))
      .mockResolvedValueOnce(response([{}]));
    try {
      await expect(
        importDemo(manifest, directory, ORIGIN, {
          fetcher,
          prepare: async () => undefined,
        }),
      ).rejects.toThrow("incomplete collection");
      expect(fetcher).toHaveBeenCalledTimes(3);
      await expect(
        readFile(join(directory, "import-verification.json")),
      ).rejects.toMatchObject({ code: "ENOENT" });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
