import { posix, win32 } from "node:path";

import { describe, expect, it } from "vitest";

import {
  createReleaseMetadata,
  resolveReleaseMetadataPath,
  validatePublisherName,
} from "./release-metadata.mjs";

describe("production release metadata", () => {
  it("creates a narrow Tauri publisher overlay", () => {
    expect(createReleaseMetadata("Graphite", 2026)).toEqual({
      bundle: {
        copyright: "Copyright (c) 2026 Graphite",
        publisher: "Graphite",
      },
    });
  });

  it("rejects private or placeholder publisher values", () => {
    for (const value of ["Other publisher", "", "Graphite\nInjected"]) {
      expect(() => validatePublisherName(value)).toThrow("neutral Graphite");
    }
  });

  it("keeps generated metadata below the runner temporary directory", () => {
    expect(resolveReleaseMetadataPath("/runner/temp", posix)).toBe(
      "/runner/temp/graphite-production-release/tauri.publisher.conf.json",
    );
    expect(resolveReleaseMetadataPath("C:\\runner\\temp", win32)).toBe(
      "C:\\runner\\temp\\graphite-production-release\\tauri.publisher.conf.json",
    );
  });
});
