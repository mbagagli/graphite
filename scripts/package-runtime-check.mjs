/** Reject unbundled custom runtimes while allowing macOS system SQLite. */
export function assertNoCustomDynamicRuntime(output, platform) {
  const customRuntime = /(?:onnxruntime|llama|usearch|onig|sqlite3)/i;
  const dependencies =
    platform === "macOS"
      ? output
          .split("\n")
          .slice(1)
          .map((line) => line.trim().split(/\s+\(/)[0])
          .filter(
            (path) => !/^\/usr\/lib\/libsqlite3(?:\.\d+)*\.dylib$/.test(path),
          )
      : [output];
  if (dependencies.some((path) => customRuntime.test(path))) {
    throw new Error(
      `${platform} executable unexpectedly depends on a separately distributed ` +
        "ONNX, llama.cpp, USearch, Oniguruma, or SQLite runtime.",
    );
  }
}
