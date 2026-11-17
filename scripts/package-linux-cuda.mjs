import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  chmod,
  copyFile,
  cp,
  mkdir,
  readFile,
  writeFile,
} from "node:fs/promises";
import { resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

export const llamaLibraryNames = [
  "libllama.so.0",
  "libggml.so.0",
  "libggml-base.so.0",
  "libggml-cpu.so.0",
  "libggml-cuda.so.0",
];
export const cudaLibraryNames = [
  "libcudart.so.13",
  "libcublas.so.13",
  "libcublasLt.so.13",
];

const usage = `Build an isolated Linux x64 CUDA candidate from local artifacts.
Usage: node scripts/package-linux-cuda.mjs INPUTS.json OUTPUT_DIRECTORY
Inputs: baseDeb, baseAppImage, cudaBinary, cpuBinary, llamaLibraries,
  cudaLibraries, appImageTool, licenses (mapping filename to local path),
  sourceRevision. All paths must be absolute. No downloads or installation.
Flow: extract templates; replace executables; bundle runtimes and licenses;
  build unsigned DEB/AppImage; write checksums and input identities.
Use matching current CPU/CUDA binaries with embedded release frontend assets.
The output directory must not exist. Test only in disposable profiles.
`;

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    ...options,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} failed: ${result.stderr}\n${result.stdout}`);
  }
  return result.stdout;
}

async function digest(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

export function launcher() {
  return `#!/bin/sh
set -eu
prefix=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
runtime="$prefix/lib/graphite-cuda"
# Resolve private inference libraries before AppImage desktop libraries.
export LD_LIBRARY_PATH="$runtime\${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
dependencies=$(ldd "$runtime/graphite")
missing=$(printf '%s\\n' "$dependencies" | grep 'not found' || true)
if [ -n "$missing" ]; then
    other=$(printf '%s\\n' "$missing" | grep -v 'libcuda.so.1 ' || true)
    if [ -n "$other" ]; then
        printf 'Graphite package is incomplete: %s\\n' "$other" >&2
        exit 1
    fi
    printf 'Graphite: NVIDIA driver library unavailable; using CPU.\\n' >&2
    exec "$runtime/graphite-cpu" "$@"
fi
exec "$runtime/graphite" "$@"
`;
}

/** Assemble matching GPU and CPU runtimes without installing the package. */
export async function packageLinuxCuda(input, output) {
  for (const key of [
    "baseDeb",
    "baseAppImage",
    "cudaBinary",
    "cpuBinary",
    "llamaLibraries",
    "cudaLibraries",
    "appImageTool",
  ]) {
    if (!input[key]?.startsWith("/"))
      throw new Error(`${key} must be absolute`);
  }
  if (
    input.sourceRevision !== null &&
    !/^[a-f0-9]{40}$/.test(input.sourceRevision)
  ) {
    throw new Error(
      "sourceRevision must be the full source commit, or null for a local build",
    );
  }
  for (const name of [
    "llama-LICENSE.txt",
    "CUDA-LICENSE.txt",
    "cuBLAS-LICENSE.txt",
  ]) {
    if (!input.licenses?.[name]?.startsWith("/")) {
      throw new Error(`Missing local license: ${name}`);
    }
  }
  for (const key of ["cpuBinary", "cudaBinary"]) {
    const header = run("readelf", ["-h", input[key]]);
    if (!/Machine:\s+Advanced Micro Devices X86-64/.test(header)) {
      throw new Error(`${key} must be a Linux x64 executable`);
    }
    const dynamic = run("readelf", ["-d", input[key]]);
    if (/\((?:RPATH|RUNPATH)\)/.test(dynamic)) {
      throw new Error(`${key} must not retain a build-specific loader path`);
    }
    if (key === "cpuBinary" && /lib(?:ggml|llama|cuda)/.test(dynamic)) {
      throw new Error("CPU companion must use its application-linked runtime");
    }
    if (key === "cudaBinary" && !dynamic.includes("libllama.so.0")) {
      throw new Error("CUDA executable must use the matching shared runtime");
    }
  }
  // Preserve previous candidates, including failed-attempt evidence.
  await mkdir(output);
  const deb = resolve(output, "deb-root");
  const extraction = resolve(output, "appimage-root");
  await mkdir(extraction);
  run("dpkg-deb", ["--raw-extract", input.baseDeb, deb]);
  run(input.baseAppImage, ["--appimage-extract"], { cwd: extraction });
  const appdir = resolve(extraction, "squashfs-root");
  const libraries = [
    ...llamaLibraryNames.map((name) => [
      name,
      resolve(input.llamaLibraries, name),
    ]),
    ...cudaLibraryNames.map((name) => [
      name,
      resolve(input.cudaLibraries, name),
    ]),
  ];
  const runtime = resolve(deb, "usr/lib/graphite-cuda");
  await mkdir(runtime, { recursive: true });
  await copyFile(input.cudaBinary, resolve(runtime, "graphite"));
  await copyFile(input.cpuBinary, resolve(runtime, "graphite-cpu"));
  const identities = [];
  for (const [name, path] of libraries) {
    const dynamic = run("readelf", ["-d", path]);
    if (
      dynamic
        .split("\n")
        .some(
          (line) =>
            /\((?:RPATH|RUNPATH)\)/.test(line) && !line.endsWith("[$ORIGIN]"),
        )
    ) {
      throw new Error(`${name} has a build-specific loader path`);
    }
    await copyFile(path, resolve(runtime, name));
    identities.push({ file: name, sha256: await digest(path) });
  }
  for (const root of [deb, appdir]) {
    if (root !== deb) {
      await cp(runtime, resolve(root, "usr/lib/graphite-cuda"), {
        recursive: true,
      });
    }
    const path = resolve(root, "usr/bin/graphite");
    await writeFile(path, launcher());
    await chmod(path, 0o755);
    const licenses = resolve(root, "usr/share/graphite/licenses");
    await mkdir(licenses, { recursive: true });
    for (const [name, source] of Object.entries(input.licenses)) {
      if (!/^[A-Za-z0-9.-]+$/.test(name))
        throw new Error("Unsafe license name");
      await copyFile(source, resolve(licenses, name));
    }
  }
  const controlPath = resolve(deb, "DEBIAN/control");
  let control = await readFile(controlPath, "utf8");
  const suffix = input.versionSuffix ?? "+cuda1";
  if (!["", "+cuda1"].includes(suffix))
    throw new Error("Unsupported package version suffix.");
  control = control.replace(/^Version: (.+)$/m, `Version: $1${suffix}`);
  const size = run("du", ["-sk", resolve(deb, "usr")]).split(/\s/)[0];
  control = control.replace(/^Installed-Size: .+$/m, `Installed-Size: ${size}`);
  await writeFile(controlPath, control);
  const version = control.match(/^Version: (.+)$/m)[1];
  const debName = `Graphite_${version}_amd64.deb`;
  const imageName = `Graphite_${version}_amd64.AppImage`;
  run("dpkg-deb", [
    "--root-owner-group",
    "-Zxz",
    "--build",
    deb,
    resolve(output, debName),
  ]);
  run(
    input.appImageTool,
    ["--no-appstream", appdir, resolve(output, imageName)],
    {
      env: { ...process.env, ARCH: "x86_64" },
    },
  );
  const packages = [];
  for (const name of [debName, imageName]) {
    packages.push({ file: name, sha256: await digest(resolve(output, name)) });
  }
  const inputs = {};
  for (const key of ["baseDeb", "baseAppImage", "cudaBinary", "cpuBinary"]) {
    inputs[key] = await digest(input[key]);
  }
  await writeFile(
    resolve(output, "SHA256SUMS.txt"),
    packages.map(({ file, sha256 }) => `${sha256}  ${file}\n`).join(""),
  );
  await writeFile(
    resolve(output, "build-metadata.json"),
    JSON.stringify(
      {
        schemaVersion: 1,
        variant: "linux-x64-cuda",
        version,
        signed: false,
        publicRelease: false,
        sourceRevision: input.sourceRevision,
        inputs,
        libraries: identities,
        packages,
        driverBundled: false,
        cpuCompanion: true,
      },
      null,
      2,
    ) + "\n",
  );
  process.stdout.write(
    `Built ${packages.length} GPU-enabled packages with CPU fallback.\n`,
  );
  return packages.map(({ file }) => resolve(output, file));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 4) throw new Error(usage);
  await packageLinuxCuda(
    JSON.parse(await readFile(process.argv[2], "utf8")),
    resolve(process.argv[3]),
  );
}
