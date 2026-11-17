import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  copyFile,
  mkdir,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { basename, dirname, isAbsolute, relative, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..");
const productionIdentity = "org.graphite.research";
const expectedPlatforms = new Map([
  ["linux-x64", ["deb", "appimage"]],
  ["macos-arm64", ["dmg"]],
]);

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function sha256(path) {
  const digest = createHash("sha256");
  for await (const chunk of createReadStream(path)) digest.update(chunk);
  return digest.digest("hex");
}

async function readJson(path) {
  return JSON.parse(await readFile(path, "utf8"));
}

function arraysEqual(left, right) {
  return (
    Array.isArray(left) &&
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

function parseChecksumManifest(content, platform) {
  const checksums = new Map();
  for (const line of content.split(/\r?\n/).filter(Boolean)) {
    const match = line.match(/^([a-f0-9]{64}) {2}([^/\\]+)$/);
    assert(match, `Invalid ${platform} checksum line: '${line}'.`);
    assert(
      !checksums.has(match[2]),
      `Duplicate ${platform} checksum entry '${match[2]}'.`,
    );
    checksums.set(match[2], match[1]);
  }
  return checksums;
}

function assertSafePackageName(filename, platform) {
  assert(
    filename === basename(filename) && !isAbsolute(filename),
    `${platform} package filename must not contain a path: '${filename}'.`,
  );
  assert(
    !filename.startsWith(".") && !filename.includes(".."),
    `${platform} package filename is unsafe: '${filename}'.`,
  );
}

export async function assembleReleaseCandidate({
  downloadsRoot,
  outputRoot,
  packageVersion,
  sourceRevision,
}) {
  assert(
    /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(packageVersion),
    `Release candidate version '${packageVersion}' is invalid.`,
  );
  assert(
    /^[a-f0-9]{40}$/.test(sourceRevision),
    "Release candidate source revision must be a full lowercase commit SHA.",
  );

  const candidateRoot = resolve(outputRoot, `graphite-${packageVersion}`);
  const candidateRelative = relative(outputRoot, candidateRoot);
  assert(
    candidateRelative === `graphite-${packageVersion}`,
    "Release candidate output escaped its staging root.",
  );
  await rm(outputRoot, { force: true, recursive: true });
  await mkdir(candidateRoot, { recursive: true });

  const downloadedEntries = await readdir(downloadsRoot, {
    withFileTypes: true,
  });
  const artifactDirectories = downloadedEntries
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();
  const expectedDirectories = [...expectedPlatforms.keys()]
    .map((platform) => `graphite-${packageVersion}-unsigned-${platform}`)
    .sort();
  assert(
    arraysEqual(artifactDirectories, expectedDirectories),
    `Release candidate requires exactly these platform artifacts: ${expectedDirectories.join(", ")}.`,
  );

  const packages = [];
  const platforms = [];
  const packageNames = new Set();
  for (const [platform, formats] of expectedPlatforms) {
    const artifactRoot = resolve(
      downloadsRoot,
      `graphite-${packageVersion}-unsigned-${platform}`,
    );
    const metadata = await readJson(
      resolve(artifactRoot, "build-metadata.json"),
    );
    assert(
      metadata.schemaVersion === 1,
      `${platform} metadata schema changed.`,
    );
    assert(
      metadata.productName === "Graphite" &&
        metadata.identifier === productionIdentity &&
        metadata.version === packageVersion &&
        metadata.sourceRevision === sourceRevision &&
        metadata.platform === platform &&
        metadata.signed === false,
      `${platform} metadata does not describe the expected unsigned production package.`,
    );
    assert(
      arraysEqual(metadata.formats, formats),
      `${platform} metadata has unexpected package formats.`,
    );
    assert(
      Array.isArray(metadata.packages) && metadata.packages.length > 0,
      `${platform} metadata contains no packages.`,
    );

    const expectedNames =
      platform === "linux-x64"
        ? [
            `Graphite_${packageVersion}_amd64.deb`,
            `Graphite_${packageVersion}_amd64.AppImage`,
          ]
        : [`Graphite_${packageVersion}_aarch64.dmg`];
    assert(
      arraysEqual(
        metadata.packages.map((entry) => entry.file).sort(),
        expectedNames.sort(),
      ),
      `${platform} package filenames do not match the release version and formats.`,
    );

    const checksumManifest = parseChecksumManifest(
      await readFile(resolve(artifactRoot, "SHA256SUMS.txt"), "utf8"),
      platform,
    );
    assert(
      checksumManifest.size === metadata.packages.length,
      `${platform} checksum and metadata package counts differ.`,
    );

    for (const packageMetadata of metadata.packages) {
      const filename = packageMetadata.file;
      assertSafePackageName(filename, platform);
      assert(
        !packageNames.has(filename),
        `Release candidate contains duplicate package filename '${filename}'.`,
      );
      packageNames.add(filename);
      assert(
        /^[a-f0-9]{64}$/.test(packageMetadata.sha256) &&
          checksumManifest.get(filename) === packageMetadata.sha256,
        `${platform} metadata and checksum manifest disagree for '${filename}'.`,
      );
      const source = resolve(artifactRoot, filename);
      const actualDigest = await sha256(source);
      assert(
        actualDigest === packageMetadata.sha256,
        `${platform} package checksum failed for '${filename}'.`,
      );
      await copyFile(source, resolve(candidateRoot, filename));
      packages.push({ file: filename, platform, sha256: actualDigest });
    }
    platforms.push({ formats, platform });
  }

  packages.sort((left, right) => left.file.localeCompare(right.file));
  platforms.sort((left, right) => left.platform.localeCompare(right.platform));
  await writeFile(
    resolve(candidateRoot, "SHA256SUMS.txt"),
    packages.map((entry) => `${entry.sha256}  ${entry.file}\n`).join(""),
    "utf8",
  );
  await writeFile(
    resolve(candidateRoot, "release-manifest.json"),
    `${JSON.stringify(
      {
        schemaVersion: 1,
        productName: "Graphite",
        identifier: productionIdentity,
        version: packageVersion,
        sourceRevision,
        signed: false,
        publicRelease: false,
        platforms,
        packages,
      },
      null,
      2,
    )}\n`,
    "utf8",
  );

  const outputEntries = (await readdir(candidateRoot, { withFileTypes: true }))
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name)
    .sort();
  const expectedOutput = [
    ...packages.map((entry) => entry.file),
    "SHA256SUMS.txt",
    "release-manifest.json",
  ].sort();
  assert(
    arraysEqual(outputEntries, expectedOutput),
    "Release candidate staging contains unexpected material.",
  );

  return { candidateRoot, packages, platforms };
}

async function main() {
  assert(
    process.env.CI === "true" && process.env.GITHUB_ACTIONS === "true",
    "Release-candidate assembly is restricted to owner-triggered GitHub Actions.",
  );
  const packageMetadata = await readJson(
    resolve(repositoryRoot, "package.json"),
  );
  const result = await assembleReleaseCandidate({
    downloadsRoot: resolve(repositoryRoot, "artifacts/package-downloads"),
    outputRoot: resolve(repositoryRoot, "artifacts/release-candidate"),
    packageVersion: packageMetadata.version,
    sourceRevision: process.env.GITHUB_SHA ?? "",
  });
  process.stdout.write(
    `Unsigned release candidate passed: ${result.packages.length} packages ` +
      `for ${result.platforms.length} native targets staged below ` +
      `${relative(repositoryRoot, result.candidateRoot)}.\n`,
  );
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) await main();
