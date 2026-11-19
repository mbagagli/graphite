import { readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const versionFiles = [
  "package.json",
  "package-lock.json",
  "src-tauri/Cargo.toml",
  "src-tauri/Cargo.lock",
];

export function validateVersion(version) {
  if (
    typeof version !== "string" ||
    !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)
  ) {
    throw new Error("Use a stable version in MAJOR.MINOR.PATCH form.");
  }
  if (version.split(".").some((part) => Number(part) > 65535)) {
    throw new Error("Each version component must be at most 65535.");
  }
  return version;
}

function serializeJson(source, data) {
  const json = `${JSON.stringify(data, null, 2)}\n`;
  return source.includes("\r\n") ? json.replaceAll("\n", "\r\n") : json;
}

export async function setVersion(root, requestedVersion, check = false) {
  const sources = await Promise.all(
    versionFiles.map((name) => readFile(resolve(root, name), "utf8")),
  );
  const metadata = JSON.parse(sources[0]);
  const version = validateVersion(requestedVersion ?? metadata.version);
  const lock = JSON.parse(sources[1]);
  if (!lock.packages?.[""]) throw new Error("Package lock root is missing.");
  const manifestPattern = /^(\[package\][\s\S]*?\nversion\s*=\s*)"[^"]+"/m;
  const lockPattern =
    /^(\[\[package\]\]\r?\nname = "graphite"\r?\nversion = )"[^"]+"/m;
  if (!manifestPattern.test(sources[2])) {
    throw new Error("Rust application version entry is missing in Cargo.toml.");
  }
  if (!lockPattern.test(sources[3])) {
    throw new Error("Rust application version entry is missing in Cargo.lock.");
  }
  const config = JSON.parse(
    await readFile(resolve(root, "src-tauri/tauri.conf.json"), "utf8"),
  );
  if (config.version !== "../package.json") {
    throw new Error("Tauri must derive its version from package.json.");
  }
  metadata.version = version;
  lock.version = version;
  lock.packages[""].version = version;
  const updated = [
    serializeJson(sources[0], metadata),
    serializeJson(sources[1], lock),
    sources[2].replace(manifestPattern, `$1"${version}"`),
    sources[3].replace(lockPattern, `$1"${version}"`),
  ];
  const changed = versionFiles.filter(
    (_, index) => sources[index] !== updated[index],
  );
  if (check && changed.length) {
    throw new Error(
      `Version consumers disagree: ${changed.join(", ")}. Run npm run version:set -- ${version}.`,
    );
  }
  if (!check) {
    for (let index = 0; index < versionFiles.length; index += 1) {
      if (sources[index] !== updated[index]) {
        await writeFile(resolve(root, versionFiles[index]), updated[index]);
      }
    }
  }
  return { version, changed, consumers: versionFiles };
}

async function main() {
  const args = process.argv.slice(2);
  if (args.length !== 1 || args[0] === "--help") {
    process.stdout.write(
      "Graphite release version\n" +
        "Usage: npm run version:set -- MAJOR.MINOR.PATCH\n" +
        "       npm run version:check\n" +
        "Validates a stable version, synchronizes package and Rust metadata,\n" +
        "and reports changed files. Tauri, About and downloads derive it.\n" +
        "Components range from 0 to 65535. No tags or commits are created.\n",
    );
    if (args[0] !== "--help") process.exitCode = 1;
    return;
  }
  const check = args[0] === "--check";
  const result = await setVersion(
    repositoryRoot,
    check ? undefined : args[0],
    check,
  );
  process.stdout.write(
    `Version ${result.version}: ${check ? "consistent" : "synchronized"}.\n` +
      `Consumers: ${result.consumers.join(", ")}.\n` +
      `Changed: ${result.changed.join(", ") || "none"}.\n`,
  );
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  await main();
}
