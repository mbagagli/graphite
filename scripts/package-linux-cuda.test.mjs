import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { afterEach, expect, test } from "vitest";
import { launcher } from "./package-linux-cuda.mjs";

const roots = [];
afterEach(async () => {
  await Promise.all(
    roots.splice(0).map((root) => rm(root, { recursive: true, force: true })),
  );
});

async function launch(dependencies, status = 0) {
  const root = await mkdtemp(resolve(tmpdir(), "graphite-cuda-launcher-"));
  roots.push(root);
  const bin = resolve(root, "bin");
  const runtime = resolve(root, "lib/graphite-cuda");
  await mkdir(bin);
  await mkdir(runtime, { recursive: true });
  await writeFile(resolve(bin, "graphite"), launcher(), { mode: 0o755 });
  await writeFile(
    resolve(bin, "ldd"),
    '#!/bin/sh\nprintf "%s\\n" "$TEST_DEPENDENCIES"\n' + `exit ${status}\n`,
    { mode: 0o755 },
  );
  for (const name of ["graphite", "graphite-cpu"]) {
    await writeFile(
      resolve(runtime, name),
      `#!/bin/sh\nprintf '${name}: %s\\n' "$1"\n`,
      { mode: 0o755 },
    );
  }
  return spawnSync(resolve(bin, "graphite"), ["argument with spaces"], {
    encoding: "utf8",
    env: { PATH: `${bin}:/usr/bin:/bin`, TEST_DEPENDENCIES: dependencies },
  });
}

test("launches the CUDA binary when its dependencies resolve", async () => {
  const result = await launch("libcuda.so.1 => /usr/lib/libcuda.so.1");
  expect(result.status).toBe(0);
  expect(result.stdout).toBe("graphite: argument with spaces\n");
});

test("uses CPU only when the driver library is absent", async () => {
  const result = await launch("libcuda.so.1 => not found");
  expect(result.status).toBe(0);
  expect(result.stdout).toBe("graphite-cpu: argument with spaces\n");
  expect(result.stderr).toContain("NVIDIA driver library unavailable");
});

test("rejects an incomplete runtime instead of hiding package damage", async () => {
  const result = await launch(
    "libcuda.so.1 => not found\nlibcublas.so.13 => not found",
  );
  expect(result.status).toBe(1);
  expect(result.stdout).toBe("");
  expect(result.stderr).toContain("package is incomplete");
});

test("does not launch when the dependency inspection fails", async () => {
  const result = await launch("", 2);
  expect(result.status).toBe(2);
  expect(result.stdout).toBe("");
});
