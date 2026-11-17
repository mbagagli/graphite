import { Buffer } from "node:buffer";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";
import { assertNoBuildIdentity } from "./package-privacy-check.mjs";

it("scans binaries across read boundaries and reports only finding categories", async () => {
  const root = await mkdtemp(join(tmpdir(), "graphite-privacy-"));
  const path = join(root, "graphite");
  const identity = "/example/private-build";
  const token = "gh" + "p_" + "A".repeat(40);
  const pem = "-----BEGIN " + "PRIVATE KEY-----\n" + "B".repeat(64);
  try {
    for (const [value, category] of [
      [identity, "local build identity"],
      [token, "credential-shaped token"],
      [pem, "private key"],
    ]) {
      for (const offset of [0, 65536 - 8]) {
        await writeFile(
          path,
          Buffer.concat([
            Buffer.alloc(offset),
            Buffer.from(value),
            Buffer.alloc(65536),
          ]),
        );
        await expect(
          assertNoBuildIdentity(path, "MacOS/graphite", [identity]),
        ).rejects.toThrow(`MacOS/graphite: ${category}`);
        try {
          await assertNoBuildIdentity(path, "MacOS/graphite", [identity]);
        } catch (error) {
          expect(error.message).not.toContain(value);
          expect(error.message).not.toContain(root);
        }
      }
    }
    await writeFile(path, Buffer.from("\0/build-home/.cargo/source.m\0"));
    await expect(
      assertNoBuildIdentity(path, "MacOS/graphite", [identity]),
    ).resolves.toBeUndefined();
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
