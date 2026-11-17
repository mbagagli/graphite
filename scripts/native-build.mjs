import { spawnSync } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
import { delimiter, resolve } from "node:path";
import process from "node:process";

export const releaseCudaArchitectures =
  "75-virtual;80-virtual;86-real;89-real;90-virtual;120a-real;121a-real";

/** Find an existing toolkit without installing or activating an environment. */
export function findCudaToolkit(root, env = process.env) {
  const configured =
    env.GRAPHITE_CUDA_TOOLKIT ?? env.CUDA_PATH ?? env.CUDA_HOME;
  if (configured && !existsSync(resolve(configured, "bin/nvcc"))) {
    throw new Error(
      "The configured CUDA toolkit has no bin/nvcc. Correct GRAPHITE_CUDA_TOOLKIT or remove the override.",
    );
  }
  const candidates = [
    configured,
    resolve(root, ".cache/cuda-toolkit"),
    ...(env.PATH ?? "").split(delimiter).map((path) => resolve(path, "..")),
    "/usr/local/cuda",
  ].filter(Boolean);
  for (const candidate of candidates) {
    if (existsSync(resolve(candidate, "bin/nvcc")))
      return realpathSync(candidate);
  }
  return null;
}

/** Locate only CUDA runtime libraries, never a whole Conda environment. */
export function cudaLibraryDirectory(toolkit) {
  for (const suffix of ["targets/x86_64-linux/lib", "lib64", "lib"]) {
    const path = resolve(toolkit, suffix);
    if (existsSync(resolve(path, "libcudart.so"))) return path;
  }
  throw new Error("The existing CUDA toolkit is missing libcudart.so.");
}

/** Read supported local device capabilities without changing the driver. */
export function localCudaArchitectures() {
  const result = spawnSync(
    "nvidia-smi",
    ["--query-gpu=compute_cap", "--format=csv,noheader"],
    { encoding: "utf8", timeout: 5000 },
  );
  if (result.status !== 0) return null;
  const values = [
    ...new Set(
      result.stdout
        .trim()
        .split(/\r?\n/)
        .map((value) => value.trim())
        .filter((value) => /^\d+\.\d+$/.test(value))
        .map((value) => value.replace(".", "")),
    ),
  ];
  return values.length ? values.join(";") : null;
}

/** Configure child processes only; leave system and shell settings untouched. */
export function cudaEnvironment(
  root,
  toolkit,
  architectures,
  env = process.env,
) {
  const runtime = cudaLibraryDirectory(toolkit);
  return {
    ...env,
    CUDA_PATH: toolkit,
    CMAKE_CUDA_COMPILER: resolve(toolkit, "bin/nvcc"),
    CMAKE_CUDA_HOST_COMPILER: env.CMAKE_CUDA_HOST_COMPILER ?? "/usr/bin/g++",
    CMAKE_CUDA_ARCHITECTURES: env.CMAKE_CUDA_ARCHITECTURES ?? architectures,
    CMAKE_BUILD_PARALLEL_LEVEL: env.CMAKE_BUILD_PARALLEL_LEVEL ?? "2",
    CARGO_BUILD_JOBS: env.CARGO_BUILD_JOBS ?? "2",
    CARGO_TARGET_DIR: resolve(root, ".cache/cargo-cuda"),
    LLAMA_BUILD_SHARED_LIBS: "1",
    LD_LIBRARY_PATH: [runtime, env.LD_LIBRARY_PATH]
      .filter(Boolean)
      .join(delimiter),
  };
}

/** Keep development portable while release builds require their GPU toolchain. */
export function developmentPlan({
  root,
  platform,
  env,
  toolkit,
  architectures,
  forceCuda = false,
}) {
  const args = ["dev", "--config", "src-tauri/tauri.dev.conf.json"];
  if (forceCuda && platform !== "linux") {
    throw new Error("Explicit CUDA development is supported on Linux only.");
  }
  if (platform === "darwin") {
    return {
      args: [...args, "--features", "agent-metal"],
      env,
      message: "Metal build: Auto uses an available GPU, with CPU fallback.",
    };
  }
  if (platform === "linux" && toolkit && architectures) {
    return {
      args: [...args, "--features", "agent-cuda"],
      env: cudaEnvironment(root, toolkit, architectures, env),
      message:
        "CUDA build: reusing the existing toolkit. Auto prefers GPU, with CPU fallback.",
    };
  }
  if (forceCuda)
    throw new Error(
      "CUDA development needs an existing toolkit and an available NVIDIA driver. Normal desktop:dev can use CPU instead.",
    );
  return {
    args,
    env,
    message:
      "CPU build: no usable local CUDA toolkit/GPU pair was detected. Inference will use CPU.",
  };
}

/** Resolve the exact shared runtime reported by this Cargo build. */
export function cargoRuntimeDirectory(stdout) {
  const builds = stdout
    .split("\n")
    .flatMap((line) => {
      try {
        return [JSON.parse(line)];
      } catch {
        return [];
      }
    })
    .filter(
      (entry) =>
        entry.reason === "build-script-executed" &&
        entry.package_id.includes("llama-cpp-sys-2"),
    );
  const output = builds.at(-1)?.out_dir;
  if (!output)
    throw new Error("Cargo did not report the built inference runtime.");
  for (const suffix of ["lib", "lib64"]) {
    const path = resolve(output, suffix);
    if (existsSync(resolve(path, "libggml-cuda.so.0"))) return path;
  }
  throw new Error(
    "The CUDA build did not install its shared inference runtime.",
  );
}
