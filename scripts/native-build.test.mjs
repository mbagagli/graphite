import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { afterEach, expect, test } from "vitest";
import {
  cargoRuntimeDirectory,
  cudaEnvironment,
  developmentPlan,
  findCudaToolkit,
} from "./native-build.mjs";
import { verifyArchive } from "./prepare-cuda.mjs";

const roots = [];
afterEach(async () => {
  await Promise.all(
    roots.splice(0).map((root) => rm(root, { recursive: true, force: true })),
  );
});
async function fixture() {
  const root = await mkdtemp(resolve(tmpdir(), "graphite-build-"));
  roots.push(root);
  const toolkit = resolve(root, ".cache/cuda-toolkit");
  await mkdir(resolve(toolkit, "bin"), { recursive: true });
  await mkdir(resolve(toolkit, "targets/x86_64-linux/lib"), {
    recursive: true,
  });
  await writeFile(resolve(toolkit, "bin/nvcc"), "fixture");
  await writeFile(
    resolve(toolkit, "targets/x86_64-linux/lib/libcudart.so"),
    "fixture",
  );
  return { root, toolkit };
}

test("normal Linux development uses an existing CUDA toolkit and driver", async () => {
  const { root, toolkit } = await fixture();
  const env = { PATH: "/usr/bin", LD_LIBRARY_PATH: "/existing-runtime" };
  expect(findCudaToolkit(root, env)).toBe(toolkit);
  const plan = developmentPlan({
    root,
    platform: "linux",
    env,
    toolkit,
    architectures: "89",
  });
  expect(plan.args).toEqual([
    "dev",
    "--config",
    "src-tauri/tauri.dev.conf.json",
    "--features",
    "agent-cuda",
  ]);
  expect(plan.env.CMAKE_CUDA_COMPILER).toBe(resolve(toolkit, "bin/nvcc"));
  expect(plan.env.LD_LIBRARY_PATH).toBe(
    `${toolkit}/targets/x86_64-linux/lib:/existing-runtime`,
  );
  expect(plan.env.LLAMA_BUILD_SHARED_LIBS).toBe("1");
  expect(plan.env.CMAKE_CUDA_ARCHITECTURES).toBe("89");
  expect(env).toEqual({
    PATH: "/usr/bin",
    LD_LIBRARY_PATH: "/existing-runtime",
  });
});

test.each([
  { toolkit: null, architectures: "89" },
  { toolkit: "/existing/toolkit", architectures: null },
])(
  "Linux development uses CPU when no usable toolkit/device pair exists: %j",
  (hardware) => {
    const plan = developmentPlan({
      root: "/fixture",
      platform: "linux",
      env: {},
      ...hardware,
    });
    expect(plan.args).not.toContain("agent-cuda");
    expect(plan.message).toContain("CPU build");
    expect(() =>
      developmentPlan({
        root: "/fixture",
        platform: "linux",
        env: {},
        forceCuda: true,
        ...hardware,
      }),
    ).toThrow("existing toolkit");
  },
);

test("Apple development explicitly enables Metal", () => {
  expect(
    developmentPlan({ root: "/fixture", platform: "darwin", env: {} }).args,
  ).toContain("agent-metal");
});

test("invalid explicit toolkit paths are not silently replaced", async () => {
  const { root } = await fixture();
  expect(() =>
    findCudaToolkit(root, { GRAPHITE_CUDA_TOOLKIT: resolve(root, "missing") }),
  ).toThrow("no bin/nvcc");
});

test("explicit architecture choices survive the shared CUDA environment", async () => {
  const { root, toolkit } = await fixture();
  expect(
    cudaEnvironment(root, toolkit, "89", { CMAKE_CUDA_ARCHITECTURES: "75;89" })
      .CMAKE_CUDA_ARCHITECTURES,
  ).toBe("75;89");
});

test("packaging resolves the runtime from the current Cargo build", async () => {
  const { root } = await fixture();
  await mkdir(resolve(root, "lib"));
  await writeFile(resolve(root, "lib/libggml-cuda.so.0"), "fixture");
  const message = JSON.stringify({
    reason: "build-script-executed",
    package_id: "registry#llama-cpp-sys-2@0.1.154",
    out_dir: root,
  });
  expect(cargoRuntimeDirectory(`non-json diagnostic\n${message}\n`)).toBe(
    resolve(root, "lib"),
  );
  expect(() => cargoRuntimeDirectory("{}")).toThrow("did not report");
});

test("toolkit archives must pass SHA256 verification before extraction", async () => {
  const { root } = await fixture();
  const path = resolve(root, "archive.conda");
  await writeFile(path, "abc");
  await expect(
    verifyArchive(
      path,
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    ),
  ).resolves.toBeUndefined();
  await expect(verifyArchive(path, "0".repeat(64))).rejects.toThrow(
    "checksum failed",
  );
});

test("the CUDA diagnostic command does not silently select Metal", () => {
  expect(() =>
    developmentPlan({
      root: "/fixture",
      platform: "darwin",
      env: {},
      forceCuda: true,
    }),
  ).toThrow("Linux only");
});
