import { Buffer } from "node:buffer";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  open,
  readFile,
  readlink,
  readdir,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { arch, homedir, hostname, release, tmpdir } from "node:os";
import { basename, dirname, extname, join, relative, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath, URL } from "node:url";
import { privacyFindings } from "./privacy-check.mjs";

import {
  launcher,
  llamaLibraryNames,
  cudaLibraryNames,
} from "./package-linux-cuda.mjs";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..");
const bundleRoot = resolve(
  repositoryRoot,
  ".cache/cargo-target/release/bundle",
);
const stagingRoot = resolve(repositoryRoot, "artifacts/package-smoke");
const productionIdentity = "org.graphite.research";
const developmentIdentity = "org.graphite.research.dev";
const target = process.argv[2];
const privateBuildIdentities = [
  ...new Set(
    [repositoryRoot, homedir(), hostname()]
      .filter((value) => value.length >= 4)
      .flatMap((value) => [value, value.replaceAll("\\", "/")]),
  ),
];

const targetDefinitions = new Map([
  [
    "linux-x64",
    {
      formats: ["deb", "appimage"],
      artifacts: [
        ["deb", ".deb"],
        ["appimage", ".AppImage"],
      ],
    },
  ],
  [
    "windows-x64",
    {
      formats: ["nsis"],
      artifacts: [["nsis", ".exe"]],
    },
  ],
  [
    "macos-arm64",
    {
      formats: ["dmg"],
      artifacts: [["dmg", ".dmg"]],
    },
  ],
]);

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function runCommand(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
    ...options,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    const details = [result.stdout, result.stderr].filter(Boolean).join("\n");
    throw new Error(
      `'${command} ${args.join(" ")}' failed with exit code ${result.status}.` +
        (details ? `\n${details.trim()}` : ""),
    );
  }
  return `${result.stdout ?? ""}${result.stderr ?? ""}`;
}

function runFirstAvailable(commands, args, options, purpose) {
  for (const command of commands) {
    const result = spawnSync(command, args, {
      encoding: "utf8",
      maxBuffer: 32 * 1024 * 1024,
      ...options,
    });
    if (result.error?.code === "ENOENT") continue;
    if (result.error) throw result.error;
    if (result.status !== 0) {
      const details = [result.stdout, result.stderr].filter(Boolean).join("\n");
      throw new Error(
        `'${command} ${args.join(" ")}' failed while ${purpose}.` +
          (details ? `\n${details.trim()}` : ""),
      );
    }
    return `${result.stdout ?? ""}${result.stderr ?? ""}`;
  }
  throw new Error(
    `Cannot ${purpose}: none of these executables are available on PATH: ` +
      commands.join(", "),
  );
}

async function listTree(root) {
  const entries = [];

  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const absolutePath = join(directory, entry.name);
      const relativePath = relative(root, absolutePath).replaceAll("\\", "/");
      entries.push({
        absolutePath,
        relativePath,
        type: entry.isDirectory()
          ? "directory"
          : entry.isFile()
            ? "file"
            : "link",
      });
      if (entry.isDirectory()) await visit(absolutePath);
    }
  }

  await visit(root);
  return entries;
}

async function assertNoBuildIdentity(path, packagePath) {
  if (basename(packagePath) === "libgnutls.so.30") {
    // Hash-reviewed upstream self-tests are public fixtures, not user keys.
    // Inspect the whole bounded library so PEM blocks cannot span chunks.
    assert(
      (await stat(path)).size <= 16 * 1024 * 1024,
      "GnuTLS exceeds the reviewed inspection size; review before release.",
    );
    const review = JSON.parse(
      await readFile(
        new URL("./public-crypto-fixtures.json", import.meta.url),
        "utf8",
      ),
    );
    const digests = new Set(review.fixtures.map((fixture) => fixture.sha256));
    assert(
      privacyFindings(await readFile(path), privateBuildIdentities, digests)
        .length === 0,
      `Package contains a private identity or credential signature: ${packagePath}`,
    );
    return;
  }
  const needles = privateBuildIdentities.map((value) => Buffer.from(value));
  const overlap = Math.max(256, ...needles.map((value) => value.length)) - 1;
  let tail = Buffer.alloc(0);
  for await (const chunk of createReadStream(path)) {
    const content = Buffer.concat([tail, chunk]);
    assert(
      !needles.some((needle) => content.includes(needle)) &&
        privacyFindings(content).length === 0,
      `Package contains a private identity or credential signature: ${packagePath}`,
    );
    tail = content.subarray(Math.max(0, content.length - overlap));
  }
}

async function assertSafeContents(root, entries) {
  const forbiddenSegments = new Set([
    ".cache",
    "artifacts",
    "attachments",
    "conversations",
    "indexes",
    "models",
    "node_modules",
    "pdf-cache",
    "research-library",
    "runtime-data",
  ]);
  const forbiddenExtensions = new Set([
    ".db",
    ".faiss",
    ".gguf",
    ".hnsw",
    ".onnx",
    ".ort",
    ".pdf",
    ".safetensors",
    ".sqlite",
    ".sqlite3",
    ".usearch",
  ]);
  const textExtensions = new Set([
    ".cfg",
    ".conf",
    ".css",
    ".desktop",
    ".html",
    ".ini",
    ".js",
    ".json",
    ".md",
    ".plist",
    ".txt",
    ".xml",
    ".yaml",
    ".yml",
  ]);
  const repositoryPath = repositoryRoot.replaceAll("\\", "/");

  for (const entry of entries) {
    const normalized = entry.relativePath.toLowerCase();
    const segments = normalized.split("/");
    assert(
      !segments.some((segment) => forbiddenSegments.has(segment)),
      `Package contains forbidden generated/private path: ${entry.relativePath}`,
    );
    assert(
      !forbiddenExtensions.has(extname(normalized)),
      `Package contains forbidden data/model file: ${entry.relativePath}`,
    );
    assert(
      !/\.(?:db|sqlite|sqlite3)(?:-|$)/i.test(normalized) &&
        !normalized.endsWith(".onnx.data") &&
        basename(normalized) !== "tokenizer.json" &&
        basename(normalized) !== "tokenizer_config.json",
      `Package contains forbidden database/model material: ${entry.relativePath}`,
    );
    assert(
      !normalized.includes("tauri.dev.conf") &&
        !normalized.includes("graphite development"),
      `Package contains development-only material: ${entry.relativePath}`,
    );

    if (entry.type === "file") {
      await assertNoBuildIdentity(entry.absolutePath, entry.relativePath);
    } else if (entry.type === "link") {
      const destination = await readlink(entry.absolutePath);
      assert(
        !privateBuildIdentities.some((value) => destination.includes(value)),
        `Package link contains a private build identity: ${entry.relativePath}`,
      );
    }

    if (entry.type !== "file" || !textExtensions.has(extname(normalized))) {
      continue;
    }
    const fileStat = await stat(entry.absolutePath);
    if (fileStat.size > 1024 * 1024) continue;
    const contentBuffer = await readFile(entry.absolutePath);
    if (contentBuffer.includes(0)) continue;
    const content = contentBuffer.toString("utf8").replaceAll("\\", "/");
    assert(
      !content.includes(developmentIdentity),
      `Package text contains the development identity: ${entry.relativePath}`,
    );
    assert(
      !content.includes(repositoryPath),
      `Package text contains the local repository path: ${entry.relativePath}`,
    );
    assert(
      !/(?:^|[\s"'=])(?:\/home\/|\/Users\/|[A-Za-z]:\/Users\/)/m.test(content),
      `Package text contains a machine-local home path: ${entry.relativePath}`,
    );
  }

  assert(entries.length > 0, `Package inspection root is empty: ${root}`);
}

async function assertProjectLicense(entries) {
  const matches = entries.filter(
    (entry) =>
      entry.type === "file" &&
      basename(entry.relativePath) === "Graphite-LICENSE.txt",
  );
  assert(
    matches.length === 1,
    `Package must contain exactly one Graphite GPL v3 license; found ${matches.length}.`,
  );
  const [expected, packaged] = await Promise.all([
    readFile(resolve(repositoryRoot, "LICENSE"), "utf8"),
    readFile(matches[0].absolutePath, "utf8"),
  ]);
  assert(
    packaged === expected,
    "Packaged Graphite license does not match the authoritative repository LICENSE.",
  );
}

async function findSingleArtifact(directory, extension) {
  let directoryEntries;
  try {
    directoryEntries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error.code === "ENOENT") {
      throw new Error(
        `Expected Tauri bundle directory does not exist: ${directory}`,
        { cause: error },
      );
    }
    throw error;
  }
  const matches = directoryEntries
    .filter(
      (entry) =>
        entry.isFile() &&
        entry.name.toLowerCase().endsWith(extension.toLowerCase()),
    )
    .map((entry) => resolve(directory, entry.name));
  assert(
    matches.length === 1,
    `Expected exactly one ${extension} artifact in ${directory}; found ${matches.length}.`,
  );
  return matches[0];
}

function assertNoCustomDynamicRuntime(output, platform) {
  const customRuntime = /(?:onnxruntime|llama|usearch|onig|sqlite3)/i;
  assert(
    !customRuntime.test(output),
    `${platform} executable unexpectedly depends on a separately distributed ` +
      "ONNX, llama.cpp, USearch, Oniguruma, or SQLite runtime.",
  );
}

function findApplicationExecutable(entries, filename) {
  const matches = entries.filter(
    (entry) =>
      entry.type === "file" &&
      basename(entry.relativePath).toLowerCase() === filename,
  );
  assert(
    matches.length >= 1,
    `Package does not contain the expected application executable '${filename}'.`,
  );
  return matches[0].absolutePath;
}

async function assertPeX64(path) {
  const handle = await open(path, "r");
  try {
    const header = Buffer.alloc(64);
    await handle.read(header, 0, header.length, 0);
    assert(
      header.toString("ascii", 0, 2) === "MZ",
      "Windows app is not a PE file.",
    );
    const peOffset = header.readUInt32LE(0x3c);
    const peHeader = Buffer.alloc(6);
    await handle.read(peHeader, 0, peHeader.length, peOffset);
    assert(
      peHeader.toString("ascii", 0, 4) === "PE\0\0",
      "Windows app has an invalid PE header.",
    );
    assert(
      peHeader.readUInt16LE(4) === 0x8664,
      "Windows package application is not x64 (AMD64).",
    );
  } finally {
    await handle.close();
  }
}

async function assertLinuxRuntime(root, entries) {
  assert(
    !entries.some((entry) =>
      /^lib(?:cuda\.so|nvidia-|(?:EGL|GLX|GLESv[12])_nvidia)/.test(
        basename(entry.relativePath),
      ),
    ),
    "Linux packages must use the existing system GPU driver, never bundle one.",
  );
  assert(
    (await readFile(resolve(root, "usr/bin/graphite"), "utf8")) === launcher(),
    "Linux package must use the reviewed GPU/CPU launcher.",
  );
  const runtime = resolve(root, "usr/lib/graphite-cuda");
  const expected = [
    "graphite",
    "graphite-cpu",
    ...llamaLibraryNames,
    ...cudaLibraryNames,
  ];
  const names = await readdir(runtime);
  assert(
    names.length === expected.length &&
      expected.every((name) => names.includes(name)),
    "Linux package must contain exactly the GPU runtime and CPU companion, without driver stubs.",
  );
  for (const name of expected) {
    const path = resolve(runtime, name);
    assert(
      /Machine:\s+Advanced Micro Devices X86-64/.test(
        runCommand("readelf", ["-h", path]),
      ),
      `${name} must be Linux x64.`,
    );
    const dynamic = runCommand("readelf", ["-d", path]);
    assert(
      !dynamic
        .split("\n")
        .some(
          (line) =>
            /\((?:RPATH|RUNPATH)\)/.test(line) && !line.endsWith("[$ORIGIN]"),
        ),
      `${name} retains a build-specific runtime path.`,
    );
    if (name === "graphite-cpu")
      assertNoCustomDynamicRuntime(dynamic, "Linux CPU companion");
    if (name === "graphite")
      assert(
        dynamic.includes("libllama.so.0"),
        "GPU executable must use the bundled inference runtime.",
      );
  }
  for (const name of [
    "llama-LICENSE.txt",
    "CUDA-LICENSE.txt",
    "cuBLAS-LICENSE.txt",
  ]) {
    assert(
      (
        await readFile(
          resolve(root, "usr/share/graphite/licenses", name),
          "utf8",
        )
      ).length > 100,
      `Linux runtime license is missing: ${name}`,
    );
  }
}

async function inspectLinux(artifacts, scratchDirectory) {
  const debRoot = resolve(scratchDirectory, "deb");
  await mkdir(debRoot);
  runCommand("dpkg-deb", ["--extract", artifacts[0], debRoot]);
  const debEntries = await listTree(debRoot);
  await assertSafeContents(debRoot, debEntries);
  await assertProjectLicense(debEntries);
  assert(
    debEntries.some((entry) => entry.relativePath === "usr/bin/graphite"),
    "DEB package is missing usr/bin/graphite.",
  );
  assert(
    debEntries.some((entry) => entry.relativePath.endsWith(".desktop")),
    "DEB package is missing its desktop entry.",
  );
  assert(
    debEntries.some(
      (entry) =>
        entry.relativePath.startsWith("usr/share/icons/") &&
        entry.type === "file",
    ),
    "DEB package is missing its installed application icon.",
  );
  assert(
    runCommand("dpkg-deb", ["--field", artifacts[0], "Package"]).trim() ===
      "graphite",
    "DEB package metadata has the wrong package name.",
  );
  assert(
    runCommand("dpkg-deb", ["--field", artifacts[0], "Architecture"]).trim() ===
      "amd64",
    "DEB package metadata is not x64 (amd64).",
  );

  const debDependencies = runCommand("dpkg-deb", [
    "--field",
    artifacts[0],
    "Depends",
  ]);
  assert(
    /(^|,\s*)libgomp1(?=\s|,|$)/.test(debDependencies),
    "DEB package must declare libgomp1 for native OpenMP inference.",
  );

  const appImageRoot = resolve(scratchDirectory, "appimage");
  await mkdir(appImageRoot);
  runCommand(artifacts[1], ["--appimage-extract"], {
    cwd: appImageRoot,
    env: { ...process.env, APPIMAGE_EXTRACT_AND_RUN: "1" },
  });
  const extractedRoot = resolve(appImageRoot, "squashfs-root");
  await lstat(extractedRoot);
  const appImageEntries = await listTree(extractedRoot);
  await assertSafeContents(extractedRoot, appImageEntries);
  await assertProjectLicense(appImageEntries);
  assert(
    appImageEntries.some((entry) => entry.relativePath === "AppRun"),
    "AppImage is missing AppRun.",
  );
  await assertLinuxRuntime(debRoot, debEntries);
  await assertLinuxRuntime(extractedRoot, appImageEntries);
}

async function inspectWindows(artifacts, scratchDirectory) {
  const extractedRoot = resolve(scratchDirectory, "nsis");
  await mkdir(extractedRoot);
  runFirstAvailable(
    ["7z.exe", "7z"],
    ["x", artifacts[0], `-o${extractedRoot}`, "-y"],
    {},
    "inspect the NSIS installer; install 7-Zip or add it to PATH",
  );
  const entries = await listTree(extractedRoot);
  await assertSafeContents(extractedRoot, entries);
  await assertProjectLicense(entries);
  const executable = findApplicationExecutable(entries, "graphite.exe");
  await assertPeX64(executable);

  const unexpectedSidecars = entries.filter(
    (entry) =>
      entry.type === "file" &&
      /(?:onnxruntime|llama|usearch|onig|sqlite3).*\.dll$/i.test(
        basename(entry.relativePath),
      ),
  );
  assert(
    unexpectedSidecars.length === 0,
    "NSIS unexpectedly contains custom native runtime sidecars: " +
      unexpectedSidecars.map((entry) => entry.relativePath).join(", "),
  );
}

async function inspectMacos(artifacts, scratchDirectory) {
  const mountPoint = resolve(scratchDirectory, "dmg");
  await mkdir(mountPoint);
  runCommand("hdiutil", [
    "attach",
    "-readonly",
    "-nobrowse",
    "-mountpoint",
    mountPoint,
    artifacts[0],
  ]);

  try {
    const entries = await listTree(mountPoint);
    await assertSafeContents(mountPoint, entries);
    await assertProjectLicense(entries);
    const appDirectories = entries.filter(
      (entry) =>
        entry.type === "directory" && entry.relativePath.endsWith(".app"),
    );
    assert(
      appDirectories.length === 1,
      `DMG must contain exactly one application bundle; found ${appDirectories.length}.`,
    );
    const appRoot = appDirectories[0].absolutePath;
    const infoPlist = resolve(appRoot, "Contents/Info.plist");
    const identifier = runCommand("plutil", [
      "-extract",
      "CFBundleIdentifier",
      "raw",
      "-o",
      "-",
      infoPlist,
    ]).trim();
    assert(
      identifier === productionIdentity,
      `DMG application identity is '${identifier}', expected '${productionIdentity}'.`,
    );
    const minimumVersion = runCommand("plutil", [
      "-extract",
      "LSMinimumSystemVersion",
      "raw",
      "-o",
      "-",
      infoPlist,
    ]).trim();
    assert(
      minimumVersion === "11.0",
      `DMG minimum macOS version is '${minimumVersion}', expected '11.0'.`,
    );
    const executable = resolve(appRoot, "Contents/MacOS/graphite");
    await lstat(executable);
    const architectures = runCommand("lipo", ["-archs", executable])
      .trim()
      .split(/\s+/);
    assert(
      architectures.includes("arm64") && !architectures.includes("x86_64"),
      `DMG application architectures are '${architectures.join(" ")}', expected ARM64 only.`,
    );
    const dynamicDependencies = runCommand("otool", ["-L", executable]);
    assertNoCustomDynamicRuntime(dynamicDependencies, "macOS");
  } finally {
    runCommand("hdiutil", ["detach", mountPoint]);
  }
}

async function sha256(path) {
  const digest = createHash("sha256");
  for await (const chunk of createReadStream(path)) digest.update(chunk);
  return digest.digest("hex");
}

assert(
  targetDefinitions.has(target),
  "Usage: npm run package:smoke -- <linux-x64|windows-x64|macos-arm64>",
);
assert(
  process.argv.length === 3,
  "Package smoke accepts exactly one native platform argument.",
);

const definition = targetDefinitions.get(target);
const artifactPaths = [];
for (const [directory, extension] of definition.artifacts) {
  artifactPaths.push(
    await findSingleArtifact(resolve(bundleRoot, directory), extension),
  );
}

const scratchDirectory = await mkdtemp(
  resolve(tmpdir(), "graphite-package-smoke-"),
);
try {
  if (target === "linux-x64") {
    await inspectLinux(artifactPaths, scratchDirectory);
  } else if (target === "windows-x64") {
    await inspectWindows(artifactPaths, scratchDirectory);
  } else {
    await inspectMacos(artifactPaths, scratchDirectory);
  }
} finally {
  await rm(scratchDirectory, { force: true, recursive: true });
}

const outputDirectory = resolve(stagingRoot, target);
assert(
  relative(stagingRoot, outputDirectory) === target,
  "Package staging must remain inside artifacts/package-smoke.",
);
await rm(outputDirectory, { force: true, recursive: true });
await mkdir(outputDirectory, { recursive: true });

const packages = [];
for (const artifactPath of artifactPaths) {
  const destination = resolve(outputDirectory, basename(artifactPath));
  await copyFile(artifactPath, destination);
  packages.push({
    file: basename(destination),
    sha256: await sha256(destination),
  });
}
packages.sort((left, right) => left.file.localeCompare(right.file));

await writeFile(
  resolve(outputDirectory, "SHA256SUMS.txt"),
  packages.map((entry) => `${entry.sha256}  ${entry.file}\n`).join(""),
  "utf8",
);
await writeFile(
  resolve(outputDirectory, "build-metadata.json"),
  `${JSON.stringify(
    {
      schemaVersion: 1,
      productName: "Graphite",
      identifier: productionIdentity,
      version: JSON.parse(
        await readFile(resolve(repositoryRoot, "package.json"), "utf8"),
      ).version,
      platform: target,
      sourceRevision: process.env.GITHUB_SHA ?? null,
      buildHost: {
        platform: process.platform,
        architecture: arch(),
        kernel: release(),
      },
      formats: definition.formats,
      signed: false,
      nativeRuntimeModel:
        target === "linux-x64"
          ? "Bundled CUDA inference runtime and application-linked CPU companion; no driver bundled"
          : "Application-linked native inference (Metal on macOS), indexing, tokenization, and SQLite code",
      packages,
    },
    null,
    2,
  )}\n`,
  "utf8",
);

const stagedEntries = await readdir(outputDirectory, { withFileTypes: true });
assert(
  stagedEntries.every(
    (entry) =>
      entry.isFile() &&
      (artifactPaths.some((path) => basename(path) === entry.name) ||
        entry.name === "SHA256SUMS.txt" ||
        entry.name === "build-metadata.json"),
  ),
  "Package staging contains an unexpected file or directory.",
);

process.stdout.write(
  `Unsigned ${target} package smoke passed; staged ${packages.length} package(s) ` +
    `with SHA-256 checksums in ${relative(repositoryRoot, outputDirectory)}.\n`,
);
