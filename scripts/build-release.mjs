import { spawnSync } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readdir, rm } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import {
  cargoRuntimeDirectory,
  cudaEnvironment,
  cudaLibraryDirectory,
  findCudaToolkit,
  releaseCudaArchitectures,
} from "./native-build.mjs";
import { packageLinuxCuda } from "./package-linux-cuda.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const bundles = { linux: "deb,appimage", darwin: "dmg" };
if (
  !bundles[process.platform] ||
  (process.platform === "linux" && process.arch !== "x64") ||
  (process.platform === "darwin" && process.arch !== "arm64")
) {
  throw new Error(
    "This release requires native Linux x64 or macOS Apple Silicon.",
  );
}
if (process.argv.length !== 2) {
  throw new Error(
    "Usage: npm run package:build (native unsigned GPU/CPU release build)",
  );
}
const toolkit = process.platform === "linux" ? findCudaToolkit(root) : null;
if (process.platform === "linux" && !toolkit) {
  throw new Error(
    "Linux releases require the pinned CUDA build toolkit. Run npm run cuda:prepare or set GRAPHITE_CUDA_TOOLKIT to an existing toolkit. End users need no toolkit.",
  );
}
const remaps = [
  [homedir(), "/build-home"],
  [root, "/graphite-source"],
];
const inheritedRustFlags = process.env.CARGO_ENCODED_RUSTFLAGS
  ? process.env.CARGO_ENCODED_RUSTFLAGS.split("\x1f")
  : (process.env.RUSTFLAGS ?? "").split(/\s+/).filter(Boolean);
const env = {
  ...process.env,
  CARGO_TARGET_DIR: resolve(root, ".cache/cargo-target"),
  LLAMA_BUILD_SHARED_LIBS: "0",
  CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "2",
  CMAKE_BUILD_PARALLEL_LEVEL: process.env.CMAKE_BUILD_PARALLEL_LEVEL ?? "2",
  CARGO_ENCODED_RUSTFLAGS: [
    ...inheritedRustFlags,
    ...remaps.map(
      ([source, target]) => `--remap-path-prefix=${source}=${target}`,
    ),
  ].join("\x1f"),
  CARGO_PROFILE_RELEASE_DEBUG: "0",
};
for (const variable of ["CFLAGS", "CXXFLAGS"]) {
  env[variable] = [
    process.env[variable] ?? "",
    ...remaps.map(
      ([source, target]) => `-ffile-prefix-map=${source}=${target}`,
    ),
  ]
    .join(" ")
    .trim();
}
if (process.platform === "darwin") {
  // Metal's .m files embed __FILE__ in assertions. C/C++ flags do not
  // apply to Objective-C; pass explicit cache entries through llama's build.
  for (const language of ["OBJC", "OBJCXX"]) {
    const variable = `CMAKE_${language}_FLAGS`;
    env[variable] = [
      process.env[variable] ?? process.env[`${language}FLAGS`] ?? "",
      ...remaps.map(
        ([source, target]) => `-ffile-prefix-map=${source}=${target}`,
      ),
    ]
      .join(" ")
      .trim();
  }
  env.MACOSX_DEPLOYMENT_TARGET = "11.0";
  // Use only the configured ad-hoc signature, never ambient Apple accounts.
  for (const name of Object.keys(env)) {
    if (name.startsWith("APPLE_")) delete env[name];
  }
}

function run(command, args, environment, capture = false, cwd = root) {
  const result = spawnSync(command, args, {
    cwd,
    env: environment,
    encoding: "utf8",
    stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0)
    throw new Error(`${command} failed with exit code ${result.status}.`);
  return result.stdout;
}

async function singleArtifact(directory, extension) {
  const names = (await readdir(directory)).filter((name) =>
    name.endsWith(extension),
  );
  if (names.length !== 1)
    throw new Error(`Expected one ${extension} template in ${directory}.`);
  return resolve(directory, names[0]);
}

run(
  process.execPath,
  [
    resolve(root, "node_modules/@tauri-apps/cli/tauri.js"),
    "build",
    "--ci",
    ...(process.platform === "linux" ? ["--no-sign"] : []),
    "--bundles",
    bundles[process.platform],
    ...(process.platform === "darwin" ? ["--features", "agent-metal"] : []),
    "--",
    "--locked",
  ],
  env,
);

if (process.platform === "linux") {
  const gpuEnv = cudaEnvironment(root, toolkit, releaseCudaArchitectures, env);
  gpuEnv.CMAKE_CUDA_FLAGS = [
    process.env.CMAKE_CUDA_FLAGS ?? "",
    ...remaps.map(
      ([source, target]) => `-Xcompiler=-ffile-prefix-map=${source}=${target}`,
    ),
  ]
    .join(" ")
    .trim();
  const messages = run(
    "cargo",
    [
      "build",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--release",
      "--locked",
      "--features",
      "agent-cuda,tauri/custom-protocol",
      "--bin",
      "graphite",
      "--message-format=json-render-diagnostics",
    ],
    gpuEnv,
    true,
  );
  const bundle = resolve(env.CARGO_TARGET_DIR, "release/bundle");
  const baseDeb = await singleArtifact(resolve(bundle, "deb"), ".deb");
  const baseAppImage = await singleArtifact(
    resolve(bundle, "appimage"),
    ".AppImage",
  );
  const scratch = await mkdtemp(resolve(root, ".cache/gpu-package-"));
  try {
    const toolRoot = resolve(scratch, "tool");
    await mkdir(toolRoot);
    run(
      resolve(
        env.CARGO_TARGET_DIR,
        ".tauri/linuxdeploy-plugin-appimage.AppImage",
      ),
      ["--appimage-extract"],
      env,
      true,
      toolRoot,
    );
    const packages = await packageLinuxCuda(
      {
        baseDeb,
        baseAppImage,
        cpuBinary: resolve(env.CARGO_TARGET_DIR, "release/graphite"),
        cudaBinary: resolve(gpuEnv.CARGO_TARGET_DIR, "release/graphite"),
        llamaLibraries: cargoRuntimeDirectory(messages),
        cudaLibraries: cudaLibraryDirectory(toolkit),
        appImageTool: resolve(toolRoot, "squashfs-root/usr/bin/appimagetool"),
        licenses: {
          "llama-LICENSE.txt": resolve(
            root,
            "src-tauri/licenses/llama-LICENSE.txt",
          ),
          "CUDA-LICENSE.txt": resolve(
            root,
            ".cache/cuda-notices/CUDA-LICENSE.txt",
          ),
          "cuBLAS-LICENSE.txt": resolve(
            root,
            ".cache/cuda-notices/cuBLAS-LICENSE.txt",
          ),
        },
        sourceRevision: process.env.GITHUB_SHA ?? null,
        versionSuffix: "",
      },
      resolve(scratch, "packages"),
    );
    await copyFile(packages[0], baseDeb);
    await copyFile(packages[1], baseAppImage);
    await copyFile(
      resolve(scratch, "packages/build-metadata.json"),
      resolve(bundle, "gpu-build-metadata.json"),
    );
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}
