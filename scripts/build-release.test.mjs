import { expect, it, vi } from "vitest";

const fixture = vi.hoisted(() => ({
  run: vi.fn(() => ({ status: 0 })),
  environment: {
    APPLE_ID: "test-account",
    APPLE_PASSWORD: "test-value",
    APPLE_SIGNING_IDENTITY: "test-certificate",
    APPLE_API_KEY_PATH: "test-key-file",
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
});
