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

const windowsCudaLibrary =
  /^(?:nvcuda|cudart|cublas|cufft|curand|cusolver|cusparse|nvrtc|nvjitlink|(?:lib)?ggml-cuda)[\w.-]*\.dll$/i;

export function isWindowsCudaLibrary(filename) {
  return windowsCudaLibrary.test(filename);
}

/** Check the executable's PE imports rather than the build runner's PATH. */
export function assertWindowsCpuImports(output) {
  const imports = [...output.matchAll(/^\s*DLL Name:\s*([^\r\n]+)$/gim)].map(
    ([, filename]) => filename.trim(),
  );
  if (imports.length === 0) {
    throw new Error("Windows executable inspection reported no DLL imports.");
  }
  assertNoCustomDynamicRuntime(imports.join("\n"), "Windows CPU");
  if (imports.some(isWindowsCudaLibrary)) {
    throw new Error("Windows CPU executable unexpectedly imports CUDA DLLs.");
  }
}
