import { createHash } from "node:crypto";
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import { assembleReleaseCandidate } from "./release-candidate.mjs";

const temporaryRoots = [];

function digest(content) {
  return createHash("sha256").update(content).digest("hex");
}

async function createPlatformArtifact(root, platform, formats, packages) {
  const artifactRoot = resolve(root, `graphite-0.1.0-unsigned-${platform}`);
  await mkdir(artifactRoot, { recursive: true });
  const packageMetadata = [];
  for (const [filename, content] of packages) {
    await writeFile(resolve(artifactRoot, filename), content);
    packageMetadata.push({ file: filename, sha256: digest(content) });
  }
  await writeFile(
    resolve(artifactRoot, "SHA256SUMS.txt"),
    packageMetadata.map((entry) => `${entry.sha256}  ${entry.file}\n`).join(""),
    "utf8",
  );
  await writeFile(
    resolve(artifactRoot, "build-metadata.json"),
    `${JSON.stringify({
      schemaVersion: 1,
      productName: "Graphite",
      identifier: "org.graphite.research",
      version: "0.1.0",
      sourceRevision: "a".repeat(40),
      platform,
      formats,
      signed: false,
      packages: packageMetadata,
    })}\n`,
    "utf8",
  );
  return artifactRoot;
}

async function createCandidateFixture() {
  const root = await mkdtemp(resolve(tmpdir(), "graphite-candidate-test-"));
  temporaryRoots.push(root);
  const downloadsRoot = resolve(root, "downloads");
  await mkdir(downloadsRoot);
  await createPlatformArtifact(
    downloadsRoot,
    "linux-x64",
    ["deb", "appimage"],
    [
      ["Graphite_0.1.0_amd64.deb", "linux-deb"],
      ["Graphite_0.1.0_amd64.AppImage", "linux-appimage"],
    ],
  );
  await createPlatformArtifact(
    downloadsRoot,
    "macos-arm64",
    ["dmg"],
    [["Graphite_0.1.0_aarch64.dmg", "macos-dmg"]],
  );
  await createPlatformArtifact(
    downloadsRoot,
    "windows-x64",
    ["nsis"],
    [["Graphite_0.1.0_x64-setup.exe", "windows-nsis"]],
  );
  return { downloadsRoot, outputRoot: resolve(root, "output") };
}

afterEach(async () => {
  await Promise.all(
    temporaryRoots
      .splice(0)
      .map((root) => rm(root, { force: true, recursive: true })),
  );
});

describe("unsigned release candidate assembly", () => {
  it("verifies and combines the exact native package matrix", async () => {
    const fixture = await createCandidateFixture();
    const result = await assembleReleaseCandidate({
      ...fixture,
      packageVersion: "0.1.0",
      sourceRevision: "a".repeat(40),
    });

    expect(result.packages).toHaveLength(4);
    expect(await readdir(result.candidateRoot)).toEqual(
      expect.arrayContaining([
        "Graphite_0.1.0_amd64.AppImage",
        "Graphite_0.1.0_amd64.deb",
        "Graphite_0.1.0_aarch64.dmg",
        "Graphite_0.1.0_x64-setup.exe",
        "SHA256SUMS.txt",
        "release-manifest.json",
      ]),
    );
    const manifest = JSON.parse(
      await readFile(
        resolve(result.candidateRoot, "release-manifest.json"),
        "utf8",
      ),
    );
    expect(manifest).toMatchObject({
      identifier: "org.graphite.research",
      publicRelease: false,
      signed: false,
      sourceRevision: "a".repeat(40),
      version: "0.1.0",
    });
  });

  it("rejects a package changed after its checksum manifest was created", async () => {
    const fixture = await createCandidateFixture();
    await writeFile(
      resolve(
        fixture.downloadsRoot,
        "graphite-0.1.0-unsigned-linux-x64",
        "Graphite_0.1.0_amd64.deb",
      ),
      "tampered",
    );

    await expect(
      assembleReleaseCandidate({
        ...fixture,
        packageVersion: "0.1.0",
        sourceRevision: "a".repeat(40),
      }),
    ).rejects.toThrow("package checksum failed");
  });

  it("requires the Windows installer alongside the other platform artifacts", async () => {
    const fixture = await createCandidateFixture();
    await rm(
      resolve(fixture.downloadsRoot, "graphite-0.1.0-unsigned-windows-x64"),
      { recursive: true },
    );

    await expect(
      assembleReleaseCandidate({
        ...fixture,
        packageVersion: "0.1.0",
        sourceRevision: "a".repeat(40),
      }),
    ).rejects.toThrow("graphite-0.1.0-unsigned-windows-x64");
  });
});

it("rejects mixing builds from different source revisions", async () => {
  const fixture = await createCandidateFixture();
  await expect(
    assembleReleaseCandidate({
      ...fixture,
      packageVersion: "0.1.0",
      sourceRevision: "b".repeat(40),
    }),
  ).rejects.toThrow("metadata does not describe");
});
