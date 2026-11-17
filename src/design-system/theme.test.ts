import { describe, expect, it } from "vitest";

import { readStoredTheme, resolveTheme, writeStoredTheme } from "./theme";

const storageKey = "graphite-theme:v1";

describe("theme preferences", () => {
  it("respects a valid stored preference over the system preference", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });

  it("falls back to the current system preference", () => {
    expect(resolveTheme(null, true)).toBe("dark");
    expect(resolveTheme("unsupported", false)).toBe("light");
  });

  it("reads and writes only supported theme values", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    expect(readStoredTheme(storage, storageKey)).toBeNull();
    expect(writeStoredTheme(storage, storageKey, "dark")).toBe(true);
    expect(readStoredTheme(storage, storageKey)).toBe("dark");
    values.set(storageKey, "unsupported");
    expect(readStoredTheme(storage, storageKey)).toBeNull();
  });

  it("tolerates unavailable browser storage", () => {
    const unavailableStorage = {
      getItem: () => {
        throw new Error("Storage unavailable");
      },
      setItem: () => {
        throw new Error("Storage unavailable");
      },
    };

    expect(readStoredTheme(unavailableStorage, storageKey)).toBeNull();
    expect(writeStoredTheme(unavailableStorage, storageKey, "light")).toBe(
      false,
    );
  });
});
