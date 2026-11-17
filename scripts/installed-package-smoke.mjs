import { Buffer } from "node:buffer";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  access,
  chmod,
  mkdir,
  open,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { createReadStream } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..");
const packageStagingRoot = resolve(repositoryRoot, "artifacts/package-smoke");
const productionIdentity = "org.graphite.research";
const developmentIdentity = "org.graphite.research.dev";
const databaseFilename = "graphite-research.sqlite3";
const preservationFilename = "phase3b-preservation-sentinel.txt";
const preservationContent = "Graphite Phase 3B preservation sentinel\n";
const developmentContent = "Graphite Phase 3B development sentinel\n";
const startupTimeoutMs = 45_000;
const stabilityWindowMs = 3_000;
const defaultPathApi = { isAbsolute, join, relative, resolve };

const targetDefinitions = new Map([
  [
    "linux-x64",
    {
      platform: "linux",
      artifacts: [
        [".deb", "deb"],
        [".AppImage", "appimage"],
      ],
    },
  ],
  [
    "windows-x64",
    {
      platform: "win32",
      artifacts: [[".exe", "installer"]],
    },
  ],
  [
    "macos-arm64",
    {
      platform: "darwin",
      artifacts: [[".dmg", "dmg"]],
    },
  ],
]);

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function exists(path) {
  try {
    await access(path);
    return true;
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

async function waitForAbsent(path, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while ((await exists(path)) && Date.now() < deadline) await sleep(100);
  return !(await exists(path));
}

function assertChildPath(parent, child, description, pathApi = defaultPathApi) {
  const childRelative = pathApi.relative(parent, child);
  assert(
    childRelative.length > 0 &&
      !childRelative.startsWith("..") &&
      !pathApi.isAbsolute(childRelative),
    `${description} must remain inside '${parent}'.`,
  );
}

function requireAbsoluteEnvironmentPath(
  environment,
  name,
  pathApi = defaultPathApi,
) {
  const value = environment[name];
  assert(value, `Installed smoke requires the ${name} environment variable.`);
  assert(pathApi.isAbsolute(value), `${name} must be an absolute path.`);
  return pathApi.resolve(value);
}

export function resolveRuntimeLayout(
  target,
  environment,
  pathApi = defaultPathApi,
) {
  const definition = targetDefinitions.get(target);
  assert(definition, `Unsupported installed-smoke target '${target}'.`);

  const runnerTemp = requireAbsoluteEnvironmentPath(
    environment,
    "RUNNER_TEMP",
    pathApi,
  );
  const scratchRoot = pathApi.resolve(
    runnerTemp,
    "graphite-installed-smoke",
    target,
  );
  assertChildPath(
    runnerTemp,
    scratchRoot,
    "Installed-smoke scratch root",
    pathApi,
  );

  const childEnvironment = { ...environment };
  let roamingBase;
  let localBase;
  if (target === "linux-x64") {
    const home = pathApi.resolve(scratchRoot, "home");
    const data = pathApi.resolve(scratchRoot, "xdg-data");
    childEnvironment.HOME = home;
    childEnvironment.XDG_DATA_HOME = data;
    childEnvironment.XDG_CONFIG_HOME = pathApi.resolve(
      scratchRoot,
      "xdg-config",
    );
    childEnvironment.XDG_CACHE_HOME = pathApi.resolve(scratchRoot, "xdg-cache");
    childEnvironment.XDG_STATE_HOME = pathApi.resolve(scratchRoot, "xdg-state");
    roamingBase = data;
    localBase = data;
  } else if (target === "macos-arm64") {
    const home = pathApi.resolve(scratchRoot, "home");
    childEnvironment.HOME = home;
    roamingBase = pathApi.resolve(home, "Library/Application Support");
    localBase = roamingBase;
  } else {
    roamingBase = requireAbsoluteEnvironmentPath(
      environment,
      "APPDATA",
      pathApi,
    );
    localBase = requireAbsoluteEnvironmentPath(
      environment,
      "LOCALAPPDATA",
      pathApi,
    );
  }

  const productionRoamingRoot = pathApi.resolve(
    roamingBase,
    productionIdentity,
  );
  const productionLocalRoot = pathApi.resolve(localBase, productionIdentity);
  const developmentRoamingRoot = pathApi.resolve(
    roamingBase,
    developmentIdentity,
  );
  const developmentLocalRoot = pathApi.resolve(localBase, developmentIdentity);
  assertChildPath(
    roamingBase,
    productionRoamingRoot,
    "Production roaming root",
    pathApi,
  );
  assertChildPath(
    localBase,
    productionLocalRoot,
    "Production local root",
    pathApi,
  );
  assertChildPath(
    roamingBase,
    developmentRoamingRoot,
    "Development roaming root",
    pathApi,
  );
  assertChildPath(
    localBase,
    developmentLocalRoot,
    "Development local root",
    pathApi,
  );

  return {
    childEnvironment,
    databasePath: pathApi.resolve(productionLocalRoot, databaseFilename),
    developmentRoots: [
      ...new Set([developmentRoamingRoot, developmentLocalRoot]),
    ],
    platform: definition.platform,
    preservationPath: pathApi.resolve(
      productionLocalRoot,
      preservationFilename,
    ),
    productionLocalRoot,
    productionRoots: [...new Set([productionRoamingRoot, productionLocalRoot])],
    scratchRoot,
  };
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

async function findSingleArtifact(target, extension) {
  const directory = resolve(packageStagingRoot, target);
  const entries = await readdir(directory, { withFileTypes: true });
  const matches = entries
    .filter(
      (entry) =>
        entry.isFile() &&
        entry.name.toLowerCase().endsWith(extension.toLowerCase()),
    )
    .map((entry) => resolve(directory, entry.name));
  assert(
    matches.length === 1,
    `Expected exactly one '${extension}' package in '${directory}'; found ${matches.length}.`,
  );
  return matches[0];
}

async function resolveArtifacts(target) {
  const definition = targetDefinitions.get(target);
  const artifacts = {};
  for (const [extension, name] of definition.artifacts) {
    artifacts[name] = await findSingleArtifact(target, extension);
  }
  return artifacts;
}

async function assertFirstRunRoots(layout) {
  for (const root of layout.productionRoots) {
    assert(
      !(await exists(root)),
      `Refusing installed smoke because production state already exists at '${root}'.`,
    );
  }
  for (const root of layout.developmentRoots) {
    assert(
      !(await exists(root)),
      `Refusing installed smoke because development state already exists at '${root}'.`,
    );
    await mkdir(root, { recursive: true });
    await writeFile(
      resolve(root, "phase3b-development-sentinel.txt"),
      developmentContent,
      "utf8",
    );
  }
}

async function assertDevelopmentRoots(layout) {
  for (const root of layout.developmentRoots) {
    const entries = await readdir(root, { withFileTypes: true });
    assert(
      entries.length === 1 &&
        entries[0].isFile() &&
        entries[0].name === "phase3b-development-sentinel.txt",
      `Production startup modified development state at '${root}'.`,
    );
    assert(
      (await readFile(resolve(root, entries[0].name), "utf8")) ===
        developmentContent,
      `Production startup changed the development sentinel at '${root}'.`,
    );
  }
}

async function assertSQLiteDatabase(path) {
  const handle = await open(path, "r");
  try {
    const header = Buffer.alloc(16);
    const { bytesRead } = await handle.read(header, 0, header.length, 0);
    assert(
      bytesRead === header.length,
      "Installed database header is truncated.",
    );
    assert(
      header.toString("utf8") === "SQLite format 3\0",
      "Installed startup did not create a valid SQLite database.",
    );
  } finally {
    await handle.close();
  }
}

async function assertNoImplicitModels(roots) {
  const forbiddenExtensions = new Set([
    ".gguf",
    ".onnx",
    ".ort",
    ".safetensors",
  ]);

  async function inspect(directory) {
    if (!(await exists(directory))) return;
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = resolve(directory, entry.name);
      assert(
        !entry.isSymbolicLink(),
        `Installed application data contains an unexpected link: '${path}'.`,
      );
      if (entry.isDirectory()) {
        await inspect(path);
      } else if (entry.isFile()) {
        const lowerName = entry.name.toLowerCase();
        assert(
          ![...forbiddenExtensions].some((suffix) =>
            lowerName.endsWith(suffix),
          ),
          `Installed startup downloaded an unexpected model asset: '${path}'.`,
        );
      }
    }
  }

  for (const root of roots) await inspect(root);
}

function captureOutput(stream, append) {
  stream?.setEncoding("utf8");
  stream?.on("data", (chunk) => append(chunk));
}

async function terminateProcessTree(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  if (process.platform === "win32") {
    runCommand("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"]);
  } else {
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
  }

  const deadline = Date.now() + 5_000;
  while (
    child.exitCode === null &&
    child.signalCode === null &&
    Date.now() < deadline
  ) {
    await sleep(100);
  }
  if (child.exitCode === null && child.signalCode === null) {
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch (error) {
      if (error.code !== "ESRCH") throw error;
    }
  }
}

async function launchUntilReady(executable, layout) {
  await access(executable);
  let standardOutput = "";
  let standardError = "";
  const child = spawn(executable, [], {
    detached: process.platform !== "win32",
    env: layout.childEnvironment,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  const appendOutput = (name) => (chunk) => {
    const combined = `${name === "stdout" ? standardOutput : standardError}${chunk}`;
    if (name === "stdout") standardOutput = combined.slice(-32_768);
    else standardError = combined.slice(-32_768);
  };
  captureOutput(child.stdout, appendOutput("stdout"));
  captureOutput(child.stderr, appendOutput("stderr"));

  try {
    const deadline = Date.now() + startupTimeoutMs;
    while (!(await exists(layout.databasePath)) && Date.now() < deadline) {
      if (child.exitCode !== null || child.signalCode !== null) {
        throw new Error(
          `Installed Graphite exited before creating its database.` +
            `\nstdout:\n${standardOutput}\nstderr:\n${standardError}`,
        );
      }
      await sleep(250);
    }
    assert(
      await exists(layout.databasePath),
      `Installed Graphite did not create '${layout.databasePath}' within ${startupTimeoutMs} ms.`,
    );
    await sleep(stabilityWindowMs);
    assert(
      child.exitCode === null && child.signalCode === null,
      `Installed Graphite exited during the startup stability window.` +
        `\nstdout:\n${standardOutput}\nstderr:\n${standardError}`,
    );
  } finally {
    await terminateProcessTree(child);
  }
}

async function validateRuntime(executable, layout, lifecycle) {
  if (lifecycle === "first") {
    await assertFirstRunRoots(layout);
  } else {
    assert(
      (await readFile(layout.preservationPath, "utf8")) === preservationContent,
      "Installed application state was not preserved across package lifecycle operations.",
    );
  }

  await launchUntilReady(executable, layout);
  await assertSQLiteDatabase(layout.databasePath);
  await assertDevelopmentRoots(layout);
  await assertNoImplicitModels(layout.productionRoots);

  if (lifecycle === "first") {
    await writeFile(layout.preservationPath, preservationContent, "utf8");
  } else {
    assert(
      (await readFile(layout.preservationPath, "utf8")) === preservationContent,
      "Installed startup changed the preservation sentinel.",
    );
  }
}

async function findSingleTreeEntryPortable(root, filename) {
  const matches = [];
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = resolve(directory, entry.name);
      if (entry.isDirectory()) await visit(path);
      else if (entry.isFile() && entry.name.toLowerCase() === filename) {
        matches.push(path);
      }
    }
  }
  await visit(root);
  assert(
    matches.length === 1,
    `Expected one '${filename}' below '${root}'; found ${matches.length}.`,
  );
  return matches[0];
}

async function linuxLifecycle(artifacts, layout) {
  let xvfb;
  let debInstalled = false;
  try {
    const existingPackage = spawnSync(
      "dpkg-query",
      ["--show", "--showformat=${Status}", "graphite"],
      { encoding: "utf8" },
    );
    assert(
      existingPackage.status !== 0,
      "Refusing installed smoke because a system Graphite package is already installed.",
    );
    const display = `:${100 + (process.pid % 500)}`;
    xvfb = spawn(
      "Xvfb",
      [display, "-screen", "0", "1440x900x24", "-nolisten", "tcp"],
      { detached: true, stdio: "ignore" },
    );
    layout.childEnvironment.DISPLAY = display;
    layout.childEnvironment.WEBKIT_DISABLE_COMPOSITING_MODE = "1";
    await sleep(1_000);
    assert(
      xvfb.exitCode === null && xvfb.signalCode === null,
      "Xvfb exited before the installed application test started.",
    );

    runCommand("sudo", ["apt-get", "install", "--yes", artifacts.deb]);
    debInstalled = true;
    await validateRuntime("/usr/bin/graphite", layout, "first");
    runCommand("sudo", ["apt-get", "remove", "--yes", "graphite"]);
    debInstalled = false;

    await chmod(artifacts.appimage, 0o755);
    await validateRuntime(artifacts.appimage, layout, "preserve");

    runCommand("sudo", ["apt-get", "install", "--yes", artifacts.deb]);
    debInstalled = true;
    await validateRuntime("/usr/bin/graphite", layout, "preserve");
  } finally {
    if (debInstalled) {
      spawnSync("sudo", ["apt-get", "remove", "--yes", "graphite"], {
        encoding: "utf8",
      });
    }
    if (xvfb) await terminateProcessTree(xvfb);
  }
}

async function installWindowsPackage(installer, installRoot) {
  await mkdir(dirname(installRoot), { recursive: true });
  runCommand(installer, ["/S", `/D=${installRoot}`]);
  return {
    executable: await findSingleTreeEntryPortable(installRoot, "graphite.exe"),
    uninstaller: await findSingleTreeEntryPortable(
      installRoot,
      "uninstall.exe",
    ),
  };
}

async function windowsLifecycle(artifacts, layout) {
  const installRoot = resolve(layout.scratchRoot, "Graphite");
  assertChildPath(layout.scratchRoot, installRoot, "Windows install root");
  let installation;
  try {
    installation = await installWindowsPackage(
      artifacts.installer,
      installRoot,
    );
    await validateRuntime(installation.executable, layout, "first");
    runCommand(installation.uninstaller, ["/S"]);
    assert(
      await waitForAbsent(installation.executable),
      "The Windows uninstaller left the application executable installed.",
    );

    installation = await installWindowsPackage(
      artifacts.installer,
      installRoot,
    );
    await validateRuntime(installation.executable, layout, "preserve");
  } finally {
    if (installation && (await exists(installation.uninstaller))) {
      spawnSync(installation.uninstaller, ["/S"], { encoding: "utf8" });
    }
  }
}

async function copyMacosApplication(dmg, layout) {
  const mountRoot = resolve(layout.scratchRoot, "dmg");
  const installRoot = resolve(layout.scratchRoot, "Applications");
  const installedApp = resolve(installRoot, "Graphite.app");
  assertChildPath(layout.scratchRoot, mountRoot, "DMG mount root");
  assertChildPath(layout.scratchRoot, installedApp, "macOS install root");
  await rm(mountRoot, { force: true, recursive: true });
  await mkdir(mountRoot, { recursive: true });
  await mkdir(installRoot, { recursive: true });
  runCommand("hdiutil", [
    "attach",
    "-readonly",
    "-nobrowse",
    "-mountpoint",
    mountRoot,
    dmg,
  ]);
  try {
    const applications = (await readdir(mountRoot, { withFileTypes: true }))
      .filter((entry) => entry.isDirectory() && entry.name.endsWith(".app"))
      .map((entry) => resolve(mountRoot, entry.name));
    assert(
      applications.length === 1,
      `DMG must contain exactly one application; found ${applications.length}.`,
    );
    await rm(installedApp, { force: true, recursive: true });
    runCommand("ditto", [applications[0], installedApp]);
  } finally {
    runCommand("hdiutil", ["detach", mountRoot]);
  }
  return installedApp;
}

async function macosLifecycle(artifacts, layout) {
  let installedApp = await copyMacosApplication(artifacts.dmg, layout);
  const executable = resolve(installedApp, "Contents/MacOS/graphite");
  await validateRuntime(executable, layout, "first");
  await rm(installedApp, { force: true, recursive: true });
  assert(!(await exists(installedApp)), "macOS application removal failed.");

  installedApp = await copyMacosApplication(artifacts.dmg, layout);
  await validateRuntime(
    resolve(installedApp, "Contents/MacOS/graphite"),
    layout,
    "preserve",
  );
  await rm(installedApp, { force: true, recursive: true });
}

async function sha256(path) {
  const digest = createHash("sha256");
  for await (const chunk of createReadStream(path)) digest.update(chunk);
  return digest.digest("hex");
}

async function main() {
  const target = process.argv[2];
  assert(
    process.argv.length === 3 && targetDefinitions.has(target),
    "Usage: npm run package:installed-smoke -- <linux-x64|windows-x64|macos-arm64>",
  );
  const definition = targetDefinitions.get(target);
  assert(
    process.env.CI === "true" && process.env.GITHUB_ACTIONS === "true",
    "Installed package smoke is restricted to owner-triggered GitHub Actions on ephemeral hosted runners.",
  );
  assert(
    process.platform === definition.platform,
    `Target '${target}' requires '${definition.platform}', not '${process.platform}'.`,
  );

  const layout = resolveRuntimeLayout(target, process.env);
  const artifacts = await resolveArtifacts(target);
  const packageDigests = {};
  for (const [name, path] of Object.entries(artifacts)) {
    packageDigests[name] = await sha256(path);
  }

  if (target === "linux-x64") await linuxLifecycle(artifacts, layout);
  else if (target === "windows-x64") {
    await windowsLifecycle(artifacts, layout);
  } else await macosLifecycle(artifacts, layout);

  await assertSQLiteDatabase(layout.databasePath);
  await assertDevelopmentRoots(layout);
  await assertNoImplicitModels(layout.productionRoots);
  process.stdout.write(
    `Installed ${target} lifecycle smoke passed: first launch, production ` +
      `storage, development isolation, package removal/reinstall, preserved ` +
      `state, and no implicit model download. Package digests: ` +
      `${JSON.stringify(packageDigests)}\n`,
  );
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) await main();
