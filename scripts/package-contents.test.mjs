import { Buffer } from "node:buffer";
import { mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";
import { assertSafeContents } from "./package-smoke.mjs";

it("accepts upstream binary strings while rejecting private package data and text", async () => {
  const root = await mkdtemp(join(tmpdir(), "graphite-contents-"));
  const path = join(root, "fixture");
  const entry = (relativePath, type = "file") => ({
    absolutePath: path,
    relativePath,
    type,
  });
  const token = "gh" + "p_" + "A".repeat(40);
  try {
    await writeFile(
      path,
      Buffer.from(`${homedir()}/upstream/source.cc\0${token}`),
    );
    await expect(
      assertSafeContents(root, [entry("usr/bin/graphite")]),
    ).resolves.toBeUndefined();
    for (const name of [
      "models/weights.gguf",
      "data/library.sqlite3",
      ".env",
    ]) {
      await expect(assertSafeContents(root, [entry(name)])).rejects.toThrow();
    }
    for (const content of [
      token,
      `${homedir()}/private`,
      "org.graphite.research.dev",
    ]) {
      await writeFile(path, content);
      await expect(
        assertSafeContents(root, [entry("settings.json")]),
      ).rejects.toThrow();
    }
    await writeFile(path, '{"product": "Graphite"}');
    await expect(
      assertSafeContents(root, [entry("settings.json")]),
    ).resolves.toBeUndefined();
    await rm(path);
    await symlink(join(homedir(), "private"), path);
    await expect(
      assertSafeContents(root, [entry("private-link", "link")]),
    ).rejects.toThrow("private build identity");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
