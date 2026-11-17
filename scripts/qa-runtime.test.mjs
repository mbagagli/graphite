import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  artifactDirectory,
  assertBrowserQaIsolation,
  firefoxOptions,
  formatDriverStartupError,
  parsePerformanceArguments,
  startWebDriver,
} from "./qa-runtime.mjs";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

describe("portable QA runtime", () => {
  it("places inspectable artifacts under the ignored repository root", () => {
    expect(artifactDirectory("visual", {})).toBe(
      resolve(repositoryRoot, "artifacts", "qa", "visual"),
    );
    expect(
      artifactDirectory("performance", {
        GRAPHITE_QA_ARTIFACTS_DIR: "custom-results",
      }),
    ).toBe(resolve(repositoryRoot, "custom-results", "performance"));
  });

  it("constructs Firefox options with an optional explicit binary", () => {
    expect(firefoxOptions({})).toEqual({ args: ["-headless"] });
    expect(
      firefoxOptions({ GRAPHITE_QA_FIREFOX_BINARY: "custom-firefox" }),
    ).toEqual({ args: ["-headless"], binary: "custom-firefox" });
  });

  it("parses safe performance labels and rejects path-shaped labels", () => {
    expect(parsePerformanceArguments(["baseline", "--large"])).toEqual({
      largeGraphMode: true,
      profileLabel: "baseline",
    });
    expect(() => parsePerformanceArguments(["../escape"])).toThrow(
      "safe 1-64 character filename",
    );
    expect(() => parsePerformanceArguments(["first", "second"])).toThrow(
      "at most one profile label",
    );
  });

  it("provides actionable driver startup guidance", () => {
    const error = formatDriverStartupError(
      "missing-driver",
      new Error("not found"),
      "diagnostic",
    );

    expect(error.message).toContain("PATH");
    expect(error.message).toContain("GRAPHITE_QA_GECKODRIVER");
    expect(error.message).toContain("diagnostic");
  });

  it("fails clearly when the configured driver executable is absent", async () => {
    await expect(
      startWebDriver({
        GRAPHITE_QA_GECKODRIVER:
          "graphite-deliberately-missing-geckodriver-executable",
        GRAPHITE_QA_DRIVER_PORT: "4447",
      }),
    ).rejects.toThrow("Install geckodriver on PATH");
  });

  it("accepts only loopback, mock-only browser sessions", async () => {
    const safeRequest = async () => ({
      externalResources: [],
      hasDemoSource: true,
      hasTauriRuntime: false,
      hostname: "127.0.0.1",
    });
    const unsafeRequest = async () => ({
      externalResources: ["https://example.invalid/resource"],
      hasDemoSource: false,
      hasTauriRuntime: true,
      hostname: "example.invalid",
    });

    await expect(
      assertBrowserQaIsolation(safeRequest, "safe"),
    ).resolves.toEqual(await safeRequest());
    await expect(
      assertBrowserQaIsolation(unsafeRequest, "unsafe"),
    ).rejects.toThrow("Browser QA isolation failed");
  });
});
