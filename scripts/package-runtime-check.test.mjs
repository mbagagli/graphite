import { expect, it } from "vitest";
import {
  assertNoCustomDynamicRuntime,
  assertWindowsCpuImports,
} from "./package-runtime-check.mjs";

const otool = (path) => `Graphite:\n\t${path} (compatibility version 1.0.0)\n`;

it("accepts macOS system SQLite and system frameworks", () => {
  expect(() =>
    assertNoCustomDynamicRuntime(
      otool("/usr/lib/libsqlite3.dylib") +
        "\t/System/Library/Frameworks/Metal.framework/Versions/A/Metal\n",
      "macOS",
    ),
  ).not.toThrow();
});

it.each([
  "@rpath/libsqlite3.dylib",
  "/opt/homebrew/lib/libsqlite3.dylib",
  "/usr/lib/../local/lib/libsqlite3.dylib",
  "/usr/lib/libllama.dylib",
  "@rpath/libonnxruntime.dylib",
  "@rpath/libusearch.dylib",
  "@rpath/libonig.dylib",
])("rejects external or forbidden runtime %s", (path) => {
  expect(() => assertNoCustomDynamicRuntime(otool(path), "macOS")).toThrow();
});

it("keeps Linux runtime rejection unchanged", () => {
  expect(() =>
    assertNoCustomDynamicRuntime(
      "libsqlite3.so => /usr/lib/libsqlite3.so",
      "Linux",
    ),
  ).toThrow();
  expect(() =>
    assertNoCustomDynamicRuntime("libc.so.6 => /usr/lib/libc.so.6", "Linux"),
  ).not.toThrow();
});

it("accepts Windows system DLLs in a CPU build", () => {
  expect(() =>
    assertWindowsCpuImports(
      "Import Tables\n\tDLL Name: KERNEL32.dll\n\tDLL Name: DWrite.dll\n",
    ),
  ).not.toThrow();
});

it.each([
  "cudart64_13.dll",
  "nvcuda.dll",
  "cublasLt64_13.dll",
  "ggml-cuda.dll",
])("rejects CUDA import %s in a Windows CPU package", (filename) => {
  expect(() => assertWindowsCpuImports(`\tDLL Name: ${filename}\n`)).toThrow(
    "imports CUDA DLLs",
  );
});

it("requires real Windows import records and a linked ONNX runtime", () => {
  expect(() => assertWindowsCpuImports("empty table")).toThrow(
    "no DLL imports",
  );
  expect(() =>
    assertWindowsCpuImports("\tDLL Name: onnxruntime.dll\r\n"),
  ).toThrow("separately distributed");
});
