import { describe, expect, it } from "vitest";

import {
  readSourceMode,
  resolveRuntimeSourceMode,
  writeSourceMode,
} from "./storage";

function memoryStorage(initial: Record<string, string> = {}): Storage {
  const values = new Map(Object.entries(initial));
  return {
    get length() {
      return values.size;
    },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(key, value),
  };
}

describe("source-mode storage", () => {
  it("reads the legacy key during the product rename", () => {
    const storage = memoryStorage({ "graphy-source-mode": "zotero" });

    expect(readSourceMode(storage)).toBe("zotero");
  });

  it("prefers the current key and rejects invalid persisted modes", () => {
    const currentStorage = memoryStorage({
      "graphite-source-mode": "demo",
      "graphy-source-mode": "zotero",
    });
    const invalidStorage = memoryStorage({
      "graphite-source-mode": "unexpected",
      "graphy-source-mode": "zotero",
    });

    expect(readSourceMode(currentStorage)).toBe("demo");
    expect(readSourceMode(invalidStorage)).toBeNull();
    expect(resolveRuntimeSourceMode(true, true, null)).toBe("demo");
  });

  it("migrates previously selected synthetic demos to the real collection", () => {
    expect(
      readSourceMode(memoryStorage({ "graphite-source-mode": "mock" })),
    ).toBe("demo");
    expect(
      readSourceMode(memoryStorage({ "graphy-source-mode": "mock" })),
    ).toBe("demo");
  });

  it("writes the Graphite key and removes the legacy key", () => {
    const storage = memoryStorage({ "graphy-source-mode": "zotero" });

    writeSourceMode(storage, "demo");

    expect(storage.getItem("graphite-source-mode")).toBe("demo");
    expect(storage.getItem("graphy-source-mode")).toBeNull();
  });

  it("defaults browser and unconfigured development runtimes to real demo data", () => {
    expect(resolveRuntimeSourceMode(false, false, "zotero")).toBe("demo");
    expect(resolveRuntimeSourceMode(true, true, null)).toBe("demo");
    expect(resolveRuntimeSourceMode(true, true, "demo")).toBe("demo");
  });

  it("uses Zotero only after an explicit desktop selection", () => {
    expect(resolveRuntimeSourceMode(true, true, "zotero")).toBe("zotero");
  });

  it("preserves the Zotero default for installed production builds", () => {
    expect(resolveRuntimeSourceMode(true, false, null)).toBe("zotero");
    expect(resolveRuntimeSourceMode(true, false, "demo")).toBe("demo");
  });
});
