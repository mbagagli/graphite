import { once } from "node:events";
import { mkdtemp, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { expect, it } from "vitest";
import { createServer } from "vite";

import config from "../vite.config.ts";

it("watches frontend edits without walking native and generated trees", async () => {
  // Vite watches canonical paths, including macOS temporary directories.
  const root = await realpath(
    await mkdtemp(join(tmpdir(), "graphite-vite-watch-")),
  );
  const excluded = [
    ".cache/cargo-target/debug/incremental",
    "target/debug",
    "src-tauri/src",
    "tools/pdf-benchmark/assets",
    "artifacts/qa",
  ];
  let server;
  try {
    for (const directory of ["src", ...excluded]) {
      await mkdir(join(root, directory), { recursive: true });
      await writeFile(join(root, directory, "sentinel.txt"), "fixture");
    }
    const source = join(root, "src", "main.ts");
    await writeFile(source, "export const value = 1;\n");
    server = await createServer({
      ...config,
      plugins: [],
      configFile: false,
      root,
      server: { ...config.server, middlewareMode: true, hmr: false },
      optimizeDeps: { noDiscovery: true, include: [] },
    });
    await expect
      .poll(() => server.watcher.getWatched()[join(root, "src")])
      .toContain("main.ts");
    const changed = once(server.watcher, "change", {
      signal: globalThis.AbortSignal.timeout(3000),
    });
    await writeFile(source, "export const value = 2;\n");
    expect((await changed)[0]).toBe(source);
    const watched = Object.keys(server.watcher.getWatched());
    for (const directory of excluded) {
      expect(
        watched.some((path) => path.startsWith(join(root, directory))),
      ).toBe(false);
    }
  } finally {
    await server?.close();
    await rm(root, { recursive: true, force: true });
  }
});
