import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import {
  copyFile,
  mkdir,
  readFile,
  rename,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { basename, dirname, resolve } from "node:path";
import { Readable, Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/** Verify every archive before Conda is allowed to extract or execute it. */
export async function verifyArchive(path, expected) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  if (hash.digest("hex") !== expected)
    throw new Error(`CUDA archive checksum failed: ${basename(path)}`);
}

async function main() {
  if (
    process.argv.slice(2).join(" ") !== "--download" ||
    process.platform !== "linux" ||
    process.arch !== "x64"
  ) {
    throw new Error(
      "Usage: node scripts/prepare-cuda.mjs --download (Linux x64). Downloads the pinned build toolkit into .cache only; never installs drivers or system libraries.",
    );
  }
  const manifest = JSON.parse(
    await readFile(resolve(root, "scripts/cuda-toolchain.lock.json"), "utf8"),
  );
  const cache = resolve(root, ".cache/cuda-downloads");
  const packages = resolve(root, ".cache/cuda-packages");
  const toolkit = resolve(root, ".cache/cuda-toolkit");
  const notices = resolve(root, ".cache/cuda-notices");
  try {
    await stat(toolkit);
    throw new Error(
      "The project already has a CUDA toolkit. Reuse it; this command will not overwrite it.",
    );
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  await mkdir(cache, { recursive: true });
  const explicit = ["@EXPLICIT"];
  for (const [index, entry] of manifest.packages.entries()) {
    const url = new globalThis.URL(entry.url);
    if (
      url.origin !== "https://conda.anaconda.org" ||
      !url.pathname.startsWith("/conda-forge/") ||
      !/^[a-f0-9]{64}$/.test(entry.sha256) ||
      !/^[a-f0-9]{32}$/.test(entry.md5)
    ) {
      throw new Error("Invalid pinned CUDA package entry.");
    }
    const path = resolve(cache, basename(url.pathname));
    try {
      await verifyArchive(path, entry.sha256);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      process.stdout.write(
        `CUDA toolkit ${index + 1}/${manifest.packages.length}: ${basename(path)}\n`,
      );
      const response = await globalThis.fetch(url, {
        redirect: "error",
        signal: globalThis.AbortSignal.timeout(300_000),
      });
      if (!response.ok || !response.body)
        throw new Error(
          `CUDA package download failed: HTTP ${response.status}`,
          { cause: error },
        );
      let bytes = 0;
      const limit = new Transform({
        transform(chunk, encoding, done) {
          bytes += chunk.length;
          done(
            bytes > 2 * 1024 ** 3
              ? new Error("CUDA package exceeds the download limit.")
              : null,
            chunk,
          );
        },
      });
      const temporary = `${path}.part`;
      try {
        await pipeline(
          Readable.fromWeb(response.body),
          limit,
          createWriteStream(temporary, { flags: "wx" }),
        );
        await verifyArchive(temporary, entry.sha256);
        await rename(temporary, path);
      } finally {
        await rm(temporary, { force: true });
      }
    }
    explicit.push(`${pathToFileURL(path).href}#${entry.md5}`);
  }
  const lock = resolve(cache, "verified-local-packages.txt");
  await writeFile(lock, `${explicit.join("\n")}\n`);
  const conda = process.env.CONDA
    ? resolve(process.env.CONDA, "bin/conda")
    : "conda";
  const result = spawnSync(
    conda,
    [
      "create",
      "--yes",
      "--offline",
      "--no-default-packages",
      "--prefix",
      toolkit,
      "--file",
      lock,
    ],
    {
      stdio: "inherit",
      env: {
        ...process.env,
        CONDARC: "/dev/null",
        CONDA_PKGS_DIRS: packages,
        CONDA_NO_PLUGINS: "true",
        CONDA_SOLVER: "classic",
        CONDA_AUTO_UPDATE_CONDA: "false",
      },
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0)
    throw new Error("The isolated CUDA toolkit could not be prepared.");
  await mkdir(notices, { recursive: true });
  for (const [packageName, license] of [
    ["cuda-cudart-13.4.92-hd2095e1_1", "CUDA-LICENSE.txt"],
    ["libcublas-13.8.0.4-h393e864_1", "cuBLAS-LICENSE.txt"],
  ]) {
    await copyFile(
      resolve(packages, packageName, "info/licenses/LICENSE"),
      resolve(notices, license),
    );
  }
  process.stdout.write(
    "Pinned CUDA build toolkit prepared in the project cache. No driver or system library installed.\n",
  );
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
