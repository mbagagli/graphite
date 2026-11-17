import { posix, win32 } from "node:path";

import { describe, expect, it } from "vitest";

import { resolveRuntimeLayout } from "./installed-package-smoke.mjs";

describe("installed package runtime layout", () => {
  it("isolates Linux state below runner-temporary XDG roots", () => {
    const layout = resolveRuntimeLayout(
      "linux-x64",
      { RUNNER_TEMP: "/runner/temp" },
      posix,
    );

    expect(layout.databasePath).toBe(
      "/runner/temp/graphite-installed-smoke/linux-x64/xdg-data/" +
        "org.graphite.research/graphite-research.sqlite3",
    );
    expect(layout.childEnvironment.HOME).toBe(
      "/runner/temp/graphite-installed-smoke/linux-x64/home",
    );
    expect(layout.developmentRoots).toEqual([
      "/runner/temp/graphite-installed-smoke/linux-x64/xdg-data/" +
        "org.graphite.research.dev",
    ]);
  });

  it("isolates macOS state below a runner-temporary home", () => {
    const layout = resolveRuntimeLayout(
      "macos-arm64",
      { RUNNER_TEMP: "/runner/temp" },
      posix,
    );

    expect(layout.databasePath).toBe(
      "/runner/temp/graphite-installed-smoke/macos-arm64/home/Library/" +
        "Application Support/org.graphite.research/graphite-research.sqlite3",
    );
    expect(layout.productionRoots).toEqual([
      "/runner/temp/graphite-installed-smoke/macos-arm64/home/Library/" +
        "Application Support/org.graphite.research",
    ]);
  });

  it("uses Windows known-folder locations only on an ephemeral runner", () => {
    const layout = resolveRuntimeLayout(
      "windows-x64",
      {
        APPDATA: "C:\\Users\\runner\\AppData\\Roaming",
        LOCALAPPDATA: "C:\\Users\\runner\\AppData\\Local",
        RUNNER_TEMP: "D:\\a\\_temp",
      },
      win32,
    );

    expect(layout.databasePath).toBe(
      "C:\\Users\\runner\\AppData\\Local\\org.graphite.research\\" +
        "graphite-research.sqlite3",
    );
    expect(layout.developmentRoots).toEqual([
      "C:\\Users\\runner\\AppData\\Roaming\\org.graphite.research.dev",
      "C:\\Users\\runner\\AppData\\Local\\org.graphite.research.dev",
    ]);
  });

  it("rejects relative runner-temporary paths", () => {
    expect(() =>
      resolveRuntimeLayout(
        "linux-x64",
        { RUNNER_TEMP: "relative/temp" },
        posix,
      ),
    ).toThrow("RUNNER_TEMP must be an absolute path");
  });
});
