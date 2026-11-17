const SOURCE_MODE_STORAGE_KEY = "graphite-source-mode";
const LEGACY_SOURCE_MODE_STORAGE_KEY = "graphy-source-mode";

export type SourceMode = "demo" | "zotero";

export function resolveRuntimeSourceMode(
  isDesktopRuntime: boolean,
  isDevelopmentBuild: boolean,
  storedMode: SourceMode | null,
): SourceMode {
  if (!isDesktopRuntime) return "demo";
  if (storedMode !== null) return storedMode;
  return isDevelopmentBuild ? "demo" : "zotero";
}

export function readSourceMode(storage: Storage): SourceMode | null {
  const value =
    storage.getItem(SOURCE_MODE_STORAGE_KEY) ??
    storage.getItem(LEGACY_SOURCE_MODE_STORAGE_KEY);
  if (value === "mock") return "demo";
  return value === "demo" || value === "zotero" ? value : null;
}

export function writeSourceMode(storage: Storage, mode: SourceMode): void {
  storage.setItem(SOURCE_MODE_STORAGE_KEY, mode);
  storage.removeItem(LEGACY_SOURCE_MODE_STORAGE_KEY);
}
