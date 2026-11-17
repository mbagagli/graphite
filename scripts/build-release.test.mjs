import { expect, it, vi } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir, homedir, platform } from "node:os";
import { join } from "node:path";

const fixture = vi.hoisted(() => ({
  run: vi.fn(() => ({ status: 0 })),
  environment: {
    APPLE_ID: "test-account",
    APPLE_PASSWORD: "test-value",
    APPLE_SIGNING_IDENTITY: "test-certificate",
    APPLE_API_KEY_PATH: "test-key-file",
    CMAKE_OBJC_FLAGS: "-DGRAPHITE_OBJC_TEST=1",
    OBJCXXFLAGS: "-DGRAPHITE_OBJCXX_TEST=1",
  },
}));

vi.mock("node:child_process", async (importOriginal) => ({
  ...(await importOriginal()),
  spawnSync: fixture.run,
}));

vi.mock("node:process", async (importOriginal) => ({
  default: {
    ...(await importOriginal()).default,
    platform: "darwin",
    arch: "arm64",
    argv: ["node", "build-release.mjs"],
    env: fixture.environment,
  },
}));

it("builds a Metal DMG without forwarding Apple credentials or skipping its ad-hoc signature", async () => {
  await import("./build-release.mjs");
  expect(fixture.run).toHaveBeenCalledTimes(1);
  const [, args, options] = fixture.run.mock.calls[0];
  expect(args).toEqual(
    expect.arrayContaining(["--bundles", "dmg", "--features", "agent-metal"]),
  );
  expect(args).not.toContain("--no-sign");
  expect(
    Object.keys(options.env).filter((key) => key.startsWith("APPLE_")),
  ).toEqual([]);
  expect(fixture.environment.APPLE_ID).toBe("test-account");
  for (const language of ["OBJC", "OBJCXX"]) {
    expect(options.env[`CMAKE_${language}_FLAGS`]).toContain(
      `-DGRAPHITE_${language}_TEST=1`,
    );
    expect(options.env[`CMAKE_${language}_FLAGS`]).toContain(
      `-ffile-prefix-map=${homedir()}=/build-home`,
    );
    expect(options.env[`CMAKE_${language}_FLAGS`]).toContain(
      "=/graphite-source",
    );
  }
});

it.runIf(platform() === "darwin")(
  "removes assertion source paths from compiled Objective-C and Objective-C++",
  async () => {
    await import("./build-release.mjs");
    const env = fixture.run.mock.calls[0][2].env;
    const root = await realpath(
      await mkdtemp(join(tmpdir(), "graphite-objc-")),
    );
    try {
      await writeFile(
        join(root, "CMakeLists.txt"),
        [
          "cmake_minimum_required(VERSION 3.16)",
          "project(privacy_fixture LANGUAGES OBJC OBJCXX)",
          "add_library(paths STATIC path.m path.mm)",
        ].join("\n"),
      );
      // Simulate Metal's __FILE__ assertion without framework dependencies.
      // The virtual path represents a dependency compiled from Cargo's cache.
      for (const [suffix, name] of [
        ["m", "objc"],
        ["mm", "objcxx"],
      ]) {
        const sourcePath = `${homedir()}/.cargo/fixture/path.${suffix}`;
        await writeFile(
          join(root, `path.${suffix}`),
          `#line 1 ${JSON.stringify(sourcePath)}\n` +
            `const char *${name}_path(void) { return __FILE__; }\n`,
        );
      }
      for (const remapped of [false, true]) {
        const build = join(root, remapped ? "remapped" : "baseline");
        execFileSync(
          "cmake",
          [
            "-S",
            root,
            "-B",
            build,
            ...["OBJC", "OBJCXX"].map(
              (language) =>
                `-DCMAKE_${language}_FLAGS=${
                  remapped ? env[`CMAKE_${language}_FLAGS`] : ""
                }`,
            ),
          ],
          { stdio: "pipe", timeout: 30000 },
        );
        execFileSync("cmake", ["--build", build], {
          stdio: "pipe",
          timeout: 30000,
        });
        const library = await readFile(join(build, "libpaths.a"));
        for (const suffix of ["m", "mm"]) {
          expect(
            library.includes(`${homedir()}/.cargo/fixture/path.${suffix}`),
          ).toBe(!remapped);
          expect(
            library.includes(`/build-home/.cargo/fixture/path.${suffix}`),
          ).toBe(remapped);
        }
      }
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  },
  120000,
);
