import { expect, it } from "vitest";
import { assertNoCustomDynamicRuntime } from "./package-runtime-check.mjs";

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
